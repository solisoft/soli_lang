//! View helpers (`app/helpers`) run by the view VM itself.
//!
//! A view runs with `tree_walk_functions`: a tree-walking function it calls
//! keeps the interpreter, which reads names in that function's closure. A
//! view helper's closure is the env every helper shares (builtins, the other
//! helpers, component classes, and the request bindings the request lifecycle
//! rebinds). The view VM's globals are not that env, so a helper compiles here
//! with every name it reads from its closure turned into an upvalue of a
//! synthetic scope, and those upvalues are filled from the closure on every
//! call. The helper then reads exactly what the tree-walker would — the same
//! builtins, the same sibling helpers, the current `req` — without the engine
//! switch, and its body runs as bytecode.
//!
//! A helper whose behavior on the VM could differ stays on the tree-walker:
//! see [`compile`]. A call whose arguments the tree-walker would treat
//! differently goes there too: see [`ViewHelper::bind`]. Neither is an
//! `EngineFallback`, which would demote the whole view. `SOLI_ENGINE_LOG=1`
//! names a view helper kept there and what kept it.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use crate::ast::expr::{Argument, Expr, ExprKind};
use crate::ast::stmt::{FunctionDecl, Parameter, Program, Stmt, StmtKind};
use crate::ast::types::{TypeAnnotation, TypeKind};
use crate::interpreter::builtins::template::HELPER_REQUEST_NAMES;
use crate::interpreter::value::{Function, Value};

use super::chunk::{CompiledModule, Constant, FunctionProto};
use super::compiler::Compiler;
use super::opcode::Op;
use super::upvalue::{Upvalue, VmClosure};

/// Names the tree-walker dispatches by name before it looks the callee up
/// (`Interpreter::evaluate_call`). Called through an upvalue, the VM would
/// call the value instead, so a helper calling one stays interpreted.
const INTERCEPTED_CALLS: &[&str] = &[
    "respond_to",
    "render_json",
    "grouped",
    "with_transaction",
    "after_commit",
    "with_tenant",
    "with_role",
    "assert_raises",
    "event",
    "Factory",
];

/// More free names than this and the helper stays interpreted: each is a
/// parameter of the synthetic scope.
const MAX_FREE_NAMES: usize = 200;

/// A view helper compiled for the view VM.
pub(crate) struct ViewHelper {
    /// The compiled helper. Its upvalues are `Closed` cells shared by every
    /// call; [`ViewHelper::bind`] fills them before each.
    closure: Rc<VmClosure>,
    /// The closure name each upvalue holds, in upvalue order.
    names: Box<[String]>,
    /// Parameters the body reads bare (not as a call's callee): an argument
    /// the tree-walker would auto-invoke there sends the call to it.
    bare_params: Box<[usize]>,
}

impl ViewHelper {
    /// The compiled helper's prototype: what a named call binds its labels
    /// against (`bind_named_arguments`) before [`ViewHelper::bind`].
    pub(crate) fn proto(&self) -> &FunctionProto {
        &self.closure.proto
    }

    /// The closure to call with `slots`, its upvalues holding what `func`'s
    /// closure holds now. `slots` are the argument values in parameter order:
    /// a positional call's arguments, or a named call's slots once its labels
    /// are bound (a parameter left to its default is `nil` there). `None`:
    /// this call runs on the tree-walker — a name the closure does not hold
    /// yet (`req` before any request), or an argument the body reads bare
    /// that the tree-walker would call.
    pub(crate) fn bind(&self, func: &Function, slots: &[Value]) -> Option<Rc<VmClosure>> {
        let auto_invoked = self.bare_params.iter().any(|&i| {
            slots
                .get(i)
                .is_some_and(crate::interpreter::executor::can_auto_invoke_with_no_args)
        });
        if auto_invoked {
            return None;
        }
        if !self.names.is_empty() {
            let closure = func.closure.try_borrow().ok()?;
            for (name, cell) in self.names.iter().zip(&self.closure.upvalues) {
                let value = closure.get(name)?;
                *cell.try_borrow_mut().ok()? = Upvalue::Closed(value);
            }
        }
        Some(Rc::clone(&self.closure))
    }
}

// Whether a view helper runs on the view VM, and how. Keyed by the body
// allocation. Only consulted for a function whose closure is the shared
// helper env, and dropped whenever helpers are reloaded — an address the
// allocator reused for some other function is never looked up.
thread_local! {
    #[allow(clippy::missing_const_for_thread_local)]
    static VIEW_HELPERS: RefCell<HashMap<usize, Option<Rc<ViewHelper>>>> =
        RefCell::new(HashMap::new());
}

/// Drop every cached decision. Called when view helpers are cleared or
/// loaded again.
pub(crate) fn clear_view_helper_vm_cache() {
    VIEW_HELPERS.with(|cache| cache.borrow_mut().clear());
}

/// `func` compiled for the view VM, or `None` when it runs on the
/// tree-walker: a method, a function from anywhere but `app/helpers`, or a
/// helper [`compile`] refuses.
pub(crate) fn view_helper(func: &Function) -> Option<Rc<ViewHelper>> {
    if func.is_method
        || !crate::interpreter::builtins::template::is_view_helper_closure(&func.closure)
    {
        return None;
    }
    // The data address of the shared statements identifies the body.
    let key = func.body.as_ptr() as usize;
    if let Some(decided) = VIEW_HELPERS.with(|cache| cache.borrow().get(&key).cloned()) {
        return decided;
    }
    let decided = match compile(func) {
        Ok(helper) => Some(Rc::new(helper)),
        Err(why) => {
            crate::template::vm_template::engine_log(|| {
                format!("view helper '{}' runs on the interpreter: {why}", func.name)
            });
            None
        }
    };
    VIEW_HELPERS.with(|cache| cache.borrow_mut().insert(key, decided.clone()));
    decided
}

/// Compile `func` for the view VM, or say why it stays on the tree-walker.
///
/// It stays there when the VM could see something else than the tree-walker:
/// the compiler refuses the body; it assigns a name its closure holds (the
/// tree-walker writes the shared closure); it names something the closure
/// does not hold, other than a request binding (the tree-walker resolves that
/// its own way); it calls a name the tree-walker intercepts; it reads a free
/// name bare whose value the tree-walker would call (`site_name` for a
/// zero-argument helper); or it declares a function or keeps a lambda
/// outside a call's arguments, either of which a bare read would call on the
/// tree-walker and not on the VM.
fn compile(func: &Function) -> Result<ViewHelper, String> {
    let shape = BodyShape::of(func);
    if shape.declares {
        return Err("it declares a function or a class of its own".to_string());
    }
    if shape.loose_lambda {
        return Err("it keeps a function outside a call's arguments".to_string());
    }
    let bound = func.closure.borrow().get_all_bindings();
    let source_path = func
        .source_path
        .as_ref()
        .map(|p| Arc::new(std::path::PathBuf::from(p)));

    // Every name the closure holds or may hold is a known global, so a read
    // of one is recorded and an assignment to one compiles to a global write.
    let seen = Rc::new(RefCell::new(HashSet::new()));
    let known = bound
        .keys()
        .cloned()
        .chain(HELPER_REQUEST_NAMES.iter().map(|name| name.to_string()));
    let module = Compiler::compile_with_globals_recording(
        &Program::new(vec![function_stmt(helper_decl(func))]),
        known,
        source_path.clone(),
        Rc::clone(&seen),
    )
    .map_err(|e| format!("the compiler refused it: {e}"))?;
    let proto = first_function(&module).ok_or("the compiler gave no function")?;
    let mut written = HashSet::new();
    global_writes(&proto, &mut written);
    if !written.is_empty() {
        return Err(format!("it assigns {}", quoted_names(&written)));
    }
    let free = seen.take();

    let intercepted = select(&free, |name| INTERCEPTED_CALLS.contains(&name));
    if !intercepted.is_empty() {
        return Err(format!(
            "it calls {}, which the interpreter runs itself",
            quoted_names(&intercepted)
        ));
    }
    let unbound = select(&free, |name| {
        !bound.contains_key(name) && !HELPER_REQUEST_NAMES.contains(&name)
    });
    if !unbound.is_empty() {
        return Err(format!(
            "it names {}, which its closure does not hold",
            quoted_names(&unbound)
        ));
    }
    let auto_invoked = select(&free, |name| {
        shape.bare_reads.contains(name)
            && (crate::interpreter::builtins::is_bare_call_builtin(name)
                || bound
                    .get(name)
                    .is_some_and(crate::interpreter::executor::can_auto_invoke_with_no_args))
    });
    if !auto_invoked.is_empty() {
        return Err(format!(
            "it reads {} without parentheses, which the interpreter calls",
            quoted_names(&auto_invoked)
        ));
    }

    let mut names: Vec<String> = free.into_iter().collect();
    if names.len() > MAX_FREE_NAMES {
        return Err(format!("it names {} things", names.len()));
    }
    names.sort_unstable();
    let inner = compile_in_scope(func, &names, source_path)?;
    // Each upvalue is a parameter of the scope: slot 0 is the scope's own,
    // so parameter `i` is slot `i + 1`.
    let upvalue_names = inner
        .upvalue_descriptors
        .iter()
        .map(|descriptor| {
            let param = usize::from(descriptor.index).checked_sub(1);
            match param.and_then(|i| names.get(i)) {
                Some(name) if descriptor.is_local => Ok(name.clone()),
                _ => Err("an upvalue that is not a closure name".to_string()),
            }
        })
        .collect::<Result<Box<[String]>, String>>()?;
    let cells = upvalue_names
        .iter()
        .map(|_| Rc::new(RefCell::new(Upvalue::Closed(Value::Null))))
        .collect();
    let bare_params = func
        .params
        .iter()
        .enumerate()
        .filter(|(_, param)| shape.bare_reads.contains(&param.name))
        .map(|(i, _)| i)
        .collect();
    Ok(ViewHelper {
        closure: Rc::new(VmClosure::new(inner, cells)),
        names: upvalue_names,
        bare_params,
    })
}

/// `func` compiled inside `def __view_helper_scope(<names>)`: every name it
/// reads from its closure is a local of that scope, so the helper reads it
/// as an upvalue.
fn compile_in_scope(
    func: &Function,
    names: &[String],
    source_path: Option<Arc<std::path::PathBuf>>,
) -> Result<Arc<FunctionProto>, String> {
    let span = func.span.unwrap_or_default();
    let scope = FunctionDecl {
        type_params: Vec::new(),
        name: "__view_helper_scope".to_string(),
        params: names
            .iter()
            .map(|name| Parameter {
                name: name.clone(),
                type_annotation: TypeAnnotation::new(TypeKind::Named("Any".to_string()), span),
                default_value: None,
                span,
                is_block_param: false,
            })
            .collect(),
        return_type: None,
        body: vec![function_stmt(helper_decl(func))].into(),
        span,
    };
    // Nothing is a global now: what the helper does not hold as a local or
    // a parameter is a scope local.
    let seen = Rc::new(RefCell::new(HashSet::new()));
    let module = Compiler::compile_with_globals_recording(
        &Program::new(vec![function_stmt(scope)]),
        std::iter::empty(),
        source_path,
        Rc::clone(&seen),
    )
    .map_err(|e| format!("the compiler refused it: {e}"))?;
    if !seen.borrow().is_empty() {
        return Err(format!("it names {}", quoted_names(&seen.borrow())));
    }
    let scope = first_function(&module).ok_or("the compiler gave no function")?;
    scope
        .chunk
        .constants
        .iter()
        .find_map(|constant| match constant {
            Constant::Function(proto) if proto.name == func.name => Some(Arc::clone(proto)),
            _ => None,
        })
        .ok_or_else(|| "the compiler gave no function".to_string())
}

/// `func` as a declaration: its parameters, declared return type and body.
fn helper_decl(func: &Function) -> FunctionDecl {
    FunctionDecl {
        type_params: Vec::new(),
        name: func.name.clone(),
        params: func.params.to_vec(),
        return_type: func.return_type.clone(),
        body: (*func.body).clone(),
        span: func.span.unwrap_or_default(),
    }
}

fn function_stmt(decl: FunctionDecl) -> Stmt {
    let span = decl.span;
    Stmt {
        kind: StmtKind::function(decl),
        span,
        source_path: None,
    }
}

fn first_function(module: &CompiledModule) -> Option<Arc<FunctionProto>> {
    module
        .main
        .chunk
        .constants
        .iter()
        .find_map(|constant| match constant {
            Constant::Function(proto) => Some(Arc::clone(proto)),
            _ => None,
        })
}

/// The globals `proto` (or a function compiled inside it) rebinds. A helper
/// compiled with its closure's names as known globals rebinds one exactly
/// when it assigns a closure name; a name it introduces is a local.
fn global_writes(proto: &FunctionProto, written: &mut HashSet<String>) {
    for op in &proto.chunk.code {
        let idx = match *op {
            Op::SetGlobal(idx)
            | Op::DefineGlobal(idx)
            | Op::AppendGlobal(idx)
            | Op::AddAssignGlobal(idx)
            | Op::AppendGlobalConst(idx, _) => idx,
            _ => continue,
        };
        let name = match proto.chunk.constants.get(usize::from(idx)) {
            Some(Constant::String(name)) => {
                let name: &str = name.as_ref();
                name.to_string()
            }
            _ => "?".to_string(),
        };
        written.insert(name);
    }
    for constant in &proto.chunk.constants {
        if let Constant::Function(inner) = constant {
            global_writes(inner, written);
        }
    }
}

fn select(names: &HashSet<String>, keep: impl Fn(&str) -> bool) -> HashSet<String> {
    names
        .iter()
        .filter(|name| keep(name.as_str()))
        .cloned()
        .collect()
}

/// What a helper's body does that the two engines treat apart.
#[derive(Default)]
struct BodyShape {
    /// Names read as a value, not as the callee of a call: the tree-walker
    /// calls a zero-argument function read this way.
    bare_reads: HashSet<String>,
    /// A `def` or a `class` inside the body.
    declares: bool,
    /// A lambda that is not a call's argument: kept in a local, a hash, or
    /// returned, it is a function a bare read would call on the tree-walker.
    loose_lambda: bool,
}

impl BodyShape {
    fn of(func: &Function) -> Self {
        let mut shape = BodyShape::default();
        // Parents are visited before their children, so a call has marked
        // its callee and its arguments by the time they are reached.
        let mut callees: HashSet<*const Expr> = HashSet::new();
        let mut arguments: HashSet<*const Expr> = HashSet::new();
        let mut declares = false;
        let mut on_stmt = |stmt: &Stmt| {
            if matches!(stmt.kind, StmtKind::Function(_) | StmtKind::Class(_)) {
                declares = true;
            }
        };
        let mut on_expr = |expr: &Expr| match &expr.kind {
            ExprKind::Call {
                callee,
                arguments: args,
            } => {
                if matches!(callee.kind, ExprKind::Variable(_)) {
                    callees.insert(&**callee as *const Expr);
                }
                for arg in args {
                    let value = match arg {
                        Argument::Positional(value) | Argument::Block(value) => value,
                        Argument::Named(named) => &named.value,
                    };
                    arguments.insert(value as *const Expr);
                }
            }
            ExprKind::Variable(name) if !callees.contains(&(expr as *const Expr)) => {
                shape.bare_reads.insert(name.clone());
            }
            ExprKind::Lambda { .. } if !arguments.contains(&(expr as *const Expr)) => {
                shape.loose_lambda = true;
            }
            _ => {}
        };
        crate::ast::walk::walk_stmts(&func.body, &mut on_stmt, &mut on_expr);
        for default in func.params.iter().filter_map(|p| p.default_value.as_ref()) {
            crate::ast::walk::walk_expr(default, &mut on_stmt, &mut on_expr);
        }
        shape.declares = declares;
        shape
    }
}

/// `` `a`, `b`, `c` ``, sorted, the first eight of them: the names that keep
/// a view helper on the tree-walker, for `SOLI_ENGINE_LOG`.
fn quoted_names(names: &HashSet<String>) -> String {
    const SHOWN: usize = 8;
    let mut sorted: Vec<&str> = names.iter().map(String::as_str).collect();
    sorted.sort_unstable();
    let shown = sorted
        .iter()
        .take(SHOWN)
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ");
    match sorted.len().saturating_sub(SHOWN) {
        0 => shown,
        more => format!("{shown} and {more} more"),
    }
}

/// Whether `func` was compiled for the view VM: `None` before its first
/// call from one.
#[cfg(test)]
fn decided(func: &Function) -> Option<bool> {
    let key = func.body.as_ptr() as usize;
    VIEW_HELPERS.with(|cache| cache.borrow().get(&key).map(Option::is_some))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::RuntimeError;
    use crate::interpreter::environment::Environment;
    use crate::lexer::Scanner;
    use crate::parser::Parser;
    use crate::vm::Vm;

    type Env = Rc<RefCell<Environment>>;

    /// Puts `env` in the view-helper slot for the test and puts the previous
    /// one back, including when the test panics.
    struct HelperEnvGuard {
        prev: Option<Env>,
    }

    impl HelperEnvGuard {
        fn set(env: Option<Env>) -> Self {
            let prev = crate::interpreter::builtins::template::set_view_helper_env_for_test(env);
            clear_view_helper_vm_cache();
            Self { prev }
        }
    }

    impl Drop for HelperEnvGuard {
        fn drop(&mut self) {
            crate::interpreter::builtins::template::set_view_helper_env_for_test(self.prev.take());
            clear_view_helper_vm_cache();
        }
    }

    fn empty_env() -> Env {
        Rc::new(RefCell::new(Environment::new()))
    }

    /// The env `load_view_helpers` builds: every builtin.
    fn builtins_env() -> Env {
        let env = empty_env();
        crate::interpreter::builtins::register_builtins(&mut env.borrow_mut(), false);
        env
    }

    /// Each `def` of `source` as a helper closing over `env`, defined in it
    /// as `load_view_helpers` does, so the helpers can call each other.
    fn define_helpers(source: &str, env: &Env) -> HashMap<String, Rc<Function>> {
        let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        let mut helpers = HashMap::new();
        for stmt in &program.statements {
            if let StmtKind::Function(decl) = &stmt.kind {
                let func = Rc::new(Function::from_decl(decl, Rc::clone(env), None));
                env.borrow_mut()
                    .define(decl.name.clone(), Value::Function(Rc::clone(&func)));
                helpers.insert(decl.name.clone(), func);
            }
        }
        helpers
    }

    fn helper(source: &str, env: &Env) -> Rc<Function> {
        define_helpers(source, env)
            .into_values()
            .next()
            .expect("a function declaration")
    }

    /// Run `source` (it binds `x`) on a VM set up as a view's: tree-walker
    /// functions, return types checked, and `globals` defined.
    fn on_view_vm(source: &str, globals: &[(&str, Value)]) -> Result<Value, RuntimeError> {
        let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        let module = Compiler::compile(&program).expect("compile error");
        let mut vm = Vm::new();
        vm.tree_walk_functions = true;
        vm.check_return_types = true;
        for (name, value) in globals {
            vm.globals.insert(name.to_string(), value.clone());
        }
        vm.execute(&module.main)?;
        Ok(vm.globals.get("x").cloned().unwrap_or(Value::Null))
    }

    /// `func` called on the view VM with `args` (the source's literals)
    /// and the same call on the tree-walker: the two results must agree.
    fn on_both(func: &Rc<Function>, args: &[Value]) -> Value {
        let mut globals: Vec<(String, Value)> =
            vec![(func.name.clone(), Value::Function(Rc::clone(func)))];
        let mut names = Vec::new();
        for (i, arg) in args.iter().enumerate() {
            names.push(format!("a{i}"));
            globals.push((format!("a{i}"), arg.clone()));
        }
        let source = format!("let x = {}({})", func.name, names.join(", "));
        let globals: Vec<(&str, Value)> = globals
            .iter()
            .map(|(name, value)| (name.as_str(), value.clone()))
            .collect();
        let on_vm = on_view_vm(&source, &globals);
        let on_tree_walker = crate::interpreter::Interpreter::for_bound_body()
            .call_function_with_this(func, None, args.to_vec());
        match (on_vm, on_tree_walker) {
            (Ok(vm), Ok(tree)) => {
                assert_eq!(vm, tree, "{source}: the engines disagree");
                vm
            }
            (vm, tree) => panic!("{source}: vm {vm:?}, tree-walker {tree:?}"),
        }
    }

    #[test]
    fn a_helper_with_no_free_name_runs_on_the_view_vm() {
        let env = empty_env();
        let _guard = HelperEnvGuard::set(Some(Rc::clone(&env)));
        let double = helper("def double(n)\n  n * 2\nend\n", &env);

        assert_eq!(on_both(&double, &[Value::Int(21)]), Value::Int(42));
        assert_eq!(decided(&double), Some(true));
        let first = view_helper(&double).expect("compiled");
        // The second call is the cache hit: the same closure.
        assert_eq!(on_both(&double, &[Value::Int(22)]), Value::Int(44));
        let second = view_helper(&double).expect("compiled");
        assert!(Rc::ptr_eq(&first.closure, &second.closure));
        // Nothing for the other callers of `jit_cache`, which expect a proto
        // with no upvalues.
        assert!(double.jit_cache.borrow().is_none());
    }

    /// The scaffold's helpers call builtins and each other. They used to
    /// stay on the tree-walker for it; they compile now, and say the same.
    #[test]
    fn the_scaffold_helpers_run_on_the_view_vm() {
        let env = builtins_env();
        let _guard = HelperEnvGuard::set(Some(Rc::clone(&env)));
        let helpers = define_helpers(
            include_str!("../scaffold/templates/application_helper.sl"),
            &env,
        );
        let s = |text: &str| Value::str(text);
        let cases: &[(&str, Vec<Value>)] = &[
            ("link_to", vec![s("Posts <all>"), s("/posts?a=1&b=2")]),
            ("link_to", vec![s("x"), s("javascript:alert(1)")]),
            ("link_to", vec![s("x"), s("https://example.com/a\"b")]),
            ("link_to_class", vec![s("x"), s("/p"), s("btn \"x\"")]),
            ("pluralize", vec![Value::Int(1), s("post"), s("posts")]),
            ("pluralize", vec![Value::Int(3), s("post"), s("posts")]),
            ("pluralize_simple", vec![Value::Int(2), s("row")]),
            ("truncate_text", vec![s("Hi"), Value::Int(8), s("…")]),
            // Past the early return: `truncate_text` and `capitalize` call
            // the string methods since the serve lane's scaffold fix (they
            // called `substring(...)`, a view-only function, and raised).
            (
                "truncate_text",
                vec![s("Hello, world"), Value::Int(8), s("…")],
            ),
            ("capitalize", vec![s("")]),
            ("capitalize", vec![s("soli")]),
            ("absolute_url", vec![s("/posts/1")]),
        ];
        for (name, args) in cases {
            on_both(&helpers[*name], args);
        }
        assert_eq!(
            on_both(&helpers["link_to"], &[s("Go"), s("javascript:x")]),
            s("<a href=\"#\">Go</a>")
        );
        // `_is_safe_link_url` with them: its locals (`slash_at`, `query_at`,
        // `fragment_at`) no longer take a name the shared env holds. It
        // assigned `h = lower.index_of("#")`, which overwrote the `h` builtin
        // for every later helper on the worker; this test used to pin that
        // clobber.
        for name in [
            "link_to",
            "link_to_class",
            "_safe_link_url",
            "_is_safe_link_url",
            "pluralize",
            "pluralize_simple",
            "truncate_text",
            "capitalize",
            "absolute_url",
        ] {
            assert_eq!(decided(&helpers[name]), Some(true), "{name}");
        }
        assert!(
            matches!(env.borrow().get("h"), Some(Value::NativeFunction(_))),
            "the helpers' `h` is still the builtin"
        );
    }

    /// A helper's bare assignment to a name the shared env holds — a
    /// builtin like `h`, a sibling helper — defines a local, whichever
    /// engine runs it: the env is frozen once its helpers are loaded
    /// (`load_view_helpers`). A builtin's name compiles to a local on the
    /// view VM (#630); a helper that assigns a sibling's name stays on the
    /// tree-walker. Neither may overwrite the env every later helper call on
    /// the worker reads.
    #[test]
    fn a_helper_assigning_a_shared_name_defines_a_local() {
        let env = builtins_env();
        let _guard = HelperEnvGuard::set(Some(Rc::clone(&env)));
        let helpers = define_helpers(
            "def shout(text)\n  h = text.upcase\n  h + \"!\"\nend\n\
             def other()\n  \"o\"\nend\n\
             def shadow()\n  other = \"local\"\n  other\nend\n\
             def escape(text)\n  h(text)\nend\n",
            &env,
        );
        env.borrow_mut().freeze();
        for _ in 0..2 {
            assert_eq!(
                on_both(&helpers["shout"], &[Value::str("hi")]),
                Value::str("HI!")
            );
            assert_eq!(on_both(&helpers["shadow"], &[]), Value::str("local"));
            assert_eq!(on_both(&helpers["other"], &[]), Value::str("o"));
            assert_eq!(
                on_both(&helpers["escape"], &[Value::str("<b>")]),
                Value::str("&lt;b&gt;")
            );
        }
        assert_eq!(decided(&helpers["shout"]), Some(true));
        assert_eq!(decided(&helpers["shadow"]), Some(false));
        assert!(matches!(
            env.borrow().get("h"),
            Some(Value::NativeFunction(_))
        ));
        assert!(matches!(
            env.borrow().get("other"),
            Some(Value::Function(_))
        ));
    }

    /// `req` is rebound on the closure for every request: a compiled helper
    /// reads the current one.
    #[test]
    fn a_helper_reading_req_sees_each_request() {
        let env = builtins_env();
        let _guard = HelperEnvGuard::set(Some(Rc::clone(&env)));
        let who = helper(
            "def who()\n  \"#{req.user} on #{len(params)}\"\nend\n",
            &env,
        );
        let request = |user: &str, params: Vec<Value>| {
            let mut req = crate::interpreter::value::HashPairs::default();
            req.insert(
                crate::interpreter::value::HashKey::String("user".into()),
                Value::str(user),
            );
            let req = Value::Hash(Rc::new(RefCell::new(req)));
            let params = Value::Array(Rc::new(RefCell::new(params)));
            crate::interpreter::builtins::template::set_helper_request_context(
                &req,
                &params,
                &Value::Null,
                &Value::Null,
                &Value::Null,
            );
        };
        request("Ada", vec![]);
        assert_eq!(on_both(&who, &[]), Value::str("Ada on 0"));
        assert_eq!(decided(&who), Some(true));
        request("Grace", vec![Value::Int(1)]);
        assert_eq!(on_both(&who, &[]), Value::str("Grace on 1"));
    }

    /// Before any request the closure has no `req`: the call runs on the
    /// tree-walker, and the helper stays compiled for later calls.
    #[test]
    fn a_request_name_not_bound_yet_sends_that_call_to_the_tree_walker() {
        let env = empty_env();
        let _guard = HelperEnvGuard::set(Some(Rc::clone(&env)));
        let who = helper("def who(fallback)\n  req ?? fallback\nend\n", &env);
        let tree = crate::interpreter::Interpreter::for_bound_body().call_function_with_this(
            &who,
            None,
            vec![Value::Int(1)],
        );
        let on_vm = on_view_vm(
            "let x = who(1)",
            &[("who", Value::Function(Rc::clone(&who)))],
        );
        // The tree-walker raises for the missing `req`, and so does the call
        // the VM hands it.
        let (Err(on_vm), Err(tree)) = (on_vm, tree) else {
            panic!("both engines raise for an unbound req");
        };
        assert!(tree.to_string().contains("req"), "{tree}");
        assert!(on_vm.to_string().contains("req"), "{on_vm}");
        assert!(!on_vm.is_engine_fallback());
        assert_eq!(decided(&who), Some(true));
        env.borrow_mut()
            .define("req".to_string(), Value::str("Ada"));
        assert_eq!(on_both(&who, &[Value::Int(1)]), Value::str("Ada"));
    }

    /// A helper that assigns a name its closure holds stays on the
    /// tree-walker, which defines a local for it: the helper env is frozen
    /// (`load_view_helpers`), so the shared `req` and `total` stay as they
    /// were. Before 2.29.4 the tree-walker wrote them into the shared env,
    /// and this test pinned that (`req` read "no" afterwards).
    #[test]
    fn a_helper_that_assigns_a_closure_name_stays_interpreted() {
        let env = empty_env();
        env.borrow_mut()
            .define("req".to_string(), Value::str("Ada"));
        env.borrow_mut().define("total".to_string(), Value::Int(0));
        let _guard = HelperEnvGuard::set(Some(Rc::clone(&env)));

        let clobber = helper("def clobber()\n  req = \"no\"\nend\n", &env);
        // A compound assignment, and one inside a block, count too.
        let count = helper(
            "def bump(xs)\n  xs.each { |x| total += x }\n  total\nend\n",
            &env,
        );
        env.borrow_mut().freeze();
        on_view_vm(
            "let x = clobber()",
            &[("clobber", Value::Function(Rc::clone(&clobber)))],
        )
        .expect("runs");
        assert_eq!(
            env.borrow().get("req"),
            Some(Value::str("Ada")),
            "assigning req defines a local, not the helper env's req"
        );
        assert_eq!(decided(&clobber), Some(false));

        on_view_vm(
            "let x = bump([1, 2])",
            &[("bump", Value::Function(Rc::clone(&count)))],
        )
        .expect("runs");
        assert_eq!(env.borrow().get("total"), Some(Value::Int(0)));
        assert_eq!(decided(&count), Some(false));
    }

    /// A-2 (2.29.4) on the view VM's helpers: a zero-argument helper read
    /// bare is called, as the tree-walker calls it — in a helper's `if` and
    /// in a compiled view's `<% if signed_in? %>` alike. `signed_in?` read as
    /// the function itself is always truthy: the signed-in branch rendered
    /// for every visitor. A lambda follows the tree-walker too: called when
    /// a helper reads its parameter bare (on the tree-walker), not called by
    /// a compiled view, which refuses to write it.
    #[test]
    fn a_bare_zero_argument_helper_is_called_as_on_the_tree_walker() {
        let env = builtins_env();
        let _guard = HelperEnvGuard::set(Some(Rc::clone(&env)));
        let helpers = define_helpers(
            "def signed_in?()\n  false\nend\n\
             def nav()\n  if signed_in?\n    \"member\"\n  else\n    \"guest\"\n  end\nend\n\
             def kept(callback)\n  holder = callback\n  holder\nend\n",
            &env,
        );
        env.borrow_mut().freeze();
        assert_eq!(on_both(&helpers["nav"], &[]), Value::str("guest"));
        assert_eq!(decided(&helpers["nav"]), Some(false));

        // A lambda handed to a helper that reads its parameter bare: the
        // tree-walker calls any zero-argument function read bare from a
        // variable, so that call goes there and both engines answer the
        // lambda's value; the helper itself stays compiled.
        let mut interpreter = crate::interpreter::Interpreter::new();
        let tokens = Scanner::new("callback = fn() { \"called\" }\n")
            .scan_tokens()
            .expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        interpreter.interpret(&program).expect("runs");
        let callback = interpreter
            .environment
            .borrow()
            .get("callback")
            .expect("the lambda");
        assert_eq!(
            on_both(&helpers["kept"], std::slice::from_ref(&callback)),
            Value::str("called")
        );
        assert_eq!(decided(&helpers["kept"]), Some(true));

        // A compiled view reading the helper bare calls it.
        let mut vm = Vm::new();
        vm.tree_walk_functions = true;
        for (name, value) in env.borrow().get_all_bindings() {
            vm.globals.insert(name, value);
        }
        let _lenient = crate::interpreter::executor::enter_template_lenient_vars();
        let nodes = crate::template::parser::parse_template(
            "<% if signed_in? %>member<% else %>guest<% end %>|<%= nav %>",
        )
        .expect("parses");
        let compiled = crate::template::vm_template::VmTemplate::compile(&nodes).expect("compiles");
        let data = Value::Hash(Rc::new(RefCell::new(
            crate::interpreter::value::HashPairs::default(),
        )));
        let rendered = compiled.render(&mut vm, &data).expect("renders");
        assert_eq!(rendered.html, "guest|guest");

        // A lambda the view reads bare is not a helper: the VM does not call
        // it, and writing a function refuses the render, which then goes to
        // the tree-walker.
        let nodes = crate::template::parser::parse_template("<%= callback %>").expect("parses");
        let compiled = crate::template::vm_template::VmTemplate::compile(&nodes).expect("compiles");
        let mut data = crate::interpreter::value::HashPairs::default();
        data.insert(
            crate::interpreter::value::HashKey::String("callback".into()),
            callback,
        );
        let data = Value::Hash(Rc::new(RefCell::new(data)));
        let refused = compiled.render(&mut vm, &data).err().expect("refused");
        assert!(refused.is_engine_fallback(), "{refused}");
    }

    /// A function declared in the body, or a lambda kept in a local, is a
    /// function a bare read calls on the tree-walker and not on the VM.
    #[test]
    fn a_helper_declaring_a_function_or_keeping_a_lambda_stays_interpreted() {
        let env = empty_env();
        env.borrow_mut()
            .define("req".to_string(), Value::str("Ada"));
        let _guard = HelperEnvGuard::set(Some(Rc::clone(&env)));

        let outer = helper(
            "def outer()\n  def inner()\n    req\n  end\n  inner\nend\n",
            &env,
        );
        assert_eq!(on_both(&outer, &[]), Value::str("Ada"));
        assert_eq!(decided(&outer), Some(false));

        let kept = helper("def kept()\n  f = fn() { 7 }\n  f\nend\n", &env);
        assert_eq!(on_both(&kept, &[]), Value::Int(7));
        assert_eq!(decided(&kept), Some(false));

        // A block handed to a call is fine.
        let block = helper("def doubled(xs)\n  xs.map { |x| x * 2 }\nend\n", &env);
        let xs = Value::Array(Rc::new(RefCell::new(vec![Value::Int(1), Value::Int(2)])));
        on_both(&block, &[xs]);
        assert_eq!(decided(&block), Some(true));
    }

    /// A zero-argument helper read without parentheses is called by the
    /// tree-walker; an upvalue read would hand the function back.
    #[test]
    fn a_bare_zero_argument_sibling_keeps_the_helper_interpreted() {
        let env = builtins_env();
        let _guard = HelperEnvGuard::set(Some(Rc::clone(&env)));
        let helpers = define_helpers(
            "def site_name()\n  \"Soli\"\nend\n\
             def title(page)\n  page + \" - \" + site_name\nend\n\
             def called(page)\n  page + \" - \" + site_name()\nend\n",
            &env,
        );
        assert_eq!(
            on_both(&helpers["title"], &[Value::str("Home")]),
            Value::str("Home - Soli")
        );
        assert_eq!(decided(&helpers["title"]), Some(false));
        assert_eq!(
            on_both(&helpers["called"], &[Value::str("Home")]),
            Value::str("Home - Soli")
        );
        assert_eq!(decided(&helpers["called"]), Some(true));
    }

    /// A parameter read bare and given a zero-argument function: the
    /// tree-walker calls it, so that call runs there. Other calls stay on
    /// the VM.
    #[test]
    fn a_bare_parameter_given_a_function_sends_that_call_to_the_tree_walker() {
        let env = empty_env();
        let _guard = HelperEnvGuard::set(Some(Rc::clone(&env)));
        let wrap = helper("def wrap(value)\n  value\nend\n", &env);
        let other = empty_env();
        let seven = helper("def seven()\n  7\nend\n", &other);
        assert_eq!(
            on_view_vm(
                "let x = wrap(seven)",
                &[
                    ("wrap", Value::Function(Rc::clone(&wrap))),
                    ("seven", Value::Function(Rc::clone(&seven))),
                ],
            )
            .expect("runs"),
            Value::Int(7)
        );
        assert_eq!(decided(&wrap), Some(true));
        assert_eq!(on_both(&wrap, &[Value::Int(3)]), Value::Int(3));
    }

    #[test]
    fn a_helper_naming_what_its_closure_lacks_or_an_intercepted_call_stays_interpreted() {
        let env = builtins_env();
        let _guard = HelperEnvGuard::set(Some(Rc::clone(&env)));
        let model = helper("def latest()\n  Post.first\nend\n", &env);
        let _ = on_view_vm(
            "let x = latest()",
            &[("latest", Value::Function(Rc::clone(&model)))],
        );
        assert_eq!(decided(&model), Some(false));

        let grouped = helper("def both()\n  grouped(fn() { 1 })\nend\n", &env);
        let _ = on_view_vm(
            "let x = both()",
            &[("both", Value::Function(Rc::clone(&grouped)))],
        );
        assert_eq!(decided(&grouped), Some(false));
    }

    /// The tree-walker checks a declared return type; the view VM does too.
    #[test]
    fn a_declared_return_type_is_checked_on_the_view_vm() {
        let env = builtins_env();
        let _guard = HelperEnvGuard::set(Some(Rc::clone(&env)));
        let label = helper(
            "def label(n) -> String\n  return nil if n == 0\n\n  str(n)\nend\n",
            &env,
        );
        assert_eq!(on_both(&label, &[Value::Int(2)]), Value::str("2"));
        assert_eq!(decided(&label), Some(true));

        let tree = crate::interpreter::Interpreter::for_bound_body()
            .call_function_with_this(&label, None, vec![Value::Int(0)])
            .expect_err("the tree-walker raises");
        let on_vm = on_view_vm(
            "let x = label(0)",
            &[("label", Value::Function(Rc::clone(&label)))],
        )
        .expect_err("the view VM raises");
        assert!(
            tree.to_string().contains("expected to return String"),
            "{tree}"
        );
        assert!(
            on_vm.to_string().contains("expected to return String"),
            "{on_vm}"
        );
        assert!(!on_vm.is_engine_fallback());
    }

    /// `inner(x, sep: …)` called with `labels` on the tree-walker, the way
    /// its `evaluate_call` binds them.
    fn named_on_tree_walker(
        func: &Rc<Function>,
        positional: Vec<Value>,
        named: &[(&str, Value)],
    ) -> Result<Value, RuntimeError> {
        crate::interpreter::Interpreter::for_bound_body().call_value_with_named(
            Value::Function(Rc::clone(func)),
            positional,
            named
                .iter()
                .map(|(name, value)| (name.to_string(), value.clone()))
                .collect(),
            None,
            crate::span::Span::default(),
        )
    }

    const SEPARATED: &str = "def inner(x, sep: String = \"-\")\n  x + sep + req\nend\n\
         def outer(x)\n  inner(x, sep: \"+\")\nend\n\
         def outer_default(x)\n  inner(x)\nend\n";

    /// A compiled helper calling a sibling with a label: the sibling binds
    /// the label on the view VM and reads its closure's `req`, as on the
    /// tree-walker. It used to be compiled against the view VM's globals,
    /// which hold no `req`.
    #[test]
    fn a_helper_calling_a_sibling_with_a_named_argument_agrees_with_the_tree_walker() {
        let env = builtins_env();
        env.borrow_mut().define("req".to_string(), Value::str("R"));
        let _guard = HelperEnvGuard::set(Some(Rc::clone(&env)));
        let helpers = define_helpers(SEPARATED, &env);
        assert_eq!(
            on_both(&helpers["outer"], &[Value::str("a")]),
            Value::str("a+R")
        );
        assert_eq!(
            on_both(&helpers["outer_default"], &[Value::str("a")]),
            Value::str("a-R")
        );
        for name in ["inner", "outer", "outer_default"] {
            assert_eq!(decided(&helpers[name]), Some(true), "{name}");
        }
    }

    /// The view itself calling a helper with labels: bound on the view VM,
    /// a default filled by the helper's prologue.
    #[test]
    fn a_view_calling_a_helper_with_a_named_argument_agrees_with_the_tree_walker() {
        let env = builtins_env();
        env.borrow_mut().define("req".to_string(), Value::str("R"));
        let _guard = HelperEnvGuard::set(Some(Rc::clone(&env)));
        let helpers = define_helpers(SEPARATED, &env);
        let inner = &helpers["inner"];
        let globals = [("inner", Value::Function(Rc::clone(inner)))];
        // The source, then the same call as positional and labelled values.
        type Case<'a> = (&'a str, Vec<Value>, Vec<(&'a str, Value)>);
        let cases: &[Case] = &[
            (
                "let x = inner(\"a\", sep: \"+\")",
                vec![Value::str("a")],
                vec![("sep", Value::str("+"))],
            ),
            (
                "let x = inner(sep: \"/\", x: \"b\")",
                vec![],
                vec![("sep", Value::str("/")), ("x", Value::str("b"))],
            ),
            (
                "let x = inner(x: \"c\")",
                vec![],
                vec![("x", Value::str("c"))],
            ),
        ];
        for (source, positional, named) in cases {
            let on_vm = on_view_vm(source, &globals).unwrap_or_else(|e| panic!("{source}: {e}"));
            let tree = named_on_tree_walker(inner, positional.clone(), named)
                .unwrap_or_else(|e| panic!("{source}: tree-walker {e}"));
            assert_eq!(on_vm, tree, "{source}: the engines disagree");
        }
        assert_eq!(decided(inner), Some(true));
        // An unknown label raises on both, and is no engine fallback.
        let on_vm = on_view_vm("let x = inner(\"a\", nope: 1)", &globals)
            .expect_err("an unknown label raises");
        let tree = named_on_tree_walker(inner, vec![Value::str("a")], &[("nope", Value::Int(1))])
            .expect_err("an unknown label raises");
        for err in [&on_vm, &tree] {
            assert!(
                matches!(err, RuntimeError::UndefinedVariable(name, _, _) if name == "nope"),
                "{err:?}"
            );
        }
        assert!(!on_vm.is_engine_fallback());
    }

    /// A named call the VM does not run itself — a helper `compile` refuses,
    /// a function from outside `app/helpers`, a labelled argument the body
    /// reads bare that the tree-walker would call — runs on the tree-walker
    /// with its labels, and is no engine fallback.
    #[test]
    fn a_named_call_the_view_vm_does_not_run_goes_to_the_tree_walker() {
        let env = builtins_env();
        env.borrow_mut().define("req".to_string(), Value::str("R"));
        let _guard = HelperEnvGuard::set(Some(Rc::clone(&env)));
        let refused = helper(
            "def tag(x, sep: String = \"-\")\n  def nested()\n    1\n  end\n  x + sep + req\nend\n",
            &env,
        );
        assert_eq!(
            on_view_vm(
                "let x = tag(\"a\", sep: \"+\")",
                &[("tag", Value::Function(Rc::clone(&refused)))],
            )
            .expect("runs on the tree-walker"),
            Value::str("a+R")
        );
        assert_eq!(decided(&refused), Some(false));

        let outside = empty_env();
        outside
            .borrow_mut()
            .define("req".to_string(), Value::str("O"));
        let foreign = helper(
            "def foreign(x, sep: String = \"-\")\n  x + sep + req\nend\n",
            &outside,
        );
        assert_eq!(
            on_view_vm(
                "let x = foreign(\"a\", sep: \"+\")",
                &[("foreign", Value::Function(Rc::clone(&foreign)))],
            )
            .expect("runs on the tree-walker"),
            Value::str("a+O")
        );
        assert_eq!(decided(&foreign), None);

        // `value` is read bare: given a zero-argument function by label, the
        // tree-walker calls it, so the call goes there.
        let wrap = helper("def wrap(tail, value = nil)\n  value\nend\n", &env);
        let seven = helper("def seven()\n  7\nend\n", &empty_env());
        let globals = [
            ("wrap", Value::Function(Rc::clone(&wrap))),
            ("seven", Value::Function(Rc::clone(&seven))),
        ];
        assert_eq!(
            on_view_vm("let x = wrap(1, value: seven)", &globals).expect("runs"),
            Value::Int(7)
        );
        assert_eq!(decided(&wrap), Some(true));
        assert_eq!(
            on_view_vm("let x = wrap(1, value: 3)", &globals).expect("runs"),
            Value::Int(3)
        );
    }

    #[test]
    fn a_function_outside_the_helper_env_stays_interpreted() {
        let closure = empty_env();
        closure
            .borrow_mut()
            .define("req".to_string(), Value::str("Ada"));
        // The helper slot points somewhere else, so `who` is not a view helper
        // even though its own closure has `req`.
        let _guard = HelperEnvGuard::set(Some(empty_env()));
        let who = helper("def who()\n  req\nend\n", &closure);
        assert_eq!(
            on_view_vm(
                "let x = who()",
                &[("who", Value::Function(Rc::clone(&who)))]
            )
            .expect("runs"),
            Value::str("Ada")
        );
        assert_eq!(decided(&who), None);
    }

    /// What `SOLI_ENGINE_LOG` says kept a view helper on the tree-walker:
    /// the names, sorted and quoted, the first eight of them.
    #[test]
    fn the_names_that_keep_a_helper_interpreted_read_sorted_and_capped() {
        fn names(list: &[&str]) -> HashSet<String> {
            list.iter().map(|name| name.to_string()).collect()
        }
        assert_eq!(
            quoted_names(&names(&["html_escape", "_safe_link_url"])),
            "`_safe_link_url`, `html_escape`"
        );
        let many: Vec<String> = (0..10).map(|i| format!("n{i}")).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        assert_eq!(
            quoted_names(&names(&many)),
            "`n0`, `n1`, `n2`, `n3`, `n4`, `n5`, `n6`, `n7` and 2 more"
        );
    }
}
