//! Frame slots for the entry script's own top-level variables.
//!
//! At file scope `n = 0`, `let n = 0` and `const N = 0` compiled to globals:
//! `DefineGlobal` or `SetGlobal`, read back with `GetGlobal`. Every access
//! hashed the name, `SetGlobal` left the fast tier, and none of the local
//! super-instructions (`IncrLocal`, `AddLocalsInPlace`, the fused compare and
//! jump) applied, so a `while` loop at file scope ran 4 to 6 times slower than
//! the same loop in a `def`.
//!
//! A global is only needed when something besides the script's own top-level
//! code can reach the name: a function, a lambda, a block or a class body that
//! names it, code outside the program (a prelude function) reading a builtin
//! the script assigns, or `defined` / `const_get` asking for a name known only
//! at run time. [`plan`] picks the names none of these can reach, and
//! `Compiler::compile_entry_script` declares each one where it is first
//! assigned, the way `let` declares a local in a function body: the value
//! stays on the main frame's stack as the variable's slot.
//!
//! Only the program `soli script.sl` runs is compiled this way (`soli -e` and
//! script executables too). Anything compiled with `Compiler::compile` or for
//! `soli serve` keeps its top-level names as globals, which their callers read
//! back from `Vm::globals`.

use std::cell::RefCell;
use std::collections::HashSet;

use crate::ast::expr::{Expr, ExprKind};
use crate::ast::stmt::{ClassDecl, ImportSpecifier, Stmt, StmtKind};
use crate::ast::walk;

/// Builtins that look a global up by a name computed at run time. A program
/// that calls one keeps every top-level variable a global.
const DYNAMIC_LOOKUPS: &[&str] = &["defined", "const_get"];

/// The most top-level variables one script keeps in frame slots; any later
/// ones stay globals. Each planned name is compared against every global the
/// program resolves while its declaration is pending (`in_place_decls` is
/// searched in order), and slot numbers are `u16`, so a generated script with
/// thousands of top-level variables must not plan them all. A hand-written
/// script has far fewer.
const MAX_SLOTS: usize = 256;

/// The top-level statements of `statements` that declare a variable the entry
/// script can keep in a frame slot: `(index, name)` for each `name = value`,
/// `let name = value` (or a bare `let name`) and `const NAME = value` that
/// stands as a statement of its own at file scope, when:
///
/// - no statement before it, and not its own value, mentions `name`, so
///   nothing can read or capture the slot before it is filled. Top-level
///   statements run once and in order, and none of them jumps back (the rule
///   `compiler_hoist::in_place_declarations` applies to a function body);
/// - no function, lambda, block, class, enum or module anywhere in the program
///   mentions `name`. Such a body would capture the slot, or read the global
///   from a frame of its own;
/// - `is_builtin(name)` is false. A script that assigns a builtin's name
///   replaces the builtin for every function that reads it, prelude functions
///   included, as on the tree-walker, and that needs the global;
/// - the program calls neither `defined` nor `const_get`.
///
/// At most [`MAX_SLOTS`] names are planned, the first ones declared.
///
/// "Mentions" is deliberately coarse: any occurrence of the identifier, read,
/// written, declared or bound. That covers a parameter, a loop or catch
/// variable, a match binding, `#{name}` in an SDBQL block, and a class's
/// superclass, includes, fields and methods.
pub(super) fn plan(statements: &[Stmt], is_builtin: &dyn Fn(&str) -> bool) -> Vec<(usize, String)> {
    let nested = names_in_nested_scopes(statements);
    let mut seen: HashSet<String> = HashSet::new();
    let mut planned = Vec::new();
    for (index, stmt) in statements.iter().enumerate() {
        let declared = declaration(stmt);
        let mentioned = RefCell::new(HashSet::new());
        match declared {
            Some((_, Some(value))) => names_in_expr(value, &mentioned),
            Some((_, None)) => {}
            None => names_in_stmt(stmt, &mentioned),
        }
        let mentioned = mentioned.into_inner();
        if let Some((name, _)) = declared {
            if planned.len() < MAX_SLOTS
                && !seen.contains(name)
                && !mentioned.contains(name)
                && !nested.contains(name)
                && !is_builtin(name)
            {
                planned.push((index, name.to_string()));
            }
            seen.insert(name.to_string());
        }
        seen.extend(mentioned);
    }
    if DYNAMIC_LOOKUPS.iter().any(|name| seen.contains(*name)) {
        return Vec::new();
    }
    planned
}

/// The name a statement can declare and its value: `name = value`,
/// `let name = value` (no value for a bare `let name`), `const NAME = value`.
fn declaration(stmt: &Stmt) -> Option<(&str, Option<&Expr>)> {
    match &stmt.kind {
        StmtKind::Let {
            name, initializer, ..
        } => Some((name.as_str(), initializer.as_ref())),
        StmtKind::Const {
            name, initializer, ..
        } => Some((name.as_str(), Some(initializer))),
        _ => super::compiler_hoist::assignment_statement(stmt)
            .map(|(name, value)| (name, Some(value))),
    }
}

/// Every name mentioned inside a function, lambda (a block is one), class,
/// enum, module or interface of `statements`, at any depth, and every name an
/// import binds.
fn names_in_nested_scopes(statements: &[Stmt]) -> HashSet<String> {
    let found = RefCell::new(HashSet::new());
    let mut on_stmt = |stmt: &Stmt| {
        if matches!(
            stmt.kind,
            StmtKind::Function(_)
                | StmtKind::Class(_)
                | StmtKind::Enum(_)
                | StmtKind::Interface(_)
                | StmtKind::Import(_)
        ) {
            names_in_stmt(stmt, &found);
        }
    };
    let mut on_expr = |expr: &Expr| {
        if matches!(expr.kind, ExprKind::Lambda { .. }) {
            names_in_expr(expr, &found);
        }
    };
    for stmt in statements {
        walk::walk_stmt(stmt, &mut on_stmt, &mut on_expr);
    }
    found.into_inner()
}

/// Add to `out` every name `stmt` mentions, nested scopes included.
fn names_in_stmt(stmt: &Stmt, out: &RefCell<HashSet<String>>) {
    let mut on_stmt = |stmt: &Stmt| note_stmt(stmt, out);
    let mut on_expr = |expr: &Expr| note_expr(expr, out);
    walk::walk_stmt(stmt, &mut on_stmt, &mut on_expr);
}

/// Add to `out` every name `expr` mentions, nested scopes included.
fn names_in_expr(expr: &Expr, out: &RefCell<HashSet<String>>) {
    let mut on_stmt = |stmt: &Stmt| note_stmt(stmt, out);
    let mut on_expr = |expr: &Expr| note_expr(expr, out);
    walk::walk_expr(expr, &mut on_stmt, &mut on_expr);
}

/// The names a statement binds or declares itself. What it contains is
/// reached by the walk that calls this.
fn note_stmt(stmt: &Stmt, out: &RefCell<HashSet<String>>) {
    let note = |name: &str| {
        out.borrow_mut().insert(name.to_string());
    };
    match &stmt.kind {
        StmtKind::Let { name, .. } | StmtKind::Const { name, .. } => note(name),
        StmtKind::For {
            variable,
            index_variable,
            ..
        } => {
            note(variable);
            if let Some(index) = index_variable {
                note(index);
            }
        }
        StmtKind::Try { catch_clauses, .. } => {
            for clause in catch_clauses {
                if let Some(var) = &clause.var_name {
                    note(var);
                }
            }
        }
        StmtKind::Function(decl) => {
            note(&decl.name);
            for param in &decl.params {
                note(&param.name);
            }
        }
        StmtKind::Class(decl) => note_class(decl, &note),
        // What is compiled is the class it lowers to: its variant constants,
        // payload constructors and the helpers they call.
        StmtKind::Enum(decl) => {
            let lowered = Stmt::new(StmtKind::class(decl.lower_to_class()), stmt.span, None);
            names_in_stmt(&lowered, out);
        }
        StmtKind::Interface(decl) => note(&decl.name),
        StmtKind::Import(decl) => match &decl.specifier {
            ImportSpecifier::All => {}
            ImportSpecifier::Named(items) => {
                for item in items {
                    note(&item.name);
                    if let Some(alias) = &item.alias {
                        note(alias);
                    }
                }
            }
            ImportSpecifier::Namespace(name) => note(name),
        },
        _ => {}
    }
}

/// The names a class declares or resolves by name, its nested classes' too.
/// The walk reaches the statements and expressions of its bodies.
fn note_class(decl: &ClassDecl, note: &dyn Fn(&str)) {
    note(&decl.name);
    if let Some(superclass) = &decl.superclass {
        note(superclass);
    }
    for name in decl
        .interfaces
        .iter()
        .chain(&decl.includes)
        .chain(&decl.extends)
    {
        note(name);
    }
    for field in &decl.fields {
        note(&field.name);
    }
    for method in decl.methods.iter().chain(&decl.concern_class_methods) {
        note(&method.name);
        for param in &method.params {
            note(&param.name);
        }
    }
    if let Some(ctor) = &decl.constructor {
        for param in &ctor.params {
            note(&param.name);
        }
    }
    for nested in &decl.nested_classes {
        note_class(nested, note);
    }
}

/// The names an expression reads or binds itself.
fn note_expr(expr: &Expr, out: &RefCell<HashSet<String>>) {
    let note = |name: &str| {
        out.borrow_mut().insert(name.to_string());
    };
    match &expr.kind {
        ExprKind::Variable(name) => note(name),
        // A view reads, calls and assigns `@name` as its local `name`
        // (`template_ivar`).
        ExprKind::Member { object, name } if matches!(object.kind, ExprKind::This) => note(name),
        // `#{name}` is read as the variable `name`, from the interpolation's
        // text (`compile_sdql_block`): that name counts, and every word in it.
        ExprKind::SdqlBlock { interpolations, .. } => {
            for interpolation in interpolations {
                let text = interpolation.expr.as_str();
                note(text.trim_start_matches('{').trim_end_matches('}').trim());
                for word in text.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
                    note(word);
                }
            }
        }
        ExprKind::Lambda { params, .. } => {
            for param in params {
                note(&param.name);
            }
        }
        ExprKind::ListComprehension { variable, .. }
        | ExprKind::HashComprehension { variable, .. } => note(variable),
        ExprKind::Match { arms, .. } => {
            for arm in arms {
                super::compiler_hoist::pattern_names(&arm.pattern, &note);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::stmt::Program;
    use crate::interpreter::value::Value;
    use crate::lexer::Scanner;
    use crate::parser::Parser;
    use crate::vm::chunk::{CompiledModule, Constant};
    use crate::vm::compiler::Compiler;
    use crate::vm::opcode::Op;

    fn parse(src: &str) -> Program {
        let tokens = Scanner::new(src).scan_tokens().expect("lex");
        Parser::new(tokens).parse().expect("parse")
    }

    /// The names `plan` keeps in frame slots, with no builtin in the way.
    fn planned(src: &str) -> Vec<String> {
        planned_with(src, &|_| false)
    }

    fn planned_with(src: &str, is_builtin: &dyn Fn(&str) -> bool) -> Vec<String> {
        plan(&parse(src).statements, is_builtin)
            .into_iter()
            .map(|(_, name)| name)
            .collect()
    }

    /// `src` compiled as the entry script, `result` (and anything named
    /// `out_*`) kept a global so that a test can read it back.
    fn compile_entry(src: &str) -> CompiledModule {
        let is_builtin = |name: &str| name == "result" || name.starts_with("out_");
        Compiler::compile_entry_script(&parse(src), None, &is_builtin).expect("compile")
    }

    /// The global names the main chunk's global ops refer to.
    fn global_ops(module: &CompiledModule) -> Vec<String> {
        let chunk = &module.main.chunk;
        let name = |idx: u16| match &chunk.constants[idx as usize] {
            Constant::String(s) => s.to_string(),
            other => panic!("not a name: {other:?}"),
        };
        chunk
            .code
            .iter()
            .filter_map(|op| match op {
                Op::GetGlobal(idx)
                | Op::SetGlobal(idx)
                | Op::DefineGlobal(idx)
                | Op::AddAssignGlobal(idx)
                | Op::AppendGlobal(idx)
                | Op::AppendGlobalConst(idx, _)
                | Op::HashGetGlobalConst(idx, _)
                | Op::HashSetGlobalConst(idx, _)
                | Op::HashGetGlobalConst2(idx, _, _) => Some(name(*idx)),
                _ => None,
            })
            .collect()
    }

    /// Run `module` on a VM seeded with the builtins and show `result`.
    fn result_of(module: &CompiledModule) -> String {
        let interpreter = crate::interpreter::Interpreter::new();
        let mut vm = crate::vm::Vm::new();
        for (name, value) in interpreter.environment.borrow().get_all_bindings() {
            vm.globals.insert(name, value);
        }
        if let Err(err) = vm.execute(&module.main) {
            panic!("vm: {err}");
        }
        let result: &Value = vm.globals.get("result").expect("a result global");
        result.to_string()
    }

    // ---------- which names are kept in frame slots ----------

    #[test]
    fn straight_line_top_level_variables_are_planned() {
        assert_eq!(
            planned("i = 0\nsum = 0\nwhile i < 3\n  sum = sum + i\n  i += 1\nend\nprint(sum)"),
            ["i", "sum"]
        );
    }

    #[test]
    fn let_and_const_declarations_are_planned() {
        assert_eq!(
            planned("let a = 1\nlet b\nconst LIMIT = 2\nb = a + LIMIT\nprint(b)"),
            ["a", "b", "LIMIT"]
        );
    }

    #[test]
    fn a_name_a_def_lambda_or_block_mentions_stays_global() {
        assert_eq!(
            planned(
                "total = 0\n[1, 2].each do |x|\n  total = total + x\nend\ncount = 0\ndef bump()\n  count\nend\nseen = 0\nget = fn() { seen }\nlast = 1\nprint(total, bump(), get(), last)"
            ),
            ["get", "last"]
        );
    }

    #[test]
    fn a_parameter_of_a_def_or_lambda_counts_as_a_mention() {
        assert_eq!(
            planned(
                "n = 1\nk = 2\ndef twice(n)\n  n * 2\nend\nf = fn(k) { k }\nprint(twice(n), f(k))"
            ),
            ["f"]
        );
    }

    #[test]
    fn a_class_body_keeps_its_names_global_without_ending_the_plan() {
        assert_eq!(
            planned(
                "limit = 3\nBase = 1\nclass Box < Base\n  def cap\n    limit\n  end\nend\nother = 1\nprint(other)"
            ),
            ["other"]
        );
    }

    /// Its name, its variants and the payload fields its constructors take.
    #[test]
    fn an_enum_keeps_its_names_global() {
        assert_eq!(
            planned(
                "Shade = 1\nLight = 2\nlevel = 3\nenum Shade { Light, Dark(level: Int) }\nreason = 4\nprint(reason)"
            ),
            ["reason"]
        );
    }

    #[test]
    fn a_read_before_the_first_assignment_keeps_the_name_global() {
        assert_eq!(
            planned("print(x) rescue nil\nx = 1\ny = x\nprint(y)"),
            ["y"]
        );
    }

    #[test]
    fn the_assignments_own_value_counts_as_a_mention() {
        assert_eq!(planned("x = [x]\ny = 2\nprint(x, y)"), ["y"]);
    }

    #[test]
    fn a_first_assignment_in_a_block_or_loop_keeps_the_name_global() {
        assert_eq!(
            planned(
                "if true\n  a = 1\nend\na = 2\nfor i in [1]\n  c = i\nend\nd = 3\nprint(a, c, d)"
            ),
            ["d"]
        );
    }

    #[test]
    fn a_comprehension_variable_or_match_binding_counts_as_a_mention() {
        assert_eq!(
            planned(
                "r = [v for v in [1]]\nv = 2\nm = match 1 { w => w }\nw = 3\nprint(r, v, m, w)"
            ),
            ["r", "m"]
        );
    }

    #[test]
    fn a_builtin_the_script_assigns_stays_global() {
        assert_eq!(
            planned_with("str = \"x\"\nother = 1\nprint(str, other)", &|name| name
                == "str"),
            ["other"]
        );
    }

    #[test]
    fn defined_or_const_get_keeps_every_name_global() {
        assert!(planned("x = 1\nprint(defined(\"x\"))").is_empty());
        assert!(planned("x = 1\nprint(const_get(\"x\"))").is_empty());
        assert!(planned("x = 1\ndef probe()\n  defined(\"x\")\nend\nprint(probe())").is_empty());
    }

    #[test]
    fn an_sdbql_interpolation_mentions_its_variable() {
        assert_eq!(
            planned(
                "rows = @sdbql{ FOR u IN users FILTER u.age > #{min_age} RETURN u }\nmin_age = 1\nprint(rows, min_age)"
            ),
            ["rows"]
        );
    }

    /// Past the cap the later names stay globals, and the script still runs.
    #[test]
    fn at_most_max_slots_names_are_planned() {
        let last = MAX_SLOTS + 9;
        let declarations: String = (0..=last).map(|i| format!("v{i} = {i}\n")).collect();
        let names = planned(&declarations);
        assert_eq!(names.len(), MAX_SLOTS);
        assert_eq!(names[MAX_SLOTS - 1], format!("v{}", MAX_SLOTS - 1));

        let module = compile_entry(&format!("{declarations}result = [v0, v{last}]\n"));
        let globals = global_ops(&module);
        assert!(!globals.iter().any(|name| name == "v0"), "{globals:?}");
        assert!(globals.iter().any(|name| *name == format!("v{last}")));
        assert_eq!(result_of(&module), format!("[0, {last}]"));
    }

    #[test]
    fn a_second_declaration_is_not_planned_again() {
        assert_eq!(
            planned("x = 1\nlet x = 2\nconst Y = 1\nprint(x, Y)"),
            ["x", "Y"]
        );
    }

    // ---------- what is emitted ----------

    /// The loop of the audit: no global op is left for `i` or `sum`, and the
    /// local super-instructions apply.
    #[test]
    fn a_top_level_loop_compiles_to_local_ops() {
        let module = compile_entry(
            "i = 0\nsum = 0\nwhile i < 10\n  sum = sum + i\n  i += 1\nend\nresult = sum",
        );
        assert_eq!(global_ops(&module), ["result"]);
        let code = &module.main.chunk.code;
        assert!(
            code.iter().any(|op| matches!(op, Op::IncrLocal(_))),
            "got: {code:?}"
        );
        assert_eq!(result_of(&module), "45");
    }

    /// `Compiler::compile` is not the entry script: its top-level names stay
    /// globals, which its callers read back.
    #[test]
    fn a_plain_compile_keeps_top_level_variables_global() {
        let program = parse("i = 0\nwhile i < 3\n  i += 1\nend");
        let module = Compiler::compile(&program).expect("compile");
        assert!(global_ops(&module).iter().any(|name| name == "i"));
    }

    #[test]
    fn a_name_a_block_mentions_keeps_its_global_ops() {
        let module = compile_entry(
            "total = 0\nn = 3\n[1, 2].each do |x|\n  total = total + x\nend\nresult = [total, n]",
        );
        let globals = global_ops(&module);
        assert!(globals.iter().any(|name| name == "total"), "{globals:?}");
        assert!(!globals.iter().any(|name| name == "n"), "{globals:?}");
        assert_eq!(result_of(&module), "[3, 3]");
    }

    /// `let`, `const`, a bare `let`, a second `let` of the same name and a
    /// block-scoped `let` that shadows a slot.
    #[test]
    fn declarations_and_redeclarations_keep_their_values() {
        let module = compile_entry(
            "let a = 1\nconst B = 10\nlet c\nc = a + B\nlet a = a + 100\nif true\n  let c = 0\n  a = a + c\nend\nresult = [a, B, c]",
        );
        assert_eq!(global_ops(&module), ["result"]);
        assert_eq!(result_of(&module), "[101, 10, 11]");
    }

    /// A comprehension after another array element is compiled in a lambda
    /// of its own (`wrap_in_lambda`), which reads the slots through upvalues.
    #[test]
    fn a_wrapped_comprehension_reads_the_slot() {
        let module = compile_entry(
            "k = 3\nxs = [1, 2]\nresult = [k, [x * k for x in xs], match k { 3 => \"three\", _ => \"other\" }]",
        );
        assert_eq!(global_ops(&module), ["result"]);
        let wrapped = module.main.chunk.constants.iter().any(
            |constant| matches!(constant, Constant::Function(inner) if inner.name == "<expr>"),
        );
        assert!(wrapped, "the comprehension should be wrapped in a lambda");
        assert_eq!(result_of(&module), "[3, [3, 6], three]");
    }

    /// A slot survives a `try` at top level, a throw inside it and a `for`
    /// loop over it.
    #[test]
    fn slots_survive_try_and_for() {
        let module = compile_entry(
            "log = []\ncount = 0\ntry\n  count = count + 1\n  throw \"boom\"\ncatch e\n  log.push(\"caught\")\nend\nfor v in [1, 2]\n  count = count + v\nend\nresult = [count, log]",
        );
        assert_eq!(global_ops(&module), ["result"]);
        assert_eq!(result_of(&module), "[4, [caught]]");
    }

    // ---------- the plan cannot be broken silently ----------

    /// A plan that promotes a name a function captures is refused, and the
    /// entry-script compile falls back to globals.
    #[test]
    fn a_captured_promoted_name_refuses_the_plan() {
        let program = parse("x = 1\nget = fn() { x }\nresult = get()");
        let bad_plan = vec![(0, "x".to_string())];
        assert!(Compiler::compile_promoted(&program, None, &bad_plan).is_err());

        let module = Compiler::compile_entry_script(&program, None, &|name| name == "result")
            .expect("compile");
        assert!(global_ops(&module).iter().any(|name| name == "x"));
        assert_eq!(result_of(&module), "1");
    }

    /// A plan whose statement reads the name before it is declared is
    /// refused too.
    #[test]
    fn a_name_read_before_its_planned_declaration_refuses_the_plan() {
        let program = parse("y = x rescue 0\nx = 1\nresult = [x, y]");
        let bad_plan = vec![(1, "x".to_string())];
        assert!(Compiler::compile_promoted(&program, None, &bad_plan).is_err());
    }
}
