//! Views compiled to VM bytecode (prototype).
//!
//! A template whose nodes are all literal text, `<%= %>` / `<%- %>` output,
//! `if` / `unless` / `for` blocks and `<% %>` code compiles to one VM
//! function. Text and output become `__tpl_write` / `__tpl_write_raw`
//! intrinsics, which the compiler turns into `Op::TemplateWrite`: the value is
//! written into `Vm::template_out`, with no string built and no native call.
//!
//! The template's free names (`rows`, `title`, `h`) become the function's
//! parameters. At render time each gets the data value of that name, else (in
//! a partial or component) the controller's `@ivar` of that name, else the
//! template env's (a helper, a builtin), else nil — the order the
//! tree-walker's scope chain resolves them in, with its lenient undefined
//! read. A bare-call builtin (`clock`) stays a global, which the bytecode
//! calls where it is read bare, as in an action.
//!
//! A `form_with` / `fields_for` block and `<%= render "partial" %>` compile
//! too. A layout is never compiled: the tree-walker still renders it.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use crate::ast::expr::{Argument, Expr, ExprKind};
use crate::ast::stmt::{FunctionDecl, Parameter, Program, Stmt, StmtKind};
use crate::ast::types::{TypeAnnotation, TypeKind};
use crate::error::RuntimeError;
use crate::interpreter::environment::inherited_field;
use crate::interpreter::value::{hash_get_value, Instance, Value};
use crate::span::Span;
use crate::template::parser::TemplateNode;
use crate::vm::chunk::{Constant, FunctionProto};
use crate::vm::{Compiler, Vm, VmClosure};

/// A template compiled to a VM function.
pub struct VmTemplate {
    closure: Value,
    params: Vec<String>,
    /// The bare-call builtins (`clock`) the template reads. They are not
    /// parameters: the bytecode reads each as a global and calls it where it
    /// is read bare (see [`VmTemplate::bare_call_unresolved`]).
    bare_calls: Vec<String>,
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
    /// What serve renders on the VM: `compile`, for a template that does
    /// enough work to gain from it (`worth_compiling`).
    pub fn compile_for_serve(nodes: &[TemplateNode]) -> Result<Self, String> {
        if !worth_compiling(nodes) {
            return Err("too small to gain from the VM".to_string());
        }
        Self::compile(nodes)
    }

    /// Compile `nodes`, or say why they need the tree-walker.
    pub fn compile(nodes: &[TemplateNode]) -> Result<Self, String> {
        let mut assigned = std::collections::BTreeSet::new();
        let mut all_assigned = std::collections::BTreeSet::new();
        let mut saves = 0u32;
        let mut body = lower_nodes(nodes, false, &mut assigned, &mut all_assigned, &mut saves)?;
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
        // globals. The second compiles them as parameters, all but the
        // bare-call builtins.
        let seen = Rc::new(RefCell::new(HashSet::new()));
        compile_function(&body, &[], Some(seen.clone()))?;
        // A bare-call builtin the view does not assign stays a global: the
        // compiler calls it where it is read bare, as it does in an action, so
        // `clock - start` is the time here as on the tree-walker. As a
        // parameter it held the function itself, `clock - start` raised, and
        // every render was done again on the tree-walker.
        let (mut bare_calls, mut params): (Vec<String>, Vec<String>) =
            seen.borrow().iter().cloned().partition(|name| {
                crate::interpreter::builtins::is_bare_call_builtin(name)
                    && !all_assigned.contains(name)
            });
        bare_calls.sort();
        // A name the view assigns is a parameter too: `<% errors = @errors ?? {} %>`
        // reads the data's `errors` before replacing it, as the tree-walker
        // does. As a plain local it would read nil.
        params.extend(all_assigned);
        params.sort();
        params.dedup();
        let proto = compile_function(&body, &params, None)?;
        Ok(Self {
            closure: Value::VmClosure(Rc::new(VmClosure::new(proto, Vec::new()))),
            params,
            bare_calls,
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
        self.render_inheriting(vm, data, None)
    }

    /// [`VmTemplate::render`] for an include: a name the data lacks reads
    /// `inherited`'s `@ivar` before `vm`'s globals.
    pub(crate) fn render_inheriting(
        &self,
        vm: &mut Vm,
        data: &Value,
        inherited: Option<&Rc<RefCell<Instance>>>,
    ) -> Result<Rendered, RuntimeError> {
        if let Some(name) = self.bare_call_unresolved(data, inherited, &vm.globals, &vm.globals) {
            return Err(RuntimeError::EngineFallback(
                format!("a bare `{name}` that is not the builtin"),
                Span::default(),
            ));
        }
        let names = std::mem::take(&mut vm.globals);
        let args = self.bind(data, inherited, &names);
        vm.globals = names;
        self.run(vm, args)
    }

    /// The first bare-call builtin the template reads (`clock`) that is not,
    /// for this render, the builtin its bytecode calls: the data or the
    /// inherited controller binds the name (a controller's `@clock`, even to
    /// nil), or a helper or an app function took it where the tree-walker
    /// looks (`names`) or where the bytecode reads it (`globals`). Such a
    /// render goes to the tree-walker, which resolves the name as it always
    /// has.
    fn bare_call_unresolved(
        &self,
        data: &Value,
        inherited: Option<&Rc<RefCell<Instance>>>,
        names: &ahash::AHashMap<String, Value>,
        globals: &ahash::AHashMap<String, Value>,
    ) -> Option<&str> {
        self.bare_calls.iter().map(String::as_str).find(|&name| {
            data_binds(data, name)
                || inherited
                    .is_some_and(|controller| inherited_field(&controller.borrow(), name).is_some())
                || !is_builtin(names.get(name), name)
                || !is_builtin(globals.get(name), name)
        })
    }

    /// The template's arguments: each free name's data value, else the
    /// inherited controller's `@ivar` of that name, else its value in `names`
    /// (the view's scope past its data), else nil.
    fn bind(
        &self,
        data: &Value,
        inherited: Option<&Rc<RefCell<Instance>>>,
        names: &ahash::AHashMap<String, Value>,
    ) -> Vec<Value> {
        self.params
            .iter()
            .map(|name| {
                data_value(data, name)
                    .or_else(|| inherited.and_then(|controller| inherited_value(controller, name)))
                    .or_else(|| names.get(name).cloned())
                    .unwrap_or(Value::Null)
            })
            .collect()
    }

    fn run(&self, vm: &mut Vm, args: Vec<Value>) -> Result<Rendered, RuntimeError> {
        vm.template_out.clear();
        vm.template_captures.clear();
        // The page leaves with the result (`mem::take` below), so each render
        // starts from an empty buffer: one of the tree-walker's pooled page
        // buffers, of its size. A VM-compiled partial rendered per row used to
        // allocate 4096 bytes and free them again each time.
        if vm.template_out.capacity() == 0 {
            vm.template_out = crate::template::renderer::take_render_buffer(4096);
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
                // A small page leaves as a tight copy and its buffer goes back
                // to the pool, as the tree-walker's does.
                Ok(Rendered {
                    html: crate::template::renderer::finish_render_buffer(std::mem::take(
                        &mut vm.template_out,
                    )),
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

/// The controller's `@ivar` an include reads for `name` past its data, a
/// `grouped {}` placeholder forced as [`data_value`] forces one.
fn inherited_value(controller: &Rc<RefCell<Instance>>, name: &str) -> Option<Value> {
    let value = inherited_field(&controller.borrow(), name)?;
    Some(match value {
        Value::Deferred(_) => value.force_deferred(),
        other => other,
    })
}

/// Whether the data has a key `name`, whatever its value: the tree-walker
/// then reads that value, not a builtin of the same name.
fn data_binds(data: &Value, name: &str) -> bool {
    match data {
        Value::Hash(map) => hash_get_value(&map.borrow(), &Value::String(name.into())).is_some(),
        _ => false,
    }
}

/// Whether `value` is the builtin function `name` itself.
fn is_builtin(value: Option<&Value>, name: &str) -> bool {
    matches!(value, Some(Value::NativeFunction(native)) if native.name == name)
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
        body: body.to_vec().into(),
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

/// The statements a template's nodes stand for. `assigned` collects the names
/// a code block binds where the layout would see them: not inside a `for` or a
/// `form_with` block (each is a scope of its own on the tree-walker).
fn lower_nodes(
    nodes: &[TemplateNode],
    in_for: bool,
    assigned: &mut std::collections::BTreeSet<String>,
    all_assigned: &mut std::collections::BTreeSet<String>,
    saves: &mut u32,
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
                let then_branch = Box::new(block(lower_nodes(
                    body,
                    in_for,
                    assigned,
                    all_assigned,
                    saves,
                )?));
                let else_branch = match else_body {
                    Some(nodes) => Some(Box::new(block(lower_nodes(
                        nodes,
                        in_for,
                        assigned,
                        all_assigned,
                        saves,
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
                    body: Box::new(block(lower_nodes(
                        body,
                        true,
                        assigned,
                        all_assigned,
                        saves,
                    )?)),
                }));
            }
            // `<%- component "card", props do %> body <%- end %>`: the body is
            // captured as the `content` slot, then the component renders with
            // it, as the renderer does (`render_component_block`). A named slot
            // (`c.slot("x") do … end`) is a `content_for` in the body.
            TemplateNode::Component { parts, body, .. } => {
                out.push(call_stmt("__tpl_capture_start", vec![]));
                out.extend(lower_nodes(body, in_for, assigned, all_assigned, saves)?);
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
            // A compiled template is a view, partial or component, never a
            // layout: `yield` is its `content` local, `yield "x"` a slot.
            TemplateNode::Yield(None) => {
                let locals = Expr::new(ExprKind::Variable("locals".to_string()), Span::default());
                out.push(call_stmt("__tpl_yield", vec![locals]));
            }
            TemplateNode::Yield(Some(name)) => {
                let name = Expr::new(ExprKind::StringLiteral(name.clone()), Span::default());
                out.push(call_stmt("__tpl_yield_named", vec![name]));
            }
            TemplateNode::ContentFor { name, body, .. } => {
                out.push(call_stmt("__tpl_capture_start", vec![]));
                out.extend(lower_nodes(body, in_for, assigned, all_assigned, saves)?);
                let name = Expr::new(ExprKind::StringLiteral(name.clone()), Span::default());
                out.push(call_stmt(
                    "__tpl_content_for",
                    vec![name, call_expr("__tpl_capture_end", vec![])],
                ));
            }
            // The tree-walker pushes a scope, binds the builder there, writes
            // `open()` / `close()` raw, and pops. Locals here are the
            // function's, so the previous value of the builder's name is kept
            // and put back. The body is a child scope: a name it assigns is
            // not handed to the layout, as a `for` body's is not. The `let`
            // sits in its own block — at the function's top level a `let`
            // would be a global.
            TemplateNode::FormWith { parts, body, .. } => {
                let saved = format!("__tpl_form_scope_{}", *saves);
                *saves += 1;
                all_assigned.insert(parts.var.clone());
                let mut inner = Vec::new();
                inner.push(let_stmt(&saved, var_expr(&parts.var)));
                inner.push(assign_stmt(&parts.var, parts.builder_expr.clone()));
                inner.push(write_stmt(parts.open_expr.clone(), false));
                inner.extend(lower_nodes(body, true, assigned, all_assigned, saves)?);
                inner.push(write_stmt(parts.close_expr.clone(), false));
                inner.push(assign_stmt(&parts.var, var_expr(&saved)));
                out.push(block(inner));
            }
            // `<%= render "name" %>` / `<%= render "name", ctx %>`. The paren
            // form and `<%- render %>` are ordinary calls and already compile.
            // No context means the view's own data (`locals`), and the partial
            // is written raw: it escaped its own output.
            TemplateNode::Partial { name, context, .. } => {
                let name_expr = Expr::new(ExprKind::StringLiteral(name.clone()), Span::default());
                let data_expr = match context {
                    Some(expr) => expr.clone(),
                    None => var_expr("locals"),
                };
                out.push(call_stmt("__tpl_partial", vec![name_expr, data_expr]));
            }
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

crate::request_local! {
    /// The `TemplateCache` rendering the view on this thread's VM, for
    /// `Op::TemplateComponent` and `Op::TemplatePartial`: the include renders
    /// through it, as the tree-walker's partial renderer does. Set only for
    /// the render's length.
    static RENDERING_CACHE: std::cell::Cell<*const crate::template::TemplateCache> =
        const { std::cell::Cell::new(std::ptr::null()) };
}

/// Sets [`RENDERING_CACHE`] for a render and puts back, on drop, the value it
/// replaced: a partial rendered inside a view ends before the view does.
struct RenderingCache(*const crate::template::TemplateCache);

impl RenderingCache {
    fn set(cache: &crate::template::TemplateCache) -> Self {
        RenderingCache(RENDERING_CACHE.with(|c| c.replace(cache as *const _)))
    }
}

impl Drop for RenderingCache {
    fn drop(&mut self) {
        RENDERING_CACHE.with(|c| c.set(self.0));
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
    let name: &str = match name {
        Value::String(s) => s,
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
    let path: std::borrow::Cow<str> = if name.contains('/') || name.contains('.') {
        name.into()
    } else {
        format!("components/{name}").into()
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

/// `<%= render "name" %>` (`Op::TemplatePartial`), as the renderer's
/// `TemplateNode::Partial` arm does it: the name is resolved by
/// `render_partial` (the `_` prefix, `components/` left clean), and `data` is
/// whatever the tag's context evaluated to — the view's own data when the tag
/// gave none. Nothing is added to it, as `partial()` adds nothing: the helpers
/// resolve in the template env and the controller's `@ivars` in the include's
/// scope.
pub(crate) fn render_partial_block(name: &Value, data: &Value) -> Result<String, String> {
    // Borrowed: formatting it into a new `String` cost 2% of an include.
    let name: &str = match name {
        Value::String(s) => s,
        other => {
            return Err(format!(
                "partial name must evaluate to string, got {}",
                other.type_name()
            ))
        }
    };
    let rendering = RENDERING_CACHE.with(|c| c.get());
    if rendering.is_null() {
        return crate::interpreter::builtins::template::get_template_cache()?
            .render_partial(name, data);
    }
    // SAFETY: set by `render_view` from a `&TemplateCache` that outlives the
    // render, cleared when it ends (`RenderingCache`), and read only on this
    // thread, inside that render.
    unsafe { &*rendering }.render_partial(name, data)
}

/// Whether a template does enough work for the VM to pay: a loop, or at least
/// [`MIN_DYNAMIC_NODES`] dynamic nodes. A render on the VM has a fixed cost
/// that a template with two `yield`s never earns back: a page of 50 such
/// components took 8% more CPU with them compiled.
fn worth_compiling(nodes: &[TemplateNode]) -> bool {
    fn count(nodes: &[TemplateNode], dynamic: &mut usize) -> bool {
        for node in nodes {
            match node {
                TemplateNode::For { .. } => return true,
                TemplateNode::Literal(_) => {}
                TemplateNode::If {
                    body, else_body, ..
                } => {
                    *dynamic += 1;
                    if count(body, dynamic) {
                        return true;
                    }
                    if let Some(else_body) = else_body {
                        if count(else_body, dynamic) {
                            return true;
                        }
                    }
                }
                TemplateNode::ContentFor { body, .. }
                | TemplateNode::FormWith { body, .. }
                | TemplateNode::Component { body, .. } => {
                    *dynamic += 1;
                    if count(body, dynamic) {
                        return true;
                    }
                }
                _ => *dynamic += 1,
            }
        }
        false
    }
    let mut dynamic = 0;
    count(nodes, &mut dynamic) || dynamic >= MIN_DYNAMIC_NODES
}

/// See [`worth_compiling`]. The VM saves ~35 ns per expression over the
/// tree-walker and costs ~0.2 µs more per render under serve.
const MIN_DYNAMIC_NODES: usize = 8;

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

fn var_expr(name: &str) -> Expr {
    Expr::new(ExprKind::Variable(name.to_string()), Span::default())
}

fn let_stmt(name: &str, value: Expr) -> Stmt {
    stmt(StmtKind::Let {
        name: name.to_string(),
        type_annotation: None,
        initializer: Some(value),
    })
}

fn assign_stmt(name: &str, value: Expr) -> Stmt {
    stmt(StmtKind::Expression(Expr::new(
        ExprKind::Assign {
            target: Box::new(var_expr(name)),
            value: Box::new(value),
        },
        Span::default(),
    )))
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
    /// helpers). The VM's globals start as the worker's (`set_worker_globals`), so
    /// a model method a view calls resolves the classes it would on an action.
    /// `install_form_builder_globals` then adds the few template-only names a
    /// `FormBuilder` method calls (`_form_safe_url`, `attr`, …): those methods
    /// are JIT-compiled onto this VM, and the worker never held them. A name
    /// written in the view itself still resolves in the view's scope.
    /// Boxed: a `Vm` is large, and a render takes one out of the pool and puts
    /// it back — by value that was two copies of the whole VM per partial.
    #[allow(clippy::vec_box)] // the point: a pointer moves, not the VM
    static VIEW_VMS: RefCell<Vec<Box<ViewVm>>> = const { RefCell::new(Vec::new()) };
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
                crate::platform::env::var("SOLI_VM_VIEWS").as_deref(),
                Ok("0") | Ok("off") | Ok("false")
            )
        })
}

/// How [`render_view`] rendered a view.
pub enum ViewRender {
    /// On the VM.
    Rendered(Rendered),
    /// Not on the VM: the caller renders the view on the tree-walker. With a
    /// [`VmFailure`] when the VM tried and failed, which the caller settles
    /// once the tree-walker has rendered the view.
    TreeWalker(Option<VmFailure>),
}

/// A render the VM failed with an error other than a refusal. Nothing it
/// wrote is sent: the caller renders the view again on the tree-walker. When
/// that render succeeds, the two engines disagree about the view, and
/// [`VmFailure::tree_walker_rendered`] keeps it on the tree-walker from then
/// on (on this thread) instead of failing on the VM on every request.
/// Dropped without it — the tree-walker raised too — the error is the view's
/// or its data's, and the view stays compiled.
pub struct VmFailure {
    /// The view's nodes: their address keys `COMPILED`, and holding them
    /// keeps it from being reused.
    nodes: Arc<Vec<TemplateNode>>,
    error: RuntimeError,
    /// The view's name for `SOLI_ENGINE_LOG`, kept only when it is on.
    name: Option<String>,
}

impl VmFailure {
    /// The tree-walker rendered the view the VM failed.
    pub fn tree_walker_rendered(self) {
        keep_on_tree_walker(&self.nodes);
        if let Some(name) = self.name {
            let error = self.error;
            engine_log(|| {
                format!(
                    "view '{name}' failed on the VM and rendered on the interpreter, \
                     which renders it from now on: {error}"
                )
            });
        }
    }
}

/// `nodes` render on the tree-walker from now on, on this thread.
fn keep_on_tree_walker(nodes: &Arc<Vec<TemplateNode>>) {
    let key = Arc::as_ptr(nodes) as usize;
    COMPILED.with(|compiled| {
        if let Ok(mut compiled) = compiled.try_borrow_mut() {
            if let Some(entry) = compiled.get_mut(&key) {
                entry.1 = None;
            }
        }
    });
}

/// A template's name in `SOLI_ENGINE_LOG` lines: its path under the views
/// directory, or the whole path when it lives elsewhere (an engine's views).
fn view_name(path: &std::path::Path, cache: &crate::template::TemplateCache) -> String {
    path.strip_prefix(cache.views_dir())
        .unwrap_or(path)
        .display()
        .to_string()
}

/// `SOLI_ENGINE_LOG=1`: say on stderr which templates render on the
/// interpreter, and why.
fn engine_log_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        crate::platform::env::var("SOLI_ENGINE_LOG")
            .is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
    })
}

/// Print `line` under `SOLI_ENGINE_LOG`, once per process: every worker
/// thread decides about a template on its own, and they decide alike.
pub(crate) fn engine_log(line: impl FnOnce() -> String) {
    if !engine_log_enabled() {
        return;
    }
    static PRINTED: std::sync::OnceLock<std::sync::Mutex<HashSet<String>>> =
        std::sync::OnceLock::new();
    let line = line();
    let first = PRINTED
        .get_or_init(Default::default)
        .lock()
        .map(|mut printed| printed.insert(line.clone()))
        .unwrap_or(true);
    if first {
        eprintln!("[soli engine] {line}");
    }
}

/// `SOLI_ENGINE_LOG=1`: a template the VM never tries — a layout, or a
/// partial rendered outside a view — says once that the tree-walker renders
/// it, and `why`, as a view that does not compile does. `kind` is how the
/// line names it ("layout", "view").
pub fn note_interpreted(
    kind: &str,
    path: &std::path::Path,
    cache: &crate::template::TemplateCache,
    why: &str,
) {
    if engine_log_enabled() && enabled() {
        engine_log(|| {
            format!(
                "{kind} '{}' renders on the interpreter: {why}",
                view_name(path, cache)
            )
        });
    }
}

/// Render a view on this thread's VM, or say that the tree-walker must:
/// views off, a view that does not compile, a render whose bare `clock` the
/// data or a helper rebinds, a view the VM refused or failed before, or a
/// render the VM refused or failed now. A failed render wrote nothing; the
/// caller renders the view again on the tree-walker, which raises the error
/// with its file and line — and when it raises none, hands back the
/// [`VmFailure`]. `inherited` is a partial's or component's controller, whose
/// `@ivars` a name its data lacks reads, as `env` does on the tree-walker.
pub fn render_view(
    nodes: &Arc<Vec<TemplateNode>>,
    data: &Value,
    inherited: Option<&Rc<RefCell<Instance>>>,
    env: &Rc<RefCell<crate::interpreter::environment::Environment>>,
    cache: &crate::template::TemplateCache,
    path: &std::path::Path,
) -> ViewRender {
    // A render does not suspend its request (`serve::coroutine`).
    #[cfg(not(target_arch = "wasm32"))]
    let _no_yield = crate::serve::coroutine::NoYield::enter();
    if !enabled() {
        check_log("[vm-views] off");
        return ViewRender::TreeWalker(None);
    }
    let key = Arc::as_ptr(nodes) as usize;
    let compiled = COMPILED.with(|compiled_views| {
        let mut compiled_views = compiled_views.borrow_mut();
        if let Some((_, compiled)) = compiled_views.get(&key) {
            return compiled.clone();
        }
        let compiled = match VmTemplate::compile_for_serve(nodes) {
            Ok(template) => Some(Rc::new(template)),
            Err(reason) => {
                engine_log(|| {
                    format!(
                        "view '{}' renders on the interpreter: {reason}",
                        view_name(path, cache)
                    )
                });
                None
            }
        };
        compiled_views.insert(key, (nodes.clone(), compiled.clone()));
        compiled
    });
    let Some(compiled) = compiled else {
        return ViewRender::TreeWalker(None);
    };
    // A VM from this thread's pool, held by this render alone: a component
    // or partial rendered from inside it takes another.
    let mut view_vm = ViewVm::take();
    // About this render's data (`@clock`) or the app's helpers, not about the
    // view: the tree-walker renders this once, and the view stays compiled.
    if let Some(name) =
        compiled.bare_call_unresolved(data, inherited, &view_vm.names, &view_vm.vm.globals)
    {
        view_vm.put_back();
        engine_log(|| {
            format!(
                "view '{}' renders on the interpreter while `{name}` is not the builtin \
                 (the data, a helper or the app binds it)",
                view_name(path, cache)
            )
        });
        return ViewRender::TreeWalker(None);
    }
    let args = compiled.bind(data, inherited, &view_vm.names);
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
        Ok(rendered) => ViewRender::Rendered(rendered),
        Err(err) => {
            if check_enabled() {
                check_log(&format!(
                    "[vm-views] {}: render failed, tree-walker used: {err}",
                    path.display()
                ));
            }
            // A refusal is about the view (a function written bare), so it
            // keeps the tree-walker from now on.
            if err.is_engine_fallback() {
                keep_on_tree_walker(nodes);
                engine_log(|| {
                    format!(
                        "view '{}' renders on the interpreter from now on: {err}",
                        view_name(path, cache)
                    )
                });
                return ViewRender::TreeWalker(None);
            }
            // Any other error may be about this request's data. The
            // tree-walker's render of the same view and data tells.
            ViewRender::TreeWalker(Some(VmFailure {
                nodes: nodes.clone(),
                error: err,
                name: engine_log_enabled().then(|| view_name(path, cache)),
            }))
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
        crate::platform::env::var("SOLI_VM_VIEWS_CHECK")
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

/// Names a JIT-compiled `FormBuilder` method calls that the worker's globals
/// do not have. They live in the template env (`template_env_bindings`); the
/// method runs on this VM and looks globals up here. A name the worker already
/// defined is left alone, unless the builder defines it itself.
const FORM_BUILDER_GLOBALS: &[&str] = &[
    "_form_safe_url",
    "_safe_attr_name",
    "FormBuilder",
    "csrf_field",
    "attr",
    "h",
];

/// A name the form builder defines itself is its own definition: on the
/// tree-walker its methods close over the builder's environment, so a view
/// helper of the same name (`csrf_field`) is not what `f.open()` calls, and
/// it must not be on the VM either. The template env holds the app's helpers
/// last, so it would hand over the helper. It replaces what `globals` already
/// holds: a view VM with no worker globals (a background job's mailer view)
/// starts from the template env's names, the helper among them, and a worker
/// whose app defines the name holds the app's.
fn install_form_builder_globals(
    globals: &mut ahash::AHashMap<String, Value>,
    template_names: &ahash::AHashMap<String, Value>,
) {
    let builder =
        crate::interpreter::builtins::template::form_builder_definitions().unwrap_or_default();
    for name in FORM_BUILDER_GLOBALS {
        let own = builder
            .iter()
            .find(|(defined, _)| defined == name)
            .map(|(_, value)| value);
        if let Some(value) = own {
            globals.insert((*name).to_string(), value.clone());
        } else if let Some(value) = template_names.get(*name) {
            globals
                .entry((*name).to_string())
                .or_insert_with(|| value.clone());
        }
    }
}

impl ViewVm {
    /// A VM from the pool, or a new one.
    fn take() -> Box<Self> {
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
        // As the tree-walker does: a function the view calls (a view helper
        // compiled here, a model method) that returns a value its declared
        // return type does not admit raises instead of handing it back.
        vm.check_return_types = true;
        vm.globals = WORKER_GLOBALS
            .with(|g| g.borrow().clone())
            .unwrap_or_else(|| names.clone());
        // The worker's table has no form builder. Without these, `f.open()`
        // dies with "Undefined variable '_form_safe_url'" and every later
        // request pays a failed VM render before the tree-walker.
        install_form_builder_globals(&mut vm.globals, &names);
        Box::new(ViewVm {
            vm,
            names,
            generation,
        })
    }

    /// Back into the pool, unless the pool was emptied meanwhile.
    fn put_back(self: Box<Self>) {
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
            // A string read from the data, appended to: the tree-walker's
            // in-place append panicked on its own borrow here.
            "<% title = title + \"!\" %><%= title %>|<% title += \"?\" %><%= title %>",
            "<% title += \"?\" %><%= title %>",
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

    /// A render takes its page buffer from the tree-walker's pool and gives
    /// it back when the page is small, handing out a tight copy: a partial
    /// rendered per row no longer allocates and frees 4096 bytes each time.
    /// A page-sized result leaves as the buffer itself, not copied.
    #[test]
    fn a_render_borrows_its_page_buffer_from_the_pool() -> Result<(), String> {
        use crate::template::renderer::{pooled_render_buffers, take_render_buffer};
        let nodes = parse_template("<p><%= title %></p>")?;
        let compiled = VmTemplate::compile(&nodes)?;
        let mut vm = Vm::new();
        let data = hash(vec![("title", Value::String("Posts".into()))]);
        // One buffer in the pool for the render to take.
        let seeded = take_render_buffer(4096);
        crate::template::renderer::recycle_include_page(seeded);
        let before = pooled_render_buffers();
        assert!(before > 0);
        for _ in 0..3 {
            let html = compiled
                .render(&mut vm, &data)
                .map_err(|e| e.to_string())?
                .html;
            assert_eq!(html, "<p>Posts</p>");
            assert!(
                html.capacity() < 1024,
                "a small page kept the 4096-byte buffer (capacity {})",
                html.capacity()
            );
            assert_eq!(pooled_render_buffers(), before, "the buffer went back");
        }

        let big = parse_template(&format!("<%= title %>{}", "x".repeat(1100)))?;
        let html = VmTemplate::compile(&big)?
            .render(&mut vm, &data)
            .map_err(|e| e.to_string())?
            .html;
        assert_eq!(html.len(), "Posts".len() + 1100);
        assert!(
            html.capacity() >= 4096,
            "a page-sized result was copied out"
        );
        assert_eq!(pooled_render_buffers(), before - 1);
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

    /// A write is a statement of its own: no Null pushed for an `Op::Pop` to
    /// discard after it, in the view's body and in a loop's alike.
    #[test]
    fn a_write_pushes_nothing_for_a_pop_to_discard() -> Result<(), String> {
        use crate::vm::opcode::Op;
        let nodes = parse_template(
            "<p><%= title %></p><% for row in rows %><li><%- row.title %></li><% end %>",
        )?;
        let compiled = VmTemplate::compile(&nodes)?;
        let Value::VmClosure(closure) = &compiled.closure else {
            return Err("not a VM closure".to_string());
        };
        let code = &closure.proto.chunk.code;
        let is_write = |op: &Op| matches!(op, Op::TemplateWrite(_) | Op::TemplateWriteConst(_));
        assert_eq!(code.iter().filter(|op| is_write(op)).count(), 6, "{code:?}");
        // The one Pop left is the loop's: `row` going out of scope at the end
        // of each pass. Each of the six writes had its own.
        assert_eq!(
            code.iter().filter(|op| matches!(op, Op::Pop)).count(),
            1,
            "{code:?}"
        );
        assert!(
            code.windows(2)
                .all(|pair| !(is_write(&pair[0]) && matches!(pair[1], Op::Null))),
            "{code:?}"
        );
        assert_renders_same(
            "<p><%= title %></p><% for row in rows %><li><%- row.title %></li><% end %>",
            &data(),
        );
        Ok(())
    }

    /// The intrinsic used as a value is nil, as a call that returns nothing,
    /// and the write still happens: the compiler pushes the Null the op no
    /// longer does.
    #[test]
    fn a_write_used_as_a_value_is_nil() -> Result<(), String> {
        let nodes = parse_template(
            "<% r = __tpl_write_raw(\"a\") %><% s = __tpl_write(\"<\") %><% t = __tpl_write_raw(count) %>\
             [<%= r.nil? %>,<%= s.nil? %>,<%= t.nil? %>]",
        )?;
        let compiled = VmTemplate::compile(&nodes)?;
        let html = compiled
            .render(&mut view_vm(), &data())
            .map_err(|e| e.to_string())?
            .html;
        assert_eq!(html, "a&lt;3[true,true,true]");
        Ok(())
    }

    /// A VM as serve builds one for a view: the form builder is Soli, so its
    /// functions run on the interpreter, and the template builtins are the
    /// globals a free name resolves in.
    fn view_vm() -> Vm {
        let mut vm = Vm::new();
        vm.tree_walk_functions = true;
        vm.check_return_types = true;
        vm.globals = crate::template::core_eval::template_env_bindings()
            .into_iter()
            .collect();
        vm
    }

    /// Serve copies the *worker* globals onto the view VM. Those never held
    /// the form builder, and `f.open()` looks `_form_safe_url` up there.
    #[test]
    fn a_form_renders_when_the_worker_globals_omit_the_builder(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let template_names: ahash::AHashMap<String, Value> =
            crate::template::core_eval::template_env_bindings()
                .into_iter()
                .collect();
        let mut worker = template_names.clone();
        for name in super::FORM_BUILDER_GLOBALS {
            worker.remove(*name);
        }
        // A name the worker already has stays the worker's.
        worker.insert("h".to_string(), Value::Int(9));
        let mut seeded = worker.clone();
        super::install_form_builder_globals(&mut seeded, &template_names);
        assert_eq!(seeded.get("h"), Some(&Value::Int(9)));
        assert!(seeded.contains_key("_form_safe_url"));
        assert!(seeded.contains_key("FormBuilder"));

        worker.remove("h");
        worker.insert("from_the_worker".to_string(), Value::Int(3));
        super::set_worker_globals(&worker);
        let mut view = super::ViewVm::take();
        assert_eq!(view.vm.globals.get("from_the_worker"), Some(&Value::Int(3)));
        assert!(view.vm.globals.contains_key("attr"));
        assert!(view.vm.globals.contains_key("csrf_field"));

        let source = "<%- form_with(post, {\"url\": \"/posts\"}) do |f| -%>\n<%- f.label(\"title\") -%>\n<%- f.text_field(\"title\") -%>\n<%- f.fields_for(\"author\") do |af| -%>\n<%- af.text_field(\"name\") -%>\n<%- end -%>\n<%- end -%>";
        let post = hash(vec![
            ("title", Value::String("Hi <there>".into())),
            ("author", hash(vec![("name", Value::String("Ada".into()))])),
        ]);
        let data = hash(vec![("post", post)]);
        let _lenient = crate::interpreter::executor::enter_template_lenient_vars();
        let nodes = parse_template(source)?;
        // Each render as its own request: the form's honeypot ids restart.
        crate::interpreter::builtins::spam::reset_honeypot_ids();
        let tree = render_nodes(&nodes, &data, None)?;
        let compiled = VmTemplate::compile(&nodes)?;
        crate::interpreter::builtins::spam::reset_honeypot_ids();
        let on_vm = compiled
            .render(&mut view.vm, &data)
            .unwrap_or_else(|e| panic!("{e}"))
            .html;
        assert_eq!(on_vm, tree);

        drop(view);
        super::WORKER_GLOBALS.with(|slot| *slot.borrow_mut() = None);
        super::empty_pool();
        Ok(())
    }

    /// A view helper named like a function the form builder defines
    /// (`csrf_field`) is not what a JIT-compiled `FormBuilder` method calls:
    /// on the tree-walker the builder's methods close over its own
    /// environment. The template env holds the app's helpers last, so it
    /// would have handed over the helper.
    #[test]
    fn the_form_builder_calls_its_own_functions_on_the_vm() -> Result<(), String> {
        let mut template_names: ahash::AHashMap<String, Value> =
            crate::template::core_eval::template_env_bindings()
                .into_iter()
                .collect();
        template_names.insert("csrf_field".to_string(), Value::Int(1));
        // Empty, as a worker's table without the builder, and already holding
        // the helper, as a view VM with no worker globals starts.
        let mut globals = ahash::AHashMap::new();
        super::install_form_builder_globals(&mut globals, &template_names);
        let mut seeded = template_names.clone();
        super::install_form_builder_globals(&mut seeded, &template_names);

        let builder = crate::interpreter::builtins::template::form_builder_definitions()?;
        let own = builder.iter().find_map(|(name, value)| match value {
            Value::Function(func) if name == "csrf_field" => Some(func.clone()),
            _ => None,
        });
        for table in [&globals, &seeded] {
            match (table.get("csrf_field"), &own) {
                (Some(Value::Function(installed)), Some(own)) => {
                    assert!(Rc::ptr_eq(installed, own), "not the builder's csrf_field")
                }
                (installed, _) => return Err(format!("csrf_field installed as {installed:?}")),
            }
        }
        // `attr` the builder does not define: the template env's.
        assert!(globals.contains_key("attr"));
        Ok(())
    }

    /// A controller whose `@ivars` are `fields`.
    fn controller(fields: Vec<(&str, Value)>) -> Rc<RefCell<Instance>> {
        let class = Rc::new(crate::interpreter::value::Class {
            name: "PostsController".to_string(),
            ..Default::default()
        });
        let mut instance = Instance::new(class);
        for (name, value) in fields {
            instance.fields.insert(name.into(), value);
        }
        Rc::new(RefCell::new(instance))
    }

    /// `source` on the tree-walker as an include with `inherited`'s `@ivars`
    /// renders it, and on the VM.
    fn render_include_on_both(
        source: &str,
        data: &Value,
        inherited: &Rc<RefCell<Instance>>,
    ) -> Result<(String, Result<Rendered, RuntimeError>), Box<dyn std::error::Error>> {
        let nodes = parse_template(source)?;
        let mut interpreter =
            crate::template::core_eval::template_interpreter(data, Some(inherited.clone()));
        let tree = crate::template::renderer::render_with_interpreter(
            &mut interpreter,
            &nodes,
            data,
            None,
            None,
        )?;
        drop(interpreter);
        let on_vm =
            VmTemplate::compile(&nodes)?.render_inheriting(&mut view_vm(), data, Some(inherited));
        Ok((tree, on_vm))
    }

    /// A partial's free names bind the controller's `@ivars` past its data
    /// and before the template env, the order the tree-walker's scope reads
    /// them in. They used to be copied into the partial's data.
    #[test]
    fn an_include_binds_the_controller_ivars_as_the_tree_walker_reads_them(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let inherited = controller(vec![
            ("title", Value::String("Posts".into())),
            ("post", Value::String("from @post".into())),
            ("params", Value::String("the request's".into())),
            ("count", Value::Int(2)),
        ]);
        let data = hash(vec![("post", Value::String("passed".into()))]);
        let cases = [
            (
                "<%= post %>|<%= @title %>|<%= title.upcase %>|<%= params.nil? %>|<%= absent.nil? %>",
                "passed|Posts|POSTS|true|true",
            ),
            (
                "<% for i in [1, 2] %><%= i %>:<%= @title %>;<% end %>",
                "1:Posts;2:Posts;",
            ),
            (
                "<% title = title + \"!\" %><%= title %>|<%= count + 1 %>",
                "Posts!|3",
            ),
            (
                "<%= [1].map { |x| title + post }.join(\",\") %>",
                "Postspassed",
            ),
            ("<%= h(@title) %>|<%= locals.length %>", "Posts|1"),
        ];
        let _lenient = crate::interpreter::executor::enter_template_lenient_vars();
        for (source, expected) in cases {
            let (tree, on_vm) = render_include_on_both(source, &data, &inherited)?;
            assert_eq!(tree, expected, "{source}");
            assert_eq!(on_vm?.html, tree, "{source}");
        }
        Ok(())
    }

    /// A controller's `@clock` is what an include's bare `clock` reads on the
    /// tree-walker: the VM refuses that render rather than call the builtin.
    #[test]
    fn an_inherited_clock_is_left_to_the_tree_walker() -> Result<(), Box<dyn std::error::Error>> {
        let inherited = controller(vec![("clock", Value::Int(5))]);
        let _lenient = crate::interpreter::executor::enter_template_lenient_vars();
        let (tree, on_vm) = render_include_on_both("<%= clock + 1 %>", &hash(vec![]), &inherited)?;
        assert_eq!(tree, "6");
        match on_vm {
            Err(err) => assert!(err.is_engine_fallback(), "{err}"),
            Ok(rendered) => return Err(format!("the builtin was called: {}", rendered.html).into()),
        }
        Ok(())
    }

    fn assert_renders_same(source: &str, data: &Value) {
        let _lenient = crate::interpreter::executor::enter_template_lenient_vars();
        let nodes = parse_template(source).unwrap_or_else(|e| panic!("{source}: {e}"));
        // Each render as its own request: a form's honeypot ids restart.
        crate::interpreter::builtins::spam::reset_honeypot_ids();
        let tree =
            render_nodes(&nodes, data, None).unwrap_or_else(|e| panic!("{source}: tree: {e}"));
        let compiled =
            VmTemplate::compile(&nodes).unwrap_or_else(|e| panic!("{source}: compile: {e}"));
        let mut vm = view_vm();
        crate::interpreter::builtins::spam::reset_honeypot_ids();
        let on_vm = compiled
            .render(&mut vm, data)
            .unwrap_or_else(|e| panic!("{source}: vm: {e}"))
            .html;
        assert_eq!(on_vm, tree, "{source}");
        assert!(
            vm.globals
                .keys()
                .all(|name| !name.starts_with("__tpl_form_scope_")),
            "a form scope leaked into the VM's globals"
        );
    }

    /// `form_with` / `fields_for` blocks, including the builder name put back
    /// when the block ends.
    #[test]
    fn a_form_block_renders_what_the_tree_walker_renders() {
        let post = hash(vec![
            ("title", Value::String("Hi <there>".into())),
            ("author", hash(vec![("name", Value::String("Ada".into()))])),
        ]);
        let items = array(vec![Value::String("a".into()), Value::String("b".into())]);
        let cases = [
            (
                "<%- form_with(post, {\"url\": \"/posts\", \"method\": \"get\"}) do -%>\n<%- f.text_field(\"title\") -%>\n<%- end -%>\n",
                hash(vec![("post", post.clone())]),
            ),
            (
                "<%- form_with(null, {\"url\": \"/search\", \"method\": \"get\"}) do |form| -%>\n<%- form.text_field(\"q\") -%>\n<%- end -%>\n[<%= form %>]",
                hash(vec![]),
            ),
            (
                "before[<%= kept %>]<%- form_with(null, {\"url\": \"/search\", \"method\": \"get\"}) do |kept| -%><%= kept.class %><%- end -%>after[<%= kept %>]",
                hash(vec![("kept", Value::String("hello".into()))]),
            ),
            (
                "<%- form_with(post, {\"url\": \"/x\"}) do |f| -%>\n<%- f.fields_for(\"author\") do |af| -%>\n<%- af.text_field(\"name\") -%>\n<%- end -%>\n<%- end -%>",
                hash(vec![("post", post.clone())]),
            ),
            (
                "<%- form_with(null, {\"url\": \"/x\", \"method\": \"get\"}) do |f| -%>[<%= f.url %>]<%- f.fields_for(\"author\") do |f| -%>[<%= f.url %>]<%- end -%>[<%= f.url %>]<%- end -%>",
                hash(vec![]),
            ),
            (
                "<% for item in items %><% form_with(null, {\"url\": \"/x\", \"method\": \"get\"}) do %><%= item %><% end %><% end %>",
                hash(vec![("items", items)]),
            ),
            (
                "<% form_with(null, {\"url\": \"/search\", \"method\": \"get\"}) do %><%= f.class %><% end %><% form_with(null, {\"url\": \"/search\", \"method\": \"get\"}) do %><%= f.class %><% end %>[<%= f %>]",
                hash(vec![]),
            ),
        ];
        for (source, data) in cases {
            assert_renders_same(source, &data);
        }
    }

    /// A name the form's body assigns stays out of what the layout sees. The
    /// builder's name does too: the block puts back whatever it shadowed.
    #[test]
    fn a_form_block_does_not_hand_its_names_to_the_layout() -> Result<(), Box<dyn std::error::Error>>
    {
        let source = "<% heading = \"Posts\" %>\
             <% form_with(null, {\"url\": \"/search\", \"method\": \"get\"}) do %>\
             <% inner = 1 %>\
             <% end %>\
             <%= heading %>";
        let _lenient = crate::interpreter::executor::enter_template_lenient_vars();
        let nodes = parse_template(source)?;
        let data = hash(vec![]);
        let tree = render_nodes(&nodes, &data, None)?;
        let compiled = VmTemplate::compile(&nodes)?;
        let rendered = compiled.render(&mut view_vm(), &data)?;
        let names: Vec<&str> = rendered.assigned.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, ["heading"]);
        assert_eq!(rendered.html, tree);
        Ok(())
    }

    /// A loop whose body is a form, or a `<%= render %>` tag, is worth
    /// compiling. A form on its own is not: the size threshold is unchanged.
    #[test]
    fn a_loop_holding_a_form_or_a_render_tag_compiles_for_serve(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let form = parse_template(
            "<% for item in items %><% form_with(null, {\"url\": \"/x\", \"method\": \"get\"}) do %><%= item %><% end %><% end %>",
        )
        ?;
        assert!(VmTemplate::compile_for_serve(&form).is_ok());

        let partial =
            parse_template("<% for row in rows %><%= render 'things/card', row %><% end %>")?;
        assert!(VmTemplate::compile_for_serve(&partial).is_ok());

        let tiny = parse_template(
            "<% form_with(null, {\"url\": \"/x\", \"method\": \"get\"}) do %><%= item %><% end %>",
        )?;
        assert!(VmTemplate::compile_for_serve(&tiny).is_err());
        Ok(())
    }

    /// `<%= render 'name' %>` passes the view's data; a context replaces it.
    /// The partial is written raw, the same text the tree-walker writes.
    #[test]
    fn a_render_tag_renders_what_the_tree_walker_renders() -> Result<(), Box<dyn std::error::Error>>
    {
        let dir = tempfile::tempdir()?;
        let views = dir.path().join("views");
        std::fs::create_dir_all(views.join("things"))?;
        std::fs::write(views.join("things").join("_card.html.slv"), "[<%= name %>]")?;
        let cache = crate::template::TemplateCache::new(&views);
        let _lenient = crate::interpreter::executor::enter_template_lenient_vars();
        let cases = [
            (
                "Hello <%= render 'things/card' %>!",
                hash(vec![("name", Value::String("Bea".into()))]),
            ),
            (
                "<%= render 'things/card', {\"name\": \"Bea\"} %>",
                hash(vec![]),
            ),
            (
                "<% who = {\"name\": \"Bea\"} %><%= render 'things/card', who %>",
                hash(vec![]),
            ),
            ("x<%= render 'things/card', missing %>y", hash(vec![])),
        ];
        for (source, data) in cases {
            let nodes = parse_template(source).unwrap_or_else(|e| panic!("{source}: {e}"));
            let render_partial = |name: &str, ctx: &Value| cache.render_partial(name, ctx);
            let tree = render_nodes(&nodes, &data, Some(&render_partial))
                .unwrap_or_else(|e| panic!("{source}: tree: {e}"));
            let compiled =
                VmTemplate::compile(&nodes).unwrap_or_else(|e| panic!("{source}: compile: {e}"));
            let _guard = RenderingCache::set(&cache);
            let on_vm = compiled
                .render(&mut view_vm(), &data)
                .unwrap_or_else(|e| panic!("{source}: vm: {e}"))
                .html;
            assert_eq!(on_vm, tree, "{source}");
        }
        Ok(())
    }

    /// A bare `clock` is the time in a compiled view, as on the tree-walker
    /// and in an action. It used to be a parameter holding the function: its
    /// value was the function, `clock - started` raised, and serve rendered
    /// every such page twice, the second time on the tree-walker.
    #[test]
    fn a_bare_clock_is_the_time_in_a_compiled_view() -> Result<(), Box<dyn std::error::Error>> {
        let data = hash(vec![("started", Value::Float(1.0))]);
        let cases = [
            "<%= (clock - started) > 0 %>",
            "<% start = clock %><%= ((clock - start) * 1000).round(2) >= 0 %>",
            "<%= type(clock) %>|<%= type(clock()) %>|<%= clock.round > 0 %>",
            "<%= \"#{clock > started}\" %>",
            "<%= [1, 2].map { |i| clock > started }.join(\",\") %>",
            "<% for i in [1] %><%= (clock + i) > started %><% end %>",
            // A `clock` the view binds itself is that binding, on both.
            "<%= [3].map { |clock| clock + 1 }.join(\",\") %>",
            "<% for clock in [7] %><%= clock %><% end %>",
        ];
        for source in cases {
            assert_renders_same(source, &data);
        }
        // Read as the global the bytecode calls, not bound as a parameter.
        let nodes = parse_template("<%= clock - started %>")?;
        assert_eq!(VmTemplate::compile(&nodes)?.params(), ["started"]);
        // One the view assigns stays a parameter, holding what it assigned.
        let nodes = parse_template("<% clock = 5 %><%= clock + 1 %>")?;
        let compiled = VmTemplate::compile(&nodes)?;
        assert_eq!(compiled.params(), ["clock"]);
        assert_eq!(compiled.render(&mut view_vm(), &data)?.html, "6");
        Ok(())
    }

    /// The tree-walker reads a `clock` the data binds (a controller's
    /// `@clock`, even nil), or a helper or an app function named `clock`,
    /// where the bytecode would call the builtin: the VM refuses that render,
    /// rather than write the time in its place.
    #[test]
    fn a_clock_the_data_or_the_app_binds_is_left_to_the_tree_walker(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let _lenient = crate::interpreter::executor::enter_template_lenient_vars();
        let nodes = parse_template("<%= clock + 1 %>")?;
        let compiled = VmTemplate::compile(&nodes)?;
        let five = hash(vec![("clock", Value::Int(5))]);
        assert_eq!(render_nodes(&nodes, &five, None)?, "6");
        for data in [five, hash(vec![("clock", Value::Null)])] {
            let Err(err) = compiled.render(&mut view_vm(), &data) else {
                return Err("a data-bound `clock` was called as the builtin".into());
            };
            assert!(err.is_engine_fallback(), "{err}");
        }
        let mut vm = view_vm();
        vm.globals.insert("clock".to_string(), Value::Int(1));
        let Err(err) = compiled.render(&mut vm, &hash(vec![])) else {
            return Err("an app's own `clock` was called as the builtin".into());
        };
        assert!(err.is_engine_fallback(), "{err}");
        Ok(())
    }

    /// How many times a probe ran on one engine.
    type Calls = Rc<std::cell::Cell<usize>>;

    /// `probe()` for a view, with the calls it got on the VM and on the
    /// tree-walker: the VM's render sets `RENDERING_CACHE`, the tree-walker's
    /// does not. It fails on the engines it is told to.
    fn counting_probe(fails_on_vm: bool, fails_on_tree_walker: bool) -> (Value, Calls, Calls) {
        let on_vm = Calls::default();
        let on_tree_walker = Calls::default();
        let (vm_calls, tree_walker_calls) = (on_vm.clone(), on_tree_walker.clone());
        let native = crate::interpreter::value::NativeFunction::new("probe", Some(0), move |_| {
            let vm = RENDERING_CACHE.with(|c| !c.get().is_null());
            let calls = if vm { &vm_calls } else { &tree_walker_calls };
            calls.set(calls.get() + 1);
            if (vm && fails_on_vm) || (!vm && fails_on_tree_walker) {
                return Err("probe failed".to_string());
            }
            Ok(Value::String("p".into()))
        });
        (Value::NativeFunction(native), on_vm, on_tree_walker)
    }

    /// Views at `views/page` and `views/things/_row` that call `probe()`
    /// twice, in a loop (worth compiling), then write `n`.
    fn probe_views() -> Result<(tempfile::TempDir, crate::template::TemplateCache), std::io::Error>
    {
        let dir = tempfile::tempdir()?;
        let views = dir.path().join("views");
        std::fs::create_dir_all(views.join("things"))?;
        let source = "<% for i in [1, 2] %><%= probe() %><% end %>|<%= n %>";
        std::fs::write(views.join("page.html.slv"), source)?;
        std::fs::write(views.join("things").join("_row.html.slv"), source)?;
        let cache = crate::template::TemplateCache::new(&views);
        Ok((dir, cache))
    }

    /// A view the VM fails on and the tree-walker renders: the two engines
    /// disagree about it, so it stays on the tree-walker. Every request used
    /// to run it on the VM, throw that away and render it again. A partial
    /// rendered inside a view goes the same way.
    #[test]
    fn a_view_only_the_vm_fails_stays_on_the_tree_walker() -> Result<(), Box<dyn std::error::Error>>
    {
        let (_dir, cache) = probe_views()?;
        let (probe, on_vm, on_tree_walker) = counting_probe(true, false);
        // A different `n` each time: the response cache never answers.
        for n in 1..=3 {
            let data = hash(vec![("probe", probe.clone()), ("n", Value::Int(n))]);
            assert_eq!(cache.render("page", &data, None)?, format!("pp|{n}"));
        }
        assert_eq!((on_vm.get(), on_tree_walker.get()), (1, 6));

        let _lenient = crate::interpreter::executor::enter_template_lenient_vars();
        let (probe, on_vm, on_tree_walker) = counting_probe(true, false);
        for n in 1..=3 {
            let data = hash(vec![("probe", probe.clone()), ("n", Value::Int(n))]);
            assert_eq!(
                cache.render_partial("things/row", &data)?,
                format!("pp|{n}")
            );
        }
        assert_eq!((on_vm.get(), on_tree_walker.get()), (1, 6));
        Ok(())
    }

    /// An error both engines raise is the view's or its data's, not the VM's:
    /// the view stays compiled, and the next request tries the VM again.
    #[test]
    fn an_error_both_engines_raise_keeps_the_view_compiled(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, cache) = probe_views()?;
        let (probe, on_vm, on_tree_walker) = counting_probe(true, true);
        for n in 1..=2 {
            let data = hash(vec![("probe", probe.clone()), ("n", Value::Int(n))]);
            assert!(cache.render("page", &data, None).is_err());
        }
        assert_eq!((on_vm.get(), on_tree_walker.get()), (2, 2));
        Ok(())
    }

    /// Through serve's render path: a page whose footer reads `clock` bare
    /// is rendered by the VM alone.
    #[test]
    fn a_page_reading_clock_bare_renders_on_the_vm_alone() -> Result<(), Box<dyn std::error::Error>>
    {
        let dir = tempfile::tempdir()?;
        let views = dir.path().join("views");
        std::fs::create_dir_all(&views)?;
        std::fs::write(
            views.join("page.html.slv"),
            "<% for i in [1, 2] %><%= probe() %><% end %>|<%= (clock - started) > 0 %>",
        )?;
        let cache = crate::template::TemplateCache::new(&views);
        let (probe, on_vm, on_tree_walker) = counting_probe(false, false);
        let data = hash(vec![("probe", probe), ("started", Value::Float(1.0))]);
        assert_eq!(cache.render("page", &data, None)?, "pp|true");
        assert_eq!((on_vm.get(), on_tree_walker.get()), (2, 0));
        Ok(())
    }

    /// `SOLI_VM_TEMPLATE_DIR=<app>/app/views cargo test … -- --ignored
    /// --nocapture`: how many of an app's views compile, and why the rest do
    /// not.
    #[test]
    #[ignore]
    fn coverage_report() {
        let Ok(dir) = crate::platform::env::var("SOLI_VM_TEMPLATE_DIR") else {
            return;
        };
        let mut reasons: std::collections::BTreeMap<String, usize> = Default::default();
        let (mut total, mut ok) = (0, 0);
        let mut stack = vec![std::path::PathBuf::from(dir)];
        while let Some(path) = stack.pop() {
            let Ok(entries) = crate::platform::fs::read_dir(&path) else {
                continue;
            };
            for entry in entries.flatten() {
                let p = entry.path();
                if crate::platform::fs::is_dir(&p) {
                    stack.push(p);
                    continue;
                }
                if !p.to_string_lossy().ends_with(".slv") {
                    continue;
                }
                total += 1;
                let Ok(source) = crate::platform::fs::read_to_string(&p) else {
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
