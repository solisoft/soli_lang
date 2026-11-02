//! Class declaration compilation.

use std::sync::Arc;

use crate::ast::stmt::{ClassDecl, ConstructorDecl, FieldDecl, MethodDecl};

use super::chunk::Constant;
use super::compiler::{CompileResult, Compiler, FunctionType};
use super::opcode::Op;

impl Compiler {
    /// Compile a class declaration.
    pub fn compile_class_decl(&mut self, decl: &ClassDecl, line: usize) -> CompileResult<()> {
        // A nested class is its parent's member (`Billing::Invoice`): no
        // global, no local; its value is left for the parent to store.
        let nested = std::mem::take(&mut self.compiling_nested_class);
        let top_level = self.scope_depth == 0 && !nested;
        let qualified = match (nested, self.class_namespace.last()) {
            (true, Some(outer)) => format!("{outer}::{}", decl.name),
            _ => decl.name.clone(),
        };
        // A top-level class declaration defines a global of that name.
        if top_level {
            self.known_globals.borrow_mut().insert(decl.name.clone());
        }
        let name_idx = self.add_string_constant(&qualified);

        // Create the class or module
        if decl.is_module {
            self.emit(Op::Module(name_idx), line);
        } else {
            self.emit(Op::Class(name_idx), line);
        }

        // Handle superclass
        if let Some(ref superclass_name) = decl.superclass {
            let super_idx = self.add_string_constant(superclass_name);
            self.emit(Op::GetGlobal(super_idx), line);
            self.emit(Op::Inherit, line);
        }

        // Bind the class to its global name *before* compiling static field
        // initializers / static blocks / class statements. The tree-walker
        // registers the class first too (`execute_class`), and an enum's
        // unit-variant static consts reference the class by name during init
        // (`static const Active = __enum_construct(Status, "Active", {})`).
        // Method/field attachments below remain visible through the global
        // because `Class` uses interior mutability. (Top-level only; nested
        // classes bind to a local at the end, as before.)
        if top_level {
            let gname = self.add_string_constant(&decl.name);
            self.emit(Op::Dup, line);
            self.emit(Op::DefineGlobal(gname), line);
        }

        // Its methods read names in its namespace (`Billing::PayInvoice`
        // reads `Invoice` as `Billing::Invoice`) — when it has one.
        let prev_namespace = self.proto.namespace.clone();
        if qualified.contains("::") || !decl.nested_classes.is_empty() {
            self.proto.namespace = Some(std::sync::Arc::from(qualified.as_str()));
        }

        // Store class context for this/super resolution
        let prev_class_ctx = self.class_context.take();
        self.class_context = Some(super::compiler::ClassContext {
            has_superclass: decl.superclass.is_some(),
        });

        // Declare the fields. An instance field's literal default (or nil) is
        // kept by `Op::Field`; the others are evaluated for each new
        // instance by one method (see `FIELD_DEFAULTS_METHOD`).
        for field in &decl.fields {
            self.compile_field(field, line)?;
        }
        let computed: Vec<(String, crate::ast::Expr)> = decl
            .fields
            .iter()
            .filter(|field| !field.is_static)
            .filter_map(|field| {
                let init = field.initializer.as_ref()?;
                (!crate::interpreter::executor::access::member::is_literal_default(init))
                    .then(|| (field.name.clone(), init.clone()))
            })
            .collect();
        if let Some(method) = computed_defaults_method(&computed) {
            self.compile_method(&method, line)?;
        }

        // Compile methods
        for method in &decl.methods {
            self.compile_method(method, line)?;
        }
        for method in &decl.concern_class_methods {
            self.compile_method(method, line)?;
        }
        self.compile_invariants(decl, &qualified, line)?;

        if decl.is_module
            && (!decl.included_hooks.is_empty()
                || !decl.extended_hooks.is_empty()
                || !decl.concern_class_methods.is_empty())
        {
            crate::interpreter::mixin_registry::register(
                decl.name.clone(),
                crate::interpreter::mixin_registry::ModuleHookRecord {
                    included: decl.included_hooks.clone(),
                    extended: decl.extended_hooks.clone(),
                    concern_method_names: decl
                        .concern_class_methods
                        .iter()
                        .map(|m| m.name.clone())
                        .collect(),
                },
            );
        }

        // Resolve the module the same way any other name read does. Hardcoding
        // `GetGlobal` only worked for a top-level `module`: one declared inside a
        // function is a local (or an upvalue), so `include Inner` there failed with
        // "Undefined variable 'Inner'" under `--vm` while the tree-walker resolved
        // it through its scope chain.
        for module_name in &decl.includes {
            self.compile_module_ref(module_name, line)?;
            self.emit(Op::Include, line);
        }
        for module_name in &decl.extends {
            self.compile_module_ref(module_name, line)?;
            self.emit(Op::Extend, line);
        }

        // Compile the constructor: its body only. The fields are set before
        // any constructor runs (`Vm::init_fields`), so a `super(...)` into a
        // parent's never sets them again, and a class without `new` has none.
        if let Some(ref ctor) = decl.constructor {
            self.compile_constructor(ctor, line)?;
        }

        // Compile static block
        if let Some(ref static_block) = decl.static_block {
            for stmt in static_block {
                // Static block runs with the class on top of the stack
                self.emit(Op::Dup, line);
                self.compile_stmt(stmt)?;
            }
        }

        // Compile class statements (validates, before_save, etc.)
        for stmt in &decl.class_statements {
            self.emit(Op::Dup, line);
            self.compile_stmt(stmt)?;
        }

        // Compile nested classes: each is stored on its parent
        // (`[parent, parent, nested]` → SetProperty → `[parent, nested]`),
        // then the copy is dropped.
        for nested in &decl.nested_classes {
            self.emit(Op::Dup, line); // parent class on stack
            self.compiling_nested_class = true;
            self.class_namespace.push(qualified.clone());
            let compiled = self.compile_class_decl(nested, line);
            self.class_namespace.pop();
            compiled?;
            let nested_name_idx = self.add_string_constant(&nested.name);
            self.emit(Op::SetProperty(nested_name_idx), line);
            self.emit(Op::Pop, line);
            // `private class`: recorded on the parent, now back on top.
            if nested.is_private {
                self.emit(Op::MarkPrivate(nested_name_idx), line);
            }
        }

        // Restore class context
        self.class_context = prev_class_ctx;
        self.proto.namespace = prev_namespace;

        // Bind the class name. Top-level classes were bound to their global
        // early (above); just discard the working copy left on the stack. A
        // nested class stays on the stack for its parent.
        if !nested {
            if self.scope_depth > 0 {
                self.add_local(decl.name.clone(), false);
            } else {
                self.emit(Op::Pop, line);
            }
        }

        Ok(())
    }

    /// Each `invariant(...) { … }` becomes a private method named after the
    /// class, and their messages the static field `__invariants__` — the same
    /// shape the tree-walker builds (see `interpreter::domain`).
    fn compile_invariants(
        &mut self,
        decl: &ClassDecl,
        qualified: &str,
        line: usize,
    ) -> CompileResult<()> {
        if decl.invariants.is_empty() {
            return Ok(());
        }
        use crate::ast::{Expr, ExprKind};
        let mut rows = Vec::with_capacity(decl.invariants.len());
        for (i, invariant) in decl.invariants.iter().enumerate() {
            let method = MethodDecl {
                visibility: crate::ast::stmt::Visibility::Private,
                is_static: false,
                name: crate::interpreter::domain::invariant_method_name(qualified, i),
                params: Vec::new(),
                return_type: None,
                body: invariant.body.clone(),
                span: invariant.span,
            };
            self.compile_method(&method, line)?;
            let field = match &invariant.field {
                Some(f) => ExprKind::StringLiteral(f.clone()),
                None => ExprKind::Null,
            };
            rows.push(Expr::new(
                ExprKind::Array(vec![
                    Expr::new(
                        ExprKind::StringLiteral(invariant.message.clone()),
                        invariant.span,
                    ),
                    Expr::new(field, invariant.span),
                ]),
                invariant.span,
            ));
        }
        self.compile_expr(&Expr::new(ExprKind::Array(rows), decl.span))?;
        let name_idx = self.add_string_constant(crate::interpreter::domain::INVARIANTS_FIELD);
        self.emit(Op::StaticField(name_idx), line);
        Ok(())
    }

    fn compile_field(&mut self, field: &FieldDecl, line: usize) -> CompileResult<()> {
        let name_idx = self.add_string_constant(&field.name);

        // A static field holds its value. An instance field's `Op::Field`
        // keeps the value each new instance starts with: its literal default
        // or nil. A default that is not a literal must not run here, at class
        // definition: it runs once per instance, as on the tree-walker.
        let start = match &field.initializer {
            Some(init)
                if field.is_static
                    || crate::interpreter::executor::access::member::is_literal_default(init) =>
            {
                Some(init)
            }
            _ => None,
        };
        match start {
            Some(init) => self.compile_expr(init)?,
            None => {
                self.emit(Op::Null, line);
            }
        }

        match (field.is_static, field.is_const) {
            (true, true) => self.emit(Op::StaticConstField(name_idx), line),
            (true, false) => self.emit(Op::StaticField(name_idx), line),
            (false, true) => self.emit(Op::ConstField(name_idx), line),
            (false, false) => self.emit(Op::Field(name_idx), line),
        };
        if !field.is_static {
            match field.visibility {
                crate::ast::stmt::Visibility::Private => {
                    self.emit(Op::MarkPrivate(name_idx), line);
                }
                crate::ast::stmt::Visibility::Protected => {
                    self.emit(Op::MarkProtected(name_idx), line);
                }
                crate::ast::stmt::Visibility::Public => {}
            }
        }
        Ok(())
    }

    /// Push the module named by an `include` / `extend` onto the stack, resolved
    /// through the full scope chain (local, upvalue, then global).
    fn compile_module_ref(&mut self, module_name: &str, line: usize) -> CompileResult<()> {
        match self.resolve_variable(module_name) {
            super::compiler::VariableAccess::Local(slot) => {
                self.emit(Op::GetLocal(slot), line);
            }
            super::compiler::VariableAccess::Upvalue(idx) => {
                self.emit(Op::GetUpvalue(idx), line);
            }
            super::compiler::VariableAccess::Global(_) => {
                let idx = self.add_string_constant(module_name);
                self.emit(Op::GetGlobal(idx), line);
            }
        }
        Ok(())
    }

    fn compile_method(&mut self, method: &MethodDecl, line: usize) -> CompileResult<()> {
        let func_type = if method.is_static {
            FunctionType::Function
        } else {
            FunctionType::Method
        };

        let _dummy = self.start_function(func_type, method.name.clone(), &method.params);

        self.begin_scope();
        self.emit_param_defaults(&method.params)?;
        self.hoist_locals(&method.body, line);
        // A method's trailing expression is its return value, exactly as for a
        // free function (`compile_function_body`). Without this the expression
        // was compiled, popped, and the method fell off the end returning
        // null — so every method written in the documented implicit-return
        // style returned null under `--vm` while the tree-walker returned the
        // value. Constructors are deliberately excluded: they return the
        // instance, not their last expression.
        // `compile_tail_stmt` also carries tail position into a trailing
        // `if`/`unless`, whose taken branch gives the value.
        if let Some((last, init)) = method.body.split_last() {
            for stmt in init {
                self.compile_stmt(stmt)?;
            }
            self.compile_tail_stmt(last)?;
        }
        self.end_scope(line);

        let mut proto = self.finish_function(line);
        proto.return_type = method.return_type.clone();
        proto.decl_span = method.span;
        let fn_idx = self.add_constant(Constant::Function(Arc::new(proto)));
        self.emit(Op::Closure(fn_idx), line);

        let name_idx = self.add_string_constant(&method.name);
        if method.is_static {
            self.emit(Op::StaticMethod(name_idx), line);
        } else {
            self.emit(Op::Method(name_idx), line);
            match method.visibility {
                crate::ast::stmt::Visibility::Private => {
                    self.emit(Op::MarkPrivate(name_idx), line);
                }
                crate::ast::stmt::Visibility::Protected => {
                    self.emit(Op::MarkProtected(name_idx), line);
                }
                crate::ast::stmt::Visibility::Public => {}
            }
        }
        Ok(())
    }

    fn compile_constructor(&mut self, ctor: &ConstructorDecl, line: usize) -> CompileResult<()> {
        let _dummy =
            self.start_function(FunctionType::Constructor, "init".to_string(), &ctor.params);

        self.begin_scope();
        self.emit_param_defaults(&ctor.params)?;
        self.hoist_locals(&ctor.body, line);
        for stmt in &ctor.body {
            self.compile_stmt(stmt)?;
        }
        self.end_scope(line);

        // Constructor always returns `this`
        let proto = self.finish_constructor(line);
        let fn_idx = self.add_constant(Constant::Function(Arc::new(proto)));
        self.emit(Op::Closure(fn_idx), line);

        let name_idx = self.add_string_constant("init");
        self.emit(Op::Method(name_idx), line);
        Ok(())
    }

    /// Finish compiling a constructor — returns `this` instead of null.
    fn finish_constructor(&mut self, line: usize) -> super::chunk::FunctionProto {
        // Return this (slot 0) instead of implicit null
        self.emit(Op::GetLocal(0), line);
        self.emit(Op::Return, line);

        let mut proto = std::mem::replace(
            &mut self.proto,
            super::chunk::FunctionProto::new(String::new()),
        );
        proto.upvalue_descriptors = std::mem::take(&mut self.upvalues);
        proto.is_method = true;

        if let Some(enclosing) = self.enclosing.take() {
            *self = *enclosing;
        }

        proto
    }
}

/// The method that evaluates a class's field defaults that are not literals
/// on a new instance: `{"created": clock(), …}` (see
/// [`FIELD_DEFAULTS_METHOD`](crate::interpreter::value::FIELD_DEFAULTS_METHOD)).
/// `None` when there are none. The compiler compiles it with the class; for
/// a class the tree-walker defined, the VM compiles it the first time it
/// builds an instance (`Vm::init_fields`).
pub(crate) fn computed_defaults_method(
    defaults: &[(String, crate::ast::Expr)],
) -> Option<MethodDecl> {
    use crate::ast::{Expr, ExprKind, Stmt, StmtKind};
    let span = defaults.first()?.1.span;
    let pairs = defaults
        .iter()
        .map(|(name, init)| {
            let key = Expr::new(ExprKind::StringLiteral(name.clone()), init.span);
            (key, init.clone())
        })
        .collect();
    let body = Stmt::new(
        StmtKind::Expression(Expr::new(ExprKind::Hash(pairs), span)),
        span,
        None,
    );
    Some(MethodDecl {
        visibility: crate::ast::stmt::Visibility::Public,
        is_static: false,
        name: crate::interpreter::value::FIELD_DEFAULTS_METHOD.to_string(),
        params: Vec::new(),
        return_type: None,
        body: vec![body].into(),
        span,
    })
}
