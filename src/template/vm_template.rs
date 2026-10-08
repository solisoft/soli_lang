//! Views compiled to VM bytecode (prototype).
//!
//! A template whose nodes are all literal text, `<%= %>` / `<%- %>` output,
//! `if` / `unless` / `for` blocks and `<% %>` code compiles to one VM
//! function. Text and output become `__tpl_write` / `__tpl_write_raw`
//! intrinsics, which the compiler turns into `Op::TemplateWrite`: the value is
//! written into `Vm::template_out`, with no string built and no native call.
//!
//! The template's free names (`rows`, `title`, `h`) become the function's
//! parameters. At render time each gets the data value of that name, else the
//! VM global (a helper, a builtin), else nil — the order the tree-walker's
//! scope chain resolves them in, with its lenient undefined read.
//!
//! Anything else (partials, `yield`, `content_for`, `form_with`, components,
//! `@ivar`) does not compile, and the template keeps the tree-walker.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use crate::ast::expr::{Argument, Expr, ExprKind};
use crate::ast::stmt::{FunctionDecl, Parameter, Program, Stmt, StmtKind};
use crate::ast::types::{TypeAnnotation, TypeKind};
use crate::error::RuntimeError;
use crate::interpreter::value::{hash_get_value, Value};
use crate::span::Span;
use crate::template::parser::TemplateNode;
use crate::vm::chunk::{Constant, FunctionProto};
use crate::vm::{Compiler, Vm, VmClosure};

/// A template compiled to a VM function.
pub struct VmTemplate {
    closure: Value,
    params: Vec<String>,
    /// Whether the function returns the variables the view assigned (see
    /// [`VmTemplate::render`]).
    exports: bool,
}

/// A rendered view: its text, and the variables it assigned where its layout
/// would see them on the tree-walker.
pub struct Rendered {
    pub html: String,
    pub assigned: Vec<(String, Value)>,
}

impl VmTemplate {
    /// Compile `nodes`, or say why they need the tree-walker.
    pub fn compile(nodes: &[TemplateNode]) -> Result<Self, String> {
        let mut assigned = std::collections::BTreeSet::new();
        let mut all_assigned = std::collections::BTreeSet::new();
        let mut body = lower_nodes(nodes, false, &mut assigned, &mut all_assigned)?;
        // A view and its layout share one interpreter on the tree-walker, so
        // `<% title = "Posts" %>` in the view is `title` in the layout. The
        // compiled function's locals die with it: it returns them instead.
        let exports = !assigned.is_empty();
        if exports {
            let pairs = assigned
                .into_iter()
                .map(|name| {
                    (
                        Expr::new(ExprKind::StringLiteral(name.clone()), Span::default()),
                        Expr::new(ExprKind::Variable(name), Span::default()),
                    )
                })
                .collect();
            body.push(stmt(StmtKind::Expression(Expr::new(
                ExprKind::Hash(pairs),
                Span::default(),
            ))));
        }
        // First pass: learn the free names, which the compiler resolves as
        // globals. The second compiles them as parameters.
        let seen = Rc::new(RefCell::new(HashSet::new()));
        compile_function(&body, &[], Some(seen.clone()))?;
        // A name the view assigns is a parameter too: `<% errors = @errors ?? {} %>`
        // reads the data's `errors` before replacing it, as the tree-walker
        // does. As a plain local it would read nil.
        let mut params: Vec<String> = seen.borrow().iter().cloned().collect();
        params.extend(all_assigned);
        params.sort();
        params.dedup();
        let proto = compile_function(&body, &params, None)?;
        Ok(Self {
            closure: Value::VmClosure(Rc::new(VmClosure::new(proto, Vec::new()))),
            params,
            exports,
        })
    }

    /// The free names the template reads, in parameter order.
    pub fn params(&self) -> &[String] {
        &self.params
    }

    /// Render with `data` on `vm`, a name the data lacks resolving in `vm`'s
    /// globals. An error leaves the VM reset and nothing written: the caller
    /// renders on the tree-walker instead.
    pub fn render(&self, vm: &mut Vm, data: &Value) -> Result<Rendered, RuntimeError> {
        let names = std::mem::take(&mut vm.globals);
        let args = self.bind(data, &names);
        vm.globals = names;
        self.run(vm, args)
    }

    /// The template's arguments: each free name's data value, else its value
    /// in `names` (the view's scope past its data), else nil.
    fn bind(&self, data: &Value, names: &ahash::AHashMap<String, Value>) -> Vec<Value> {
        self.params
            .iter()
            .map(|name| {
                data_value(data, name)
                    .or_else(|| names.get(name).cloned())
                    .unwrap_or(Value::Null)
            })
            .collect()
    }

    fn run(&self, vm: &mut Vm, args: Vec<Value>) -> Result<Rendered, RuntimeError> {
        vm.template_out.clear();
        vm.template_captures.clear();
        // The page leaves with the result (`mem::take` below), so each render
        // starts from an empty buffer: size it as the tree-walker's renderer
        // does instead of growing it a few times over.
        if vm.template_out.capacity() == 0 {
            vm.template_out.reserve(4096);
        }
        match vm.call_value_direct(self.closure.clone(), &args, Span::default()) {
            Ok(result) => {
                let assigned = match (self.exports, result) {
                    (true, Value::Hash(map)) => map
                        .borrow()
                        .iter()
                        .map(|(k, v)| (k.to_value().to_string(), v.clone()))
                        .collect(),
                    _ => Vec::new(),
                };
                Ok(Rendered {
                    html: std::mem::take(&mut vm.template_out),
                    assigned,
                })
            }
            Err(e) => {
                vm.reset();
                vm.template_out.clear();
                vm.template_captures.clear();
                Err(e)
            }
        }
    }
}

/// The value a view's bare `name` reads from its data: the key itself, or the
/// whole hash for `locals`, as `core_eval::create_template_interpreter` binds.
fn data_value(data: &Value, name: &str) -> Option<Value> {
    // `locals` is always a hash: the data, or an empty one when the data is
    // not a hash (a partial rendered with no locals).
    if name == "locals" {
        return Some(match data {
            Value::Hash(_) => data.clone(),
            _ => Value::Hash(Rc::new(RefCell::new(
                crate::interpreter::value::HashPairs::default(),
            ))),
        });
    }
    let Value::Hash(map) = data else {
        return None;
    };
    let value = hash_get_value(&map.borrow(), &Value::String(name.into())).cloned()?;
    // A controller `@ivar` set inside `grouped(fn() { ... })` arrives as a
    // placeholder; the tree-walker resolves it on read, as this does on bind.
    Some(match value {
        Value::Deferred(_) => value.force_deferred(),
        other => other,
    })
}

fn compile_function(
    body: &[Stmt],
    params: &[String],
    seen: Option<Rc<RefCell<HashSet<String>>>>,
) -> Result<Arc<FunctionProto>, String> {
    let span = Span::default();
    let decl = FunctionDecl {
        name: "__template".to_string(),
        params: params
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
        body: body.to_vec(),
        span,
    };
    let program = Program::new(vec![Stmt::new(StmtKind::function(decl), span, None)]);
    let module = Compiler::compile_template(&program, seen).map_err(|e| format!("compile: {e}"))?;
    module
        .main
        .chunk
        .constants
        .iter()
        .find_map(|c| match c {
            Constant::Function(proto) => Some(proto.clone()),
            _ => None,
        })
        .ok_or_else(|| "compile: no function".to_string())
}

/// The statements a template's nodes stand for, or the node only the
/// tree-walker renders. `assigned` collects the names a code block binds where
/// the layout would see them: not inside a `for` (its body is a scope of its
/// own on the tree-walker).
fn lower_nodes(
    nodes: &[TemplateNode],
    in_for: bool,
    assigned: &mut std::collections::BTreeSet<String>,
    all_assigned: &mut std::collections::BTreeSet<String>,
) -> Result<Vec<Stmt>, String> {
    let mut out = Vec::with_capacity(nodes.len());
    for node in nodes {
        match node {
            TemplateNode::Literal(text) => {
                let literal = Expr::new(ExprKind::StringLiteral(text.clone()), Span::default());
                out.push(write_stmt(literal, false));
            }
            TemplateNode::CoreOutput { expr, escaped, .. } => {
                out.push(write_stmt(expr.clone(), *escaped));
            }
            TemplateNode::CoreCodeBlock { stmts, .. } => {
                let names: Vec<String> = stmts.iter().filter_map(bare_name_assigned).collect();
                if !in_for {
                    assigned.extend(names.iter().cloned());
                }
                all_assigned.extend(names);
                out.extend(stmts.iter().cloned())
            }
            TemplateNode::If {
                condition,
                body,
                else_body,
                ..
            } => {
                let then_branch =
                    Box::new(block(lower_nodes(body, in_for, assigned, all_assigned)?));
                let else_branch = match else_body {
                    Some(nodes) => Some(Box::new(block(lower_nodes(
                        nodes,
                        in_for,
                        assigned,
                        all_assigned,
                    )?))),
                    None => None,
                };
                out.push(stmt(StmtKind::If {
                    condition: condition.clone(),
                    then_branch,
                    else_branch,
                }));
            }
            TemplateNode::For {
                var,
                index_var,
                iterable,
                body,
                ..
            } => {
                // The renderer iterates a hash as `[key, value]` pairs and
                // forces a `grouped {}` placeholder; `__tpl_iter` does both.
                let iterable = Expr::new(
                    ExprKind::Call {
                        callee: Box::new(Expr::new(
                            ExprKind::Variable("__tpl_iter".to_string()),
                            Span::default(),
                        )),
                        arguments: vec![Argument::Positional(iterable.clone())],
                    },
                    Span::default(),
                );
                out.push(stmt(StmtKind::For {
                    variable: var.clone(),
                    index_variable: index_var.clone(),
                    iterable,
                    body: Box::new(block(lower_nodes(body, true, assigned, all_assigned)?)),
                }));
            }
            // `<%- component "card", props do %> body <%- end %>`: the body is
            // captured as the `content` slot, then the component renders with
            // it, as the renderer does (`render_component_block`). A named slot
            // (`c.slot("x") do … end`) is a `content_for` in the body.
            TemplateNode::Component { parts, body, .. } => {
                out.push(call_stmt("__tpl_capture_start", vec![]));
                out.extend(lower_nodes(body, in_for, assigned, all_assigned)?);
                let props = parts
                    .props
                    .clone()
                    .unwrap_or_else(|| Expr::new(ExprKind::Null, Span::default()));
                out.push(call_stmt(
                    "__tpl_component",
                    vec![
                        parts.name.clone(),
                        props,
                        call_expr("__tpl_capture_end", vec![]),
                    ],
                ));
            }
            TemplateNode::ContentFor { name, body, .. } => {
                out.push(call_stmt("__tpl_capture_start", vec![]));
                out.extend(lower_nodes(body, in_for, assigned, all_assigned)?);
                let name = Expr::new(ExprKind::StringLiteral(name.clone()), Span::default());
                out.push(call_stmt(
                    "__tpl_content_for",
                    vec![name, call_expr("__tpl_capture_end", vec![])],
                ));
            }
            other => return Err(node_kind(other).to_string()),
        }
    }
    Ok(out)
}

fn call_expr(name: &str, args: Vec<Expr>) -> Expr {
    Expr::new(
        ExprKind::Call {
            callee: Box::new(Expr::new(
                ExprKind::Variable(name.to_string()),
                Span::default(),
            )),
            arguments: args.into_iter().map(Argument::Positional).collect(),
        },
        Span::default(),
    )
}

fn call_stmt(name: &str, args: Vec<Expr>) -> Stmt {
    stmt(StmtKind::Expression(call_expr(name, args)))
}

thread_local! {
    /// The `TemplateCache` rendering the view on this thread's VM, for
    /// `Op::TemplateComponent`: the component renders through it, as the
    /// tree-walker's partial renderer does. Set only for the render's length.
    static RENDERING_CACHE: std::cell::Cell<*const crate::template::TemplateCache> =
        const { std::cell::Cell::new(std::ptr::null()) };
}

/// Sets [`RENDERING_CACHE`] for a render and clears it on drop.
struct RenderingCache;

impl RenderingCache {
    fn set(cache: &crate::template::TemplateCache) -> Self {
        RENDERING_CACHE.with(|c| c.set(cache as *const _));
        RenderingCache
    }
}

impl Drop for RenderingCache {
    fn drop(&mut self) {
        RENDERING_CACHE.with(|c| c.set(std::ptr::null()));
    }
}

/// A component block's render (`Op::TemplateComponent`), as the renderer's
/// `TemplateNode::Component` arm does it: the props when they are a hash, the
/// captured body as `content`, the name resolved under `components/` unless it
/// carries a `/` or a `.`.
pub(crate) fn render_component_block(
    name: &Value,
    props: &Value,
    content: Value,
) -> Result<String, String> {
    use crate::interpreter::value::{HashKey, HashPairs};
    let name = match name {
        Value::String(s) => s.to_string(),
        other => {
            return Err(format!(
                "component name must evaluate to string, got {}",
                other.type_name()
            ))
        }
    };
    let mut map = HashPairs::default();
    if let Value::Hash(props) = props {
        for (k, v) in props.borrow().iter() {
            map.insert(k.clone(), v.clone());
        }
    }
    map.insert(HashKey::String("content".into()), content);
    let data = Value::Hash(Rc::new(RefCell::new(map)));
    let path = if name.contains('/') || name.contains('.') {
        name
    } else {
        format!("components/{name}")
    };
    let rendering = RENDERING_CACHE.with(|c| c.get());
    if rendering.is_null() {
        return crate::interpreter::builtins::template::get_template_cache()?
            .render_partial(&path, &data);
    }
    // SAFETY: set by `render_view` from a `&TemplateCache` that outlives the
    // render, cleared when it ends (`RenderingCache`), and read only on this
    // thread, inside that render.
    unsafe { &*rendering }.render_partial(&path, &data)
}

/// The bare name a code-block statement binds, if any (`x = …`, `x += …`,
/// `let x = …`).
fn bare_name_assigned(stmt: &Stmt) -> Option<String> {
    match &stmt.kind {
        StmtKind::Let { name, .. } | StmtKind::Const { name, .. } => Some(name.clone()),
        StmtKind::Expression(expr) => match &expr.kind {
            ExprKind::Assign { target, .. } | ExprKind::CompoundAssign { target, .. } => {
                match &target.kind {
                    ExprKind::Variable(name) => Some(name.clone()),
                    _ => None,
                }
            }
            _ => None,
        },
        _ => None,
    }
}

fn node_kind(node: &TemplateNode) -> &'static str {
    match node {
        TemplateNode::Yield(_) => "yield",
        TemplateNode::ContentFor { .. } => "content_for",
        TemplateNode::FormWith { .. } => "form_with",
        TemplateNode::Component { .. } => "component",
        TemplateNode::Partial { .. } => "partial",
        _ => "other node",
    }
}

fn write_stmt(value: Expr, escaped: bool) -> Stmt {
    let name = if escaped {
        "__tpl_write"
    } else {
        "__tpl_write_raw"
    };
    let callee = Expr::new(ExprKind::Variable(name.to_string()), Span::default());
    stmt(StmtKind::Expression(Expr::new(
        ExprKind::Call {
            callee: Box::new(callee),
            arguments: vec![Argument::Positional(value)],
        },
        Span::default(),
    )))
}

fn block(stmts: Vec<Stmt>) -> Stmt {
    stmt(StmtKind::Block(stmts))
}

fn stmt(kind: StmtKind) -> Stmt {
    Stmt::new(kind, Span::default(), None)
}

// --- Serving views on the VM ---

thread_local! {
    /// This worker thread's VMs for views, and with each the names a view resolves past
    /// its data (`core_eval::template_env_bindings`: builtins, view and route
    /// helpers). The VM's globals are the worker's (`set_worker_globals`), so
    /// a model method a view calls resolves the classes it would on an action;
    /// a name written in the view itself still resolves in the view's scope.
    static VIEW_VMS: RefCell<Vec<ViewVm>> = const { RefCell::new(Vec::new()) };
    /// Bumped whenever the pool is emptied: a VM taken out before then is
    /// not put back, since it holds the old globals.
    static POOL_GENERATION: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// The worker VM's globals, set by serve at boot and on reload.
    static WORKER_GLOBALS: RefCell<Option<ahash::AHashMap<String, Value>>> =
        const { RefCell::new(None) };
    /// Views compiled on this thread, keyed by their parsed nodes (the `Arc`
    /// is kept in the entry so its address cannot be reused). `None`
    /// remembers a view that needs the tree-walker.
    #[allow(clippy::type_complexity)]
    static COMPILED: RefCell<ahash::AHashMap<usize, (Arc<Vec<TemplateNode>>, Option<Rc<VmTemplate>>)>> =
        RefCell::new(ahash::AHashMap::new());
}

/// Whether views render on the VM: in production, unless `SOLI_VM_VIEWS=0`.
/// `--dev` keeps the tree-walker, as it does for actions.
pub fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    !crate::interpreter::builtins::template::is_dev_mode()
        && *ON.get_or_init(|| {
            !matches!(
                std::env::var("SOLI_VM_VIEWS").as_deref(),
                Ok("0") | Ok("off") | Ok("false")
            )
        })
}

/// Render a view on this thread's VM, or `None` when the tree-walker must:
/// views off, a view that does not compile, a view rendered from inside
/// another (the VM is in use), or a render the VM refused or failed. A failed
/// render wrote nothing; the caller renders the view again on the tree-walker,
/// which raises the error with its file and line.
pub fn render_view(
    nodes: &Arc<Vec<TemplateNode>>,
    data: &Value,
    env: &Rc<RefCell<crate::interpreter::environment::Environment>>,
    cache: &crate::template::TemplateCache,
    path: &std::path::Path,
) -> Option<Rendered> {
    if !enabled() {
        check_log("[vm-views] off");
        return None;
    }
    let key = Arc::as_ptr(nodes) as usize;
    let compiled = COMPILED.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some((_, compiled)) = cache.get(&key) {
            return compiled.clone();
        }
        let compiled = VmTemplate::compile(nodes).ok().map(Rc::new);
        cache.insert(key, (nodes.clone(), compiled.clone()));
        compiled
    })?;
    // A VM from this thread's pool, held by this render alone: a component
    // or partial rendered from inside it takes another.
    let mut view_vm = ViewVm::take();
    let args = compiled.bind(data, &view_vm.names);
    view_vm.vm.tree_walk_env = Some(env.clone());
    view_vm.vm.template_content_before = None;
    let outcome = {
        let _cache = RenderingCache::set(cache);
        compiled.run(&mut view_vm.vm, args)
    };
    view_vm.vm.tree_walk_env = None;
    // What a failed render must take back: the tree-walker renders the view
    // again, and its `content_for` blocks would be captured twice.
    if let Some(before) = view_vm.vm.template_content_before.take() {
        if outcome.is_err() {
            crate::template::content_store::restore(before);
        }
    }
    view_vm.put_back();
    match outcome {
        Ok(rendered) => Some(rendered),
        Err(err) => {
            check_log(&format!(
                "[vm-views] {}: render failed, tree-walker used: {err}",
                path.display()
            ));
            // A refusal is about the view (a function written bare), so it
            // keeps the tree-walker from now on; any other error may be about
            // this request's data, and the view stays compiled.
            if err.is_engine_fallback() {
                COMPILED.with(|cache| {
                    if let Some(entry) = cache.borrow_mut().get_mut(&key) {
                        entry.1 = None;
                    }
                });
            }
            None
        }
    }
}

/// `SOLI_VM_VIEWS_CHECK=<file>`: render every compiled view on both engines
/// and append to `<file>` where they differ (see
/// `TemplateCache::render_uncached`). A diagnostic for this change's tests.
pub fn check_enabled() -> bool {
    check_file().is_some()
}

fn check_file() -> Option<&'static str> {
    static FILE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    FILE.get_or_init(|| {
        std::env::var("SOLI_VM_VIEWS_CHECK")
            .ok()
            .filter(|v| !v.is_empty())
    })
    .as_deref()
}

fn check_log(line: &str) {
    use std::io::Write;
    if let Some(path) = check_file() {
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(f, "{line}");
        }
    }
}

/// Print the first place a view's VM render differs from the tree-walker's.
pub fn report_difference(path: &str, on_vm: &str, tree: &str) {
    if on_vm == tree {
        check_log(&format!("[vm-views] {path}: same ({} bytes)", tree.len()));
        return;
    }
    let at = on_vm
        .bytes()
        .zip(tree.bytes())
        .position(|(a, b)| a != b)
        .unwrap_or_else(|| on_vm.len().min(tree.len()));
    let window = |s: &str| -> String {
        let start = s.floor_char_boundary(at.saturating_sub(80));
        let end = s.ceil_char_boundary((at + 120).min(s.len()));
        s[start..end].replace('\n', "\\n")
    };
    check_log(&format!(
        "[vm-views] {path}: differs at byte {at}\n  vm:   {}\n  tree: {}",
        window(on_vm),
        window(tree)
    ));
}

/// The worker VM's globals, for the views' VM. Called by serve once the
/// worker VM has them, and again whenever it reloads them.
pub fn set_worker_globals(globals: &ahash::AHashMap<String, Value>) {
    WORKER_GLOBALS.with(|g| *g.borrow_mut() = Some(globals.clone()));
    empty_pool();
}

/// One of this thread's view VMs, with the names a view resolves past its
/// data and the pool generation it was made in.
struct ViewVm {
    vm: Vm,
    names: ahash::AHashMap<String, Value>,
    generation: u64,
}

impl ViewVm {
    /// A VM from the pool, or a new one.
    fn take() -> Self {
        let generation = POOL_GENERATION.with(|g| g.get());
        if let Some(vm) = VIEW_VMS.with(|pool| pool.borrow_mut().pop()) {
            return vm;
        }
        let names: ahash::AHashMap<String, Value> =
            crate::template::core_eval::template_env_bindings()
                .into_iter()
                .collect();
        let mut vm = Vm::new();
        vm.tree_walk_functions = true;
        vm.globals = WORKER_GLOBALS
            .with(|g| g.borrow().clone())
            .unwrap_or_else(|| names.clone());
        ViewVm {
            vm,
            names,
            generation,
        }
    }

    /// Back into the pool, unless the pool was emptied meanwhile.
    fn put_back(self) {
        if self.generation == POOL_GENERATION.with(|g| g.get()) {
            VIEW_VMS.with(|pool| pool.borrow_mut().push(self));
        }
    }
}

fn empty_pool() {
    POOL_GENERATION.with(|g| g.set(g.get() + 1));
    VIEW_VMS.with(|pool| pool.borrow_mut().clear());
}

/// Drop this thread's compiled views and view VM: the views or the helpers
/// they resolve changed.
pub fn reset_thread() {
    empty_pool();
    COMPILED.with(|cache| {
        if let Ok(mut cache) = cache.try_borrow_mut() {
            cache.clear();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template::parser::parse_template;

    use crate::interpreter::value::{HashKey, HashPairs};
    use crate::template::renderer::render_nodes;

    fn hash(pairs: Vec<(&str, Value)>) -> Value {
        let mut map = HashPairs::default();
        for (k, v) in pairs {
            map.insert(HashKey::String(k.into()), v);
        }
        Value::Hash(Rc::new(RefCell::new(map)))
    }

    fn array(items: Vec<Value>) -> Value {
        Value::Array(Rc::new(RefCell::new(items)))
    }

    fn data() -> Value {
        let rows = array(
            (1..=3)
                .map(|i| {
                    hash(vec![
                        ("id", Value::Int(i)),
                        ("title", Value::String(format!("Post <{i}> & co").into())),
                        (
                            "tags",
                            array(vec![Value::String("a".into()), Value::String("b".into())]),
                        ),
                    ])
                })
                .collect(),
        );
        hash(vec![
            ("rows", rows),
            ("title", Value::String("Hello \"world\"".into())),
            ("count", Value::Int(3)),
            ("ratio", Value::Float(2.5)),
            ("flag", Value::Bool(true)),
            ("nothing", Value::Null),
            ("empty", array(vec![])),
            (
                "settings",
                hash(vec![
                    ("theme", Value::String("dark".into())),
                    ("size", Value::Int(2)),
                ]),
            ),
        ])
    }

    /// Every template here renders byte for byte the same on both engines.
    #[test]
    fn the_vm_renders_what_the_tree_walker_renders() {
        let cases = [
            "<p><%= title %></p>",
            "<p><%- title %></p>",
            "<%= count %>|<%= ratio %>|<%= flag %>|<%= nothing %>|<%= undefined_name %>|",
            "<%= rows.length %> <%= empty.length %> <%= title.upcase %> <%= title.length %>",
            "<% if flag %>yes<% else %>no<% end %>",
            "<% if count > 5 %>big<% elsif count > 2 %>mid<% else %>small<% end %>",
            "<% unless nothing %>none<% end %>",
            "<% for row in rows %><li><%= row[\"id\"] %>: <%= row[\"title\"] %></li><% end %>",
            "<% for row, i in rows %><%= i %>=<%= row.title %>;<% end %>",
            "<% for pair in settings %>[<%= pair[0] %>:<%= pair[1] %>]<% end %>",
            "<% for row in rows %><% for tag in row[\"tags\"] %><%= row.id %><%= tag %><% end %><% end %>",
            "<% total = 0 %><% for row in rows %><% total = total + row.id %><% end %>sum=<%= total %>",
            "<% label = title.downcase %><b><%= label %></b>",
            "<%= \"#{count} items, #{ratio * 2}\" %>",
            "<%= rows.map { |r| r.id * 10 }.join(\",\") %>",
            "<%= settings.theme %>-<%= settings[\"size\"] + 1 %>",
            "<%= nothing.nil? %> <%= title.blank? %> <%= empty.length > 0 %>",
            "<% for row in empty %>never<% end %>after",
            "<% rows.each do |row| %><%= row.id %><% end %>",
            "<%= @title %>|<%= @undefined_ivar %>|",
            "<% title = @title.upcase %><%= title %>|<% count = count + 1 %><%= count %>",
            "<% settings = @settings ?? {} %><%= settings.keys().length %>",
            "<%= locals[\"count\"] %>|<%= locals[\"absent\"].to_s %>|",
        ];
        // What serve's renderer enters: an undefined name, and an `@ivar`
        // with no instance, read as nil / the bare local.
        let _lenient = crate::interpreter::executor::enter_template_lenient_vars();
        let mut vm = Vm::new();
        let data = data();
        for source in cases {
            let nodes = parse_template(source).unwrap_or_else(|e| panic!("{source}: {e}"));
            let tree =
                render_nodes(&nodes, &data, None).unwrap_or_else(|e| panic!("{source}: {e}"));
            let compiled = VmTemplate::compile(&nodes)
                .unwrap_or_else(|e| panic!("{source}: does not compile: {e}"));
            let on_vm = compiled
                .render(&mut vm, &data)
                .unwrap_or_else(|e| panic!("{source}: {e}"))
                .html;
            assert_eq!(on_vm, tree, "{source}");
        }
    }

    /// What a view assigns where its layout sees it on the tree-walker comes
    /// back with the page; a `for` body's assignments do not.
    #[test]
    fn a_view_hands_its_assignments_to_the_layout() -> Result<(), String> {
        let nodes = parse_template(
            "<% heading = \"Posts\" %><% if flag %><% note = 1 %><% end %>\
             <% for row in rows %><% inner = row %><% end %><%= heading %>",
        )?;
        let compiled = VmTemplate::compile(&nodes)?;
        let rendered = compiled
            .render(&mut Vm::new(), &data())
            .map_err(|e| e.to_string())?;
        let names: Vec<&str> = rendered.assigned.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, ["heading", "note"]);
        assert_eq!(rendered.html, "Posts");
        Ok(())
    }

    /// A function written bare is called by the tree-walker's renderer; the
    /// VM refuses it so the caller renders there instead.
    #[test]
    fn a_bare_function_output_falls_back() -> Result<(), String> {
        let nodes = parse_template("<%= helper %>")?;
        let compiled = VmTemplate::compile(&nodes)?;
        let helper = Value::NativeFunction(crate::interpreter::value::NativeFunction::new(
            "helper",
            Some(0),
            |_| Ok(Value::String("x".into())),
        ));
        let Err(err) = compiled.render(&mut Vm::new(), &hash(vec![("helper", helper)])) else {
            return Err("a bare function was written instead of refused".to_string());
        };
        assert!(err.is_engine_fallback(), "{err}");
        Ok(())
    }

    /// `SOLI_VM_TEMPLATE_DIR=<app>/app/views cargo test … -- --ignored
    /// --nocapture`: how many of an app's views compile, and why the rest do
    /// not.
    #[test]
    #[ignore]
    fn coverage_report() {
        let Ok(dir) = std::env::var("SOLI_VM_TEMPLATE_DIR") else {
            return;
        };
        let mut reasons: std::collections::BTreeMap<String, usize> = Default::default();
        let (mut total, mut ok) = (0, 0);
        let mut stack = vec![std::path::PathBuf::from(dir)];
        while let Some(path) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&path) else {
                continue;
            };
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                if !p.to_string_lossy().ends_with(".slv") {
                    continue;
                }
                total += 1;
                let Ok(source) = std::fs::read_to_string(&p) else {
                    continue;
                };
                let reason = match parse_template(&source) {
                    Err(_) => "parse error".to_string(),
                    Ok(nodes) => match VmTemplate::compile(&nodes) {
                        Ok(_) => {
                            ok += 1;
                            continue;
                        }
                        Err(reason) => reason.chars().take(60).collect(),
                    },
                };
                *reasons.entry(reason).or_default() += 1;
            }
        }
        println!("compiled {ok}/{total}");
        for (reason, n) in reasons {
            println!("{n:5}  {reason}");
        }
    }
}
