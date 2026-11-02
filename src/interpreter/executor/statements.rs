//! Statement execution.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::ast::expr::Argument;
use crate::ast::*;
use crate::error::RuntimeError;
use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, Function, HashKey, HashPairs, Value};
use crate::span::Span;

use super::{ControlFlow, Interpreter, RuntimeResult};

impl Interpreter {
    /// Execute a statement, returning control flow information.
    pub(crate) fn execute(&mut self, stmt: &Stmt) -> RuntimeResult<ControlFlow> {
        // Handler budget. Every loop iteration and every call runs statements,
        // so checking here covers all of them; the check is a counter decrement
        // that reads the clock only every few thousand statements.
        if crate::interpreter::deadline::expired() {
            return Err(RuntimeError::General {
                message: crate::interpreter::deadline::timeout_message(),
                span: stmt.span,
            });
        }
        // ATTRIBUER LA LIGNE AU FICHIER QUI LA PORTE, PAS AU FICHIER D'ENTREE.
        //
        // `current_source_path` est le script d'entree (la spec en cours, la
        // requete en cours) : s'en servir ici faisait tomber les hits de
        // CHAQUE fonction appelee sur le fichier de l'appelant. Un controleur
        // dont les tests passent ressortait a 2,7 %, et ses lignes comptees
        // non couvertes s'executaient pourtant a chaque requete.
        //
        // `current_file_path()` prefere le fichier de la frame d'appel, pose
        // depuis le `source_path` de la fonction. C'est deja ce que fait
        // `record_coverage` pour les expressions, et ce que fait la
        // localisation des ERREURS quelques centaines de lignes plus bas —
        // avec le meme commentaire, et pour la meme raison.
        let source_path = if self.coverage_active() {
            stmt.source_path
                .clone()
                .or_else(|| self.current_file_path())
        } else {
            None
        };
        if let Some(ref path) = source_path {
            if let Some(ref tracker) = self.coverage_tracker {
                if let Ok(guard) = tracker.lock() {
                    guard.record_line_hit(path, stmt.span.line_usize());
                }
            } else if let Some(global) = crate::coverage::get_global_coverage_tracker() {
                if let Ok(guard) = global.lock() {
                    guard.record_line_hit(path, stmt.span.line_usize());
                }
            }
        }
        match &stmt.kind {
            StmtKind::Expression(expr) => {
                let value = self.evaluate(expr)?;
                // Check for breakpoint marker
                if matches!(value, Value::Breakpoint) {
                    let env_json = self.serialize_environment_for_debug();
                    let mut stack_trace = self.get_stack_trace();
                    let file = self
                        .current_source_path
                        .as_ref()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_else(|| "unknown".to_string());
                    stack_trace
                        .insert(0, format!("debug() at {}:{}", file, expr.span.line_usize()));
                    return Err(RuntimeError::breakpoint(expr.span, env_json, stack_trace));
                }
                if matches!(value, Value::Continue) {
                    return Ok(ControlFlow::Continue);
                }
                Ok(ControlFlow::Normal(value))
            }

            StmtKind::Let {
                name, initializer, ..
            } => {
                let value = if let Some(init) = initializer {
                    self.evaluate(init)?
                } else {
                    Value::Null
                };
                // define_or_update avoids a String allocation per execution when
                // the slot already exists — matters when `let` runs in a lambda
                // body that's invoked repeatedly through a reused environment
                // (array_map/filter/each iteration).
                self.environment.borrow_mut().define_or_update(name, value);
                Ok(ControlFlow::Normal(Value::Null))
            }

            StmtKind::Const {
                name, initializer, ..
            } => {
                let value = self.evaluate(initializer)?;
                self.environment
                    .borrow_mut()
                    .define_const(name.clone(), value);
                Ok(ControlFlow::Normal(Value::Null))
            }

            StmtKind::Block(statements) => self.execute_block(
                statements,
                Environment::with_enclosing(self.environment.clone()),
            ),

            StmtKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let cond_value = self.evaluate(condition)?;
                if cond_value.is_truthy() {
                    self.execute(then_branch)
                } else if let Some(else_br) = else_branch {
                    self.execute(else_br)
                } else {
                    Ok(ControlFlow::Normal(Value::Null))
                }
            }

            StmtKind::Unless {
                condition,
                then_branch,
                else_branch,
            } => {
                let cond_value = self.evaluate(condition)?;
                if !cond_value.is_truthy() {
                    self.execute(then_branch)
                } else if let Some(else_br) = else_branch {
                    self.execute(else_br)
                } else {
                    Ok(ControlFlow::Normal(Value::Null))
                }
            }

            StmtKind::While { condition, body } => {
                // Fast path: body is a Block — pre-allocate the block env once
                // and reuse it across iterations via execute_block_in. Avoids
                // 2 HashMap allocations per iteration for the common case of
                // `while cond { ... }`. Safe because StmtKind::Let now uses
                // define_or_update (same-slot write on re-entry).
                //
                // Skipped when the body creates a closure: a reused environment
                // would make closures captured in different iterations share the
                // same loop-local bindings. Those bodies fall through to the
                // normal path below, which executes the block in a fresh
                // environment each iteration (so each closure captures its own).
                if let StmtKind::Block(body_stmts) = &body.kind {
                    if !super::loop_capture::body_creates_closures(body_stmts) {
                        let loop_env_rc = Rc::new(RefCell::new(Environment::with_enclosing(
                            self.environment.clone(),
                        )));
                        while self.evaluate(condition)?.is_truthy() {
                            match self.execute_block_in(body_stmts, loop_env_rc.clone())? {
                                ControlFlow::Return(v) => return Ok(ControlFlow::Return(v)),
                                ControlFlow::Normal(_) => {}
                                ControlFlow::Throw(e) => return Ok(ControlFlow::Throw(e)),
                                ControlFlow::Continue => {}
                                ControlFlow::Break => break,
                            }
                        }
                        return Ok(ControlFlow::Normal(Value::Null));
                    }
                }

                while self.evaluate(condition)?.is_truthy() {
                    match self.execute(body)? {
                        ControlFlow::Return(v) => return Ok(ControlFlow::Return(v)),
                        ControlFlow::Normal(_) => {}
                        ControlFlow::Throw(e) => return Ok(ControlFlow::Throw(e)),
                        ControlFlow::Continue => {}
                        ControlFlow::Break => break,
                    }
                }
                Ok(ControlFlow::Normal(Value::Null))
            }

            StmtKind::For {
                variable,
                index_variable,
                iterable,
                body,
            } => self.execute_for_loop(variable, index_variable.as_deref(), iterable, body),

            StmtKind::Break => Ok(ControlFlow::Break),

            StmtKind::Return(value) => {
                let return_value = if let Some(expr) = value {
                    self.evaluate(expr)?
                } else {
                    Value::Null
                };
                Ok(ControlFlow::Return(return_value))
            }

            StmtKind::Function(decl) => {
                let source_path = self
                    .current_source_path
                    .as_ref()
                    .map(|p| p.to_string_lossy().to_string());
                let mut func = Function::from_decl(decl, self.environment.clone(), source_path);
                func.kernel = self.kernels.as_ref().and_then(|k| k.for_decl(decl));
                self.environment
                    .borrow_mut()
                    .define(decl.name.clone(), Value::Function(Rc::new(func)));
                Ok(ControlFlow::Normal(Value::Null))
            }

            StmtKind::Class(decl) => {
                self.execute_class(decl)?;
                Ok(ControlFlow::Normal(Value::Null))
            }

            StmtKind::Enum(decl) => {
                // Enums lower to an ordinary class the executor already runs.
                self.execute_class(&decl.lower_to_class())?;
                Ok(ControlFlow::Normal(Value::Null))
            }

            StmtKind::Interface(_) => {
                // Interfaces are handled at type-check time, no runtime effect
                Ok(ControlFlow::Normal(Value::Null))
            }

            StmtKind::Import(import_decl) => {
                // Module imports are resolved before execution by the ModuleResolver
                // If we reach here, it means import resolution hasn't been done yet
                Err(RuntimeError::General {
                    message: format!(
                        "Import '{}' was not resolved. Run module resolution first.",
                        import_decl.path
                    ),
                    span: stmt.span,
                })
            }

            StmtKind::Export(inner) => {
                // Export just executes the inner declaration and marks it as exported
                // The module system tracks what's exported
                self.execute(inner)
            }

            StmtKind::Throw(value) => {
                let error_value = self.evaluate(value)?;
                Ok(ControlFlow::Throw(error_value))
            }

            // NOTE: every path out of this statement must run `finally`
            // through `run_finally` and honour what it returns — see that
            // helper for why discarding it is wrong.
            StmtKind::Try {
                try_block,
                catch_clauses,
                finally_block,
            } => {
                let try_result = self.execute(try_block);

                let throw_value = match try_result {
                    Ok(control_flow) => match control_flow {
                        ControlFlow::Normal(_) | ControlFlow::Continue => None,
                        ControlFlow::Break => {
                            if let Some(over) = self.run_finally(finally_block.as_deref())? {
                                return Ok(over);
                            }
                            return Ok(ControlFlow::Break);
                        }
                        ControlFlow::Return(v) => {
                            if let Some(over) = self.run_finally(finally_block.as_deref())? {
                                return Ok(over);
                            }
                            return Ok(ControlFlow::Return(v));
                        }
                        ControlFlow::Throw(error) => Some(error),
                    },
                    // A user `throw` that crossed a call boundary arrives as
                    // `Thrown` and still holds its value — unwrap it, so
                    // catching a thrown hash yields the hash and not its
                    // rendering. Every other error is a Rust-side failure with
                    // no value to recover, so it is caught as its message.
                    Err(RuntimeError::Thrown { value, .. }) => Some(value),
                    Err(e) => {
                        // `catchable_message` strips the routing sentinels so a
                        // caught RecordNotFound/forbidden reads as its message,
                        // not as `__Forbidden__:nope`.
                        Some(Value::String(e.catchable_message().into()))
                    }
                };

                if let Some(error) = throw_value {
                    let mut caught = false;

                    for clause in catch_clauses {
                        // Check type match if typed catch
                        if let Some(ref type_name) = clause.type_name {
                            if !Self::value_matches_catch_type(&error, type_name) {
                                continue;
                            }
                        }

                        // Matched! Execute catch body
                        let mut catch_env = Environment::with_enclosing(self.environment.clone());
                        if let Some(ref var_name) = clause.var_name {
                            catch_env.define(var_name.clone(), error.clone());
                        }

                        let previous = std::mem::replace(
                            &mut self.environment,
                            Rc::new(RefCell::new(catch_env)),
                        );
                        let catch_result = self.execute(&clause.body);
                        self.environment = previous;

                        match catch_result {
                            Ok(ControlFlow::Normal(_) | ControlFlow::Continue) => {}
                            Ok(ControlFlow::Break) => {
                                if let Some(over) = self.run_finally(finally_block.as_deref())? {
                                    return Ok(over);
                                }
                                return Ok(ControlFlow::Break);
                            }
                            Ok(ControlFlow::Return(v)) => {
                                if let Some(over) = self.run_finally(finally_block.as_deref())? {
                                    return Ok(over);
                                }
                                return Ok(ControlFlow::Return(v));
                            }
                            Ok(ControlFlow::Throw(new_error)) => {
                                if let Some(over) = self.run_finally(finally_block.as_deref())? {
                                    return Ok(over);
                                }
                                return Ok(ControlFlow::Throw(new_error));
                            }
                            Err(e) => {
                                if let Some(over) = self.run_finally(finally_block.as_deref())? {
                                    return Ok(over);
                                }
                                return Err(e);
                            }
                        }
                        caught = true;
                        break;
                    }

                    if !caught {
                        // No clause matched — run `finally`, then re-throw. A
                        // `return`/`throw` from the finally block itself wins
                        // over the exception in flight, which is how it
                        // discards a pending error in Ruby.
                        if let Some(over) = self.run_finally(finally_block.as_deref())? {
                            return Ok(over);
                        }
                        return Ok(ControlFlow::Throw(error));
                    }
                }

                if let Some(over) = self.run_finally(finally_block.as_deref())? {
                    return Ok(over);
                }

                Ok(ControlFlow::Normal(Value::Null))
            }
        }
    }

    fn value_matches_catch_type(value: &Value, type_name: &str) -> bool {
        match value {
            Value::Instance(inst) => {
                let inst = inst.borrow();
                Self::class_matches_name(&inst.class, type_name)
            }
            _ => false,
        }
    }

    fn class_matches_name(class: &Class, type_name: &str) -> bool {
        if class.name == type_name {
            return true;
        }
        if let Some(ref superclass) = class.superclass {
            return Self::class_matches_name(superclass, type_name);
        }
        false
    }

    /// Run a `finally` block on a path that is already leaving the `try`
    /// statement, and report whether the block took over.
    ///
    /// `finally` runs on *every* exit — normal, `return`, `throw`, `break`,
    /// and a Rust-side error — which is the whole point of writing one. Each
    /// of those paths used to run it and then throw away its result, so a
    /// `return` or `throw` inside the finally block was silently ignored
    /// whenever the try was already unwinding. `Some(cf)` here means the
    /// finally block exited on its own and its exit replaces the one in
    /// progress, including discarding a pending exception — which is what
    /// Ruby's `ensure` does, and what the fall-through path here already did.
    fn run_finally(&mut self, finally_block: Option<&Stmt>) -> RuntimeResult<Option<ControlFlow>> {
        let Some(block) = finally_block else {
            return Ok(None);
        };
        Ok(match self.execute(block)? {
            ControlFlow::Return(v) => Some(ControlFlow::Return(v)),
            ControlFlow::Throw(e) => Some(ControlFlow::Throw(e)),
            ControlFlow::Break => Some(ControlFlow::Break),
            ControlFlow::Normal(_) | ControlFlow::Continue => None,
        })
    }

    fn execute_for_loop(
        &mut self,
        variable: &str,
        index_variable: Option<&str>,
        iterable: &Expr,
        body: &Stmt,
    ) -> RuntimeResult<ControlFlow> {
        // Range fast path: `for i in a..b` iterates directly instead of
        // materializing the range into an array first (eval_range collects
        // the whole range into a Vec, which is wasteful for large ranges).
        if let ExprKind::Binary {
            left,
            operator: BinaryOp::Range,
            right,
        } = &iterable.kind
        {
            let left_val = self.evaluate(left)?;
            let right_val = self.evaluate(right)?;
            match (&left_val, &right_val) {
                (Value::Int(start), Value::Int(end)) => {
                    let mut current = *start;
                    let end = *end;
                    return self.run_for_loop(variable, index_variable, body, || {
                        if current < end {
                            let val = Value::Int(current);
                            current += 1;
                            Some(val)
                        } else {
                            None
                        }
                    });
                }
                _ => {
                    // Same error eval_range would produce for non-int operands.
                    return Err(RuntimeError::type_error(
                        format!(
                            "range (..) expects two integers, got {} and {}",
                            left_val.type_name(),
                            right_val.type_name()
                        ),
                        iterable.span,
                    ));
                }
            }
        }

        let iter_value = self.evaluate(iterable)?;
        // QueryBuilder is iterable: materialize it into an Array up front.
        let iter_value = match iter_value {
            Value::QueryBuilder(qb) => crate::interpreter::builtins::model::raise_if_error_value(
                crate::interpreter::builtins::model::execute_query_builder(&qb.borrow())
                    .force_deferred(),
            )
            .map_err(|message| RuntimeError::General {
                message,
                span: iterable.span,
            })?,
            // A read inside `grouped(fn() { ... })` hands back a Deferred
            // placeholder. Resolve it here (flushing the batch if it is still
            // pending) so `for x in @posts` iterates the rows. Without this the
            // Deferred falls through to the error arm below, where `type_name()`
            // forces it and reports the *resolved* type — producing the
            // self-contradictory "cannot iterate over array".
            deferred @ Value::Deferred(_) => deferred.force_deferred(),
            other => other,
        };
        match iter_value {
            Value::Array(arr) => {
                // Live bounds-checked indexing: no upfront snapshot clone of
                // the whole array. The borrow is taken only long enough to
                // clone the current item, then released before the body runs,
                // so a body that mutates the array is safe (and observed
                // live, matching the VM's iterator).
                let mut i = 0usize;
                self.run_for_loop(variable, index_variable, body, || {
                    let items = arr.borrow();
                    if i < items.len() {
                        let item = items[i].clone();
                        i += 1;
                        Some(item)
                    } else {
                        None
                    }
                })
            }
            _ => {
                // Include the offending value in the message so the user can
                // see *which* string (or scalar) snuck into the iterable slot.
                // Strings get truncated to keep the page readable; longer
                // ones drop a trailing `…`. Other scalars are short enough
                // to inline verbatim.
                let message = match &iter_value {
                    Value::String(s) => {
                        const MAX: usize = 120;
                        let preview: String = if s.chars().count() > MAX {
                            let mut p: String = s.chars().take(MAX).collect();
                            p.push('…');
                            p
                        } else {
                            s.clone().to_string()
                        };
                        format!("cannot iterate over string {:?}", preview)
                    }
                    Value::Null => "cannot iterate over null".to_string(),
                    Value::Int(n) => format!("cannot iterate over int {}", n),
                    Value::Float(n) => format!("cannot iterate over float {}", n),
                    Value::Bool(b) => format!("cannot iterate over bool {}", b),
                    other => format!("cannot iterate over {}", other.type_name()),
                };
                Err(RuntimeError::type_error(message, iterable.span))
            }
        }
    }

    /// Drive a for-loop body over the values produced by `next`.
    ///
    /// Handles both execution strategies:
    /// - closure-capturing bodies get a fresh environment per iteration so
    ///   each closure captures a distinct loop binding;
    /// - other bodies reuse a single loop environment (define_or_update
    ///   same-slot writes instead of fresh HashMap allocations).
    fn run_for_loop(
        &mut self,
        variable: &str,
        index_variable: Option<&str>,
        body: &Stmt,
        mut next: impl FnMut() -> Option<Value>,
    ) -> RuntimeResult<ControlFlow> {
        if super::loop_capture::stmt_creates_closures(body) {
            let outer = self.environment.clone();
            let mut i: i64 = 0;
            while let Some(item) = next() {
                let iter_env = Rc::new(RefCell::new(Environment::with_enclosing(outer.clone())));
                {
                    let mut env = iter_env.borrow_mut();
                    env.define(variable.to_string(), item);
                    if let Some(idx_var) = index_variable {
                        env.define(idx_var.to_string(), Value::Int(i));
                    }
                }
                let prev_env = std::mem::replace(&mut self.environment, iter_env);
                let result = self.execute(body);
                self.environment = prev_env;
                match result {
                    Err(e) => return Err(e),
                    Ok(ControlFlow::Return(v)) => return Ok(ControlFlow::Return(v)),
                    Ok(ControlFlow::Throw(e)) => return Ok(ControlFlow::Throw(e)),
                    Ok(ControlFlow::Break) => break,
                    Ok(ControlFlow::Normal(_)) | Ok(ControlFlow::Continue) => {}
                }
                i += 1;
            }
            return Ok(ControlFlow::Normal(Value::Null));
        }

        let loop_env_rc = Rc::new(RefCell::new(Environment::with_enclosing(
            self.environment.clone(),
        )));
        loop_env_rc
            .borrow_mut()
            .define(variable.to_string(), Value::Null);
        if let Some(idx_var) = index_variable {
            loop_env_rc
                .borrow_mut()
                .define(idx_var.to_string(), Value::Int(0));
        }

        let prev_env = std::mem::replace(&mut self.environment, loop_env_rc.clone());

        let mut i: i64 = 0;
        while let Some(item) = next() {
            {
                let mut env = loop_env_rc.borrow_mut();
                env.define_or_update(variable, item);
                if let Some(idx_var) = index_variable {
                    env.define_or_update(idx_var, Value::Int(i));
                }
            }
            let result = self.execute(body);
            match result {
                Err(e) => {
                    self.environment = prev_env;
                    return Err(e);
                }
                Ok(ControlFlow::Return(v)) => {
                    self.environment = prev_env;
                    return Ok(ControlFlow::Return(v));
                }
                Ok(ControlFlow::Throw(e)) => {
                    self.environment = prev_env;
                    return Ok(ControlFlow::Throw(e));
                }
                Ok(ControlFlow::Break) => break,
                Ok(ControlFlow::Normal(_)) | Ok(ControlFlow::Continue) => {}
            }
            i += 1;
        }

        self.environment = prev_env;
        Ok(ControlFlow::Normal(Value::Null))
    }

    pub(super) fn execute_class(&mut self, decl: &ClassDecl) -> RuntimeResult<()> {
        let qualified = qualified_class_name(&decl.name);
        // `module Billing` declared again — in another file of the same
        // bounded context — adds to the module instead of replacing it.
        if decl.is_module {
            let existing = self.environment.borrow().get(&decl.name);
            if let Some(Value::Class(module)) = existing {
                if module.is_module && module.name == qualified {
                    return self.reopen_module(module, decl);
                }
            }
        }
        let superclass = if let Some(ref superclass_name) = decl.superclass {
            match self.environment.borrow().get(superclass_name) {
                Some(Value::Class(class)) => Some(class),
                Some(_) => {
                    return Err(RuntimeError::NotAClass(superclass_name.clone(), decl.span));
                }
                None => {
                    return Err(RuntimeError::undefined_variable(superclass_name, decl.span));
                }
            }
        } else {
            None
        };

        // Check if this class extends Model (directly or indirectly)
        let extends_model = superclass.as_ref().is_some_and(|sc| sc.is_model_subclass());

        // Create environment for methods (with potential super binding). A
        // top-level class or module reads its nested classes unqualified
        // (`Inner.new(...)` in a method of `Outer`); a nested one already
        // reads them through the namespace its declaration bound.
        let binds_namespace = !decl.nested_classes.is_empty() && self.current_namespace().is_none();
        let method_env = if binds_namespace || superclass.is_some() {
            let mut env = Environment::with_enclosing(self.environment.clone());
            if binds_namespace {
                env.define(
                    crate::interpreter::domain::NAMESPACE_BINDING.to_string(),
                    Value::String(qualified.clone().into()),
                );
            }
            // Store the superclass for super calls within methods of this class
            if let Some(ref sc) = superclass {
                env.define(
                    "__defining_superclass__".to_string(),
                    Value::Class(sc.clone()),
                );
            }
            Rc::new(RefCell::new(env))
        } else {
            self.environment.clone()
        };

        // Collect methods
        let mut methods = HashMap::new();
        let mut static_methods = HashMap::new();
        let mut private_members = HashSet::new();
        let mut protected_members = HashSet::new();

        let source_path = self
            .current_source_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string());

        for method_decl in &decl.methods {
            let func = Function::from_method(method_decl, method_env.clone(), source_path.clone());
            if method_decl.is_static {
                let mut func = func;
                func.binds_this = !decl.is_module && reads_this(&method_decl.body);
                static_methods.insert(method_decl.name.clone(), Rc::new(func));
            } else {
                match method_decl.visibility {
                    crate::ast::stmt::Visibility::Private => {
                        private_members.insert(method_decl.name.clone());
                    }
                    crate::ast::stmt::Visibility::Protected => {
                        protected_members.insert(method_decl.name.clone());
                    }
                    crate::ast::stmt::Visibility::Public => {}
                }
                let func = Rc::new(func);
                // Modules also expose instance methods as module functions
                // so `MyModule.hello()` works (Ruby `extend self`).
                if decl.is_module {
                    static_methods.insert(method_decl.name.clone(), func.clone());
                }
                methods.insert(method_decl.name.clone(), func);
            }
        }

        // Each `invariant(...) { … }` block becomes a private method, named
        // after this class so a subclass's do not replace its parent's.
        for (i, invariant) in decl.invariants.iter().enumerate() {
            let method_decl = crate::ast::stmt::MethodDecl {
                visibility: crate::ast::stmt::Visibility::Private,
                is_static: false,
                name: crate::interpreter::domain::invariant_method_name(&qualified, i),
                params: Vec::new(),
                return_type: None,
                body: invariant.body.clone(),
                span: invariant.span,
            };
            let func = Function::from_method(&method_decl, method_env.clone(), source_path.clone());
            private_members.insert(method_decl.name.clone());
            methods.insert(method_decl.name.clone(), Rc::new(func));
        }

        if !private_members.is_empty() || !protected_members.is_empty() {
            crate::interpreter::value::note_restricted_methods_declared();
        }

        // Create constructor if present
        let constructor = decl.constructor.as_ref().map(|ctor| {
            Rc::new(Function {
                name: "new".to_string(),
                params: ctor.params.clone().into(),
                body: Rc::new(ctor.body.clone()),
                closure: method_env.clone(),
                is_method: true,
                span: Some(ctor.span),
                source_path: self
                    .current_source_path
                    .as_ref()
                    .map(|p| p.to_string_lossy().to_string()),
                defining_superclass: None,
                return_type: None,
                cached_env: RefCell::new(None),
                jit_cache: RefCell::new(None),
                binds_this: false,
                kernel: None,
            })
        });

        // If extending Model, inherit Model's native static methods
        let native_static_methods = if extends_model {
            if let Some(ref sc) = superclass {
                sc.native_static_methods.clone()
            } else {
                HashMap::new()
            }
        } else {
            HashMap::new()
        };

        let mut fields = HashMap::new();
        let mut static_field_initializers = HashMap::new();
        let mut const_fields = HashSet::new();
        let mut static_const_fields = HashSet::new();
        for field in &decl.fields {
            if field.is_static {
                static_field_initializers.insert(field.name.clone(), field.initializer.clone());
                if field.is_const {
                    static_const_fields.insert(field.name.clone());
                }
            } else {
                fields.insert(field.name.clone(), field.initializer.clone());
                if field.is_const {
                    const_fields.insert(field.name.clone());
                }
                match field.visibility {
                    crate::ast::stmt::Visibility::Private => {
                        private_members.insert(field.name.clone());
                    }
                    crate::ast::stmt::Visibility::Protected => {
                        protected_members.insert(field.name.clone());
                    }
                    crate::ast::stmt::Visibility::Public => {}
                }
            }
        }
        if !private_members.is_empty() || !protected_members.is_empty() {
            crate::interpreter::value::note_restricted_methods_declared();
        }

        let class = Class {
            name: qualified,
            superclass,
            methods: Rc::new(RefCell::new(methods)),
            static_methods,
            native_static_methods,
            native_methods: HashMap::new(),
            constructor,
            static_fields: Rc::new(RefCell::new(HashMap::new())),
            fields,
            nested_classes: Rc::new(RefCell::new(HashMap::new())),
            const_fields,
            static_const_fields,
            private_members: Rc::new(RefCell::new(private_members)),
            protected_members: Rc::new(RefCell::new(protected_members)),
            is_module: decl.is_module,
            ..Default::default()
        };

        *class.declared_fields.borrow_mut() = decl
            .fields
            .iter()
            .filter(|f| !f.is_static)
            .map(|f| f.name.clone())
            .collect();
        if !decl.invariants.is_empty() {
            let specs: Vec<(String, Option<String>)> = decl
                .invariants
                .iter()
                .map(|i| (i.message.clone(), i.field.clone()))
                .collect();
            class.static_fields.borrow_mut().insert(
                crate::interpreter::domain::INVARIANTS_FIELD.to_string(),
                crate::interpreter::domain::invariants_value(&specs),
            );
        }
        crate::interpreter::domain::note_class(&class);
        let class_rc = Rc::new(class);
        crate::interpreter::domain::register_class(&class_rc);
        if !decl.included_hooks.is_empty() {
            *class_rc.included_hook_stmts.borrow_mut() = decl.included_hooks.clone();
        }
        if !decl.extended_hooks.is_empty() {
            *class_rc.extended_hook_stmts.borrow_mut() = decl.extended_hooks.clone();
        }
        if !decl.concern_class_methods.is_empty() {
            let mut names = class_rc.concern_method_names.borrow_mut();
            let mut stored = class_rc.concern_static_methods.borrow_mut();
            let mut on_module = class_rc.mixin_static_methods.borrow_mut();
            for method_decl in &decl.concern_class_methods {
                let func = Rc::new(Function::from_method(
                    method_decl,
                    method_env.clone(),
                    source_path.clone(),
                ));
                names.push(method_decl.name.clone());
                stored.insert(method_decl.name.clone(), func.clone());
                on_module.insert(method_decl.name.clone(), func);
            }
        }
        self.environment
            .borrow_mut()
            .define(decl.name.clone(), Value::Class(class_rc.clone()));

        // Call inherited hook if superclass has one
        if let Some(ref superclass) = class_rc.superclass {
            // Check for inherited static method
            if let Some(inherited_func) = superclass.find_static_method("inherited") {
                self.call_function(&inherited_func, vec![Value::Class(class_rc.clone())])?;
            }
            // Check for inherited native static method
            if let Some(inherited_native) = superclass.find_native_static_method("inherited") {
                let span = Span::new(0, 0, 1, 1);
                let result: Result<Value, String> =
                    (inherited_native.func)(&[Value::Class(class_rc.clone())]);
                result.map_err(|e| RuntimeError::new(e, span))?;
            }
        }

        // Register model classes for lazy relation conversion
        if extends_model {
            use crate::interpreter::builtins::model::register_model_class;
            register_model_class(&class_rc.name, class_rc.clone());
        }
        // The callbacks the body declares — mixins' `included` hooks, the
        // static block, the class-level calls — replace the class's when it
        // has run, so running it again (a hot reload) does not add them twice.
        // A model inheriting from another starts from its parent's.
        let callbacks_declaration = extends_model.then(|| {
            use crate::interpreter::builtins::model::callbacks;
            let inherited = class_rc
                .superclass
                .as_ref()
                .filter(|sc| sc.name != "Model" && sc.is_model_subclass())
                .map(|parent| callbacks::registered(&parent.name))
                .unwrap_or_default();
            callbacks::Declaration::begin(&class_rc.name, inherited)
        });

        // Initialize static fields
        for (field_name, field_initializer) in static_field_initializers {
            let value = if let Some(init_expr) = field_initializer {
                self.evaluate(&init_expr)?
            } else {
                Value::Null
            };
            class_rc
                .static_fields
                .borrow_mut()
                .insert(field_name, value);
        }

        self.apply_mixins(&class_rc, &decl.includes, &decl.extends, decl.span)?;

        // Execute static block if present
        if let Some(ref static_block) = decl.static_block {
            // Create a temporary "this" context for the static block
            // The static block can access the class via `this` or directly
            let static_env = Rc::new(RefCell::new(Environment::with_enclosing(
                self.environment.clone(),
            )));
            let this_value = Value::Class(class_rc.clone());
            static_env
                .borrow_mut()
                .define("this".to_string(), this_value);

            // Execute each statement in the static block
            for stmt in static_block {
                self.execute_with_env(stmt, static_env.clone())?;
            }
        }

        // Execute class-level statements (validates, callbacks, etc.) for Model subclasses
        if extends_model && !decl.class_statements.is_empty() {
            self.execute_class_level_calls(&class_rc, &decl.class_statements)?;
        }

        // Process nested classes
        if !decl.nested_classes.is_empty() {
            self.execute_nested_classes(&decl.nested_classes, class_rc.clone())?;
        }

        if let Some(declaration) = callbacks_declaration {
            declaration.install();
        }
        Ok(())
    }

    /// Run class-body DSL calls (`validates`, `has_many`, …) with the class
    /// prepended as the first argument — same dispatch as a Model class body.
    pub(crate) fn execute_class_level_calls(
        &mut self,
        class_rc: &Rc<Class>,
        stmts: &[crate::ast::Stmt],
    ) -> RuntimeResult<()> {
        for stmt in stmts {
            // For expression statements that are function calls,
            // we need to pass the class as the first argument
            if let crate::ast::StmtKind::Expression(expr) = &stmt.kind {
                if let crate::ast::ExprKind::Call { callee, arguments } = &expr.kind {
                    // `state_machine :field do … end` — the declarative state
                    // machine DSL. Unlike `validates`/`scope`, its block must
                    // be invoked with `&mut Interpreter` (so nested
                    // `event`/`transition`/`guard` calls run), so it can't be a
                    // plain native. Route it to `define_state_machine` and skip
                    // the generic class-statement dispatch below.
                    if let crate::ast::ExprKind::Variable(fname) = &callee.kind {
                        if fname == "state_machine" {
                            let block = arguments.iter().find_map(|a| match a {
                                Argument::Block(e) => Some(e),
                                Argument::Positional(e)
                                    if matches!(e.kind, crate::ast::ExprKind::Lambda { .. }) =>
                                {
                                    Some(e)
                                }
                                _ => None,
                            });
                            let field = arguments.iter().find_map(|a| match a {
                                Argument::Positional(e)
                                    if !matches!(e.kind, crate::ast::ExprKind::Lambda { .. }) =>
                                {
                                    Some(e)
                                }
                                _ => None,
                            });
                            if let (Some(field_expr), Some(block_expr)) = (field, block) {
                                self.define_state_machine(
                                    class_rc.clone(),
                                    field_expr,
                                    block_expr,
                                    stmt.span,
                                )?;
                                continue;
                            }
                        }
                    }

                    // Get the callee value (should be a native function from Model)
                    let callee_val = self.evaluate(callee)?;

                    // Build arguments with class as first argument
                    let mut args = vec![Value::Class(class_rc.clone())];
                    let has_named = arguments
                        .iter()
                        .any(|arg| matches!(arg, Argument::Named(_)));
                    if has_named {
                        let mut hash_pairs = HashPairs::default();
                        for arg in arguments {
                            match arg {
                                Argument::Positional(expr) => {
                                    args.push(self.evaluate(expr)?);
                                }
                                Argument::Named(named) => {
                                    let val = self.evaluate(&named.value)?;
                                    hash_pairs
                                        .insert(HashKey::String(named.name.clone().into()), val);
                                }
                                Argument::Block(_) => {
                                    return Err(RuntimeError::type_error(
                                        "model validation does not support block arguments",
                                        stmt.span,
                                    ));
                                }
                            }
                        }
                        args.push(Value::Hash(Rc::new(RefCell::new(hash_pairs))));
                    } else {
                        for arg in arguments {
                            match arg {
                                Argument::Positional(expr) => {
                                    args.push(self.evaluate(expr)?);
                                }
                                Argument::Named(_) => {
                                    return Err(RuntimeError::type_error(
                                        "model validation does not support named arguments",
                                        stmt.span,
                                    ));
                                }
                                Argument::Block(_) => {
                                    return Err(RuntimeError::type_error(
                                        "model validation does not support block arguments",
                                        stmt.span,
                                    ));
                                }
                            }
                        }
                    }

                    // Call the function with the class
                    self.call_value(callee_val, args, stmt.span)?;
                } else {
                    self.execute(stmt)?;
                }
            } else {
                self.execute(stmt)?;
            }
        }
        Ok(())
    }

    fn apply_mixins(
        &mut self,
        class: &Rc<crate::interpreter::value::Class>,
        includes: &[String],
        extends: &[String],
        span: crate::span::Span,
    ) -> RuntimeResult<()> {
        for name in includes {
            let module = self.resolve_module(name, span)?;
            // Fire hooks for every module that actually joined the class —
            // the named one plus its transitive includes, innermost first.
            // Firing only for the named module meant a nested concern's
            // `included do` was registered against the intermediate module and
            // never ran against the host.
            let added = class
                .include_module_collecting(&module)
                .map_err(|e| RuntimeError::new(e, span))?;
            for joined in &added {
                self.fire_mixin_hooks(class, joined, true, span)?;
            }
        }
        for name in extends {
            let module = self.resolve_module(name, span)?;
            class
                .extend_module(&module)
                .map_err(|e| RuntimeError::new(e, span))?;
            self.fire_mixin_hooks(class, &module, false, span)?;
        }
        Ok(())
    }

    fn fire_mixin_hooks(
        &mut self,
        class: &Rc<Class>,
        module: &Rc<Class>,
        including: bool,
        span: crate::span::Span,
    ) -> RuntimeResult<()> {
        let hooks = if including {
            module.included_hook_stmts.borrow().clone()
        } else {
            module.extended_hook_stmts.borrow().clone()
        };
        for body in hooks {
            self.execute_class_level_calls(class, &body)?;
        }
        let hook_name = if including { "included" } else { "extended" };
        if let Some(func) = module.find_static_method(hook_name) {
            self.call_function(&func, vec![Value::Class(class.clone())])?;
        }
        if let Some(native) = module.find_native_static_method(hook_name) {
            (native.func)(&[Value::Class(class.clone())])
                .map_err(|e| RuntimeError::new(e, span))?;
        }
        Ok(())
    }

    fn resolve_module(
        &self,
        name: &str,
        span: crate::span::Span,
    ) -> RuntimeResult<Rc<crate::interpreter::value::Class>> {
        match self.environment.borrow().get(name) {
            Some(Value::Class(class)) => Ok(class),
            Some(other) => Err(RuntimeError::type_error(
                format!("expected module {}, got {}", name, other.type_name()),
                span,
            )),
            None => Err(RuntimeError::undefined_variable(name, span)),
        }
    }

    /// Execute nested class declarations
    fn execute_nested_classes(
        &mut self,
        nested_decls: &[ClassDecl],
        parent_class: Rc<Class>,
    ) -> RuntimeResult<()> {
        for nested_decl in nested_decls {
            let _namespace = NamespaceGuard::enter(&parent_class.name);
            // Save the current environment
            let previous_env = self.environment.clone();

            // The environment the nested class is declared in, enclosing the
            // parent's — the one it was declared in before, when it is
            // declared again in the same place (see `nested_class_env`).
            let nested_env =
                nested_class_env(&qualified_class_name(&nested_decl.name), &previous_env);

            // Define the parent class in the nested environment
            nested_env.borrow_mut().define(
                parent_class.name.clone(),
                Value::Class(parent_class.clone()),
            );
            // Names read unqualified in its methods resolve in its namespace.
            nested_env.borrow_mut().define(
                crate::interpreter::domain::NAMESPACE_BINDING.to_string(),
                Value::String(qualified_class_name(&nested_decl.name).into()),
            );

            // Set the environment to the nested environment
            self.environment = nested_env;

            // Execute the nested class declaration
            self.execute_class(nested_decl)?;

            // After execute_class, the nested class should be in the environment
            // Get it and store it in the parent's nested_classes map
            if let Some(Value::Class(nested_class)) =
                self.environment.borrow().get(&nested_decl.name)
            {
                parent_class
                    .nested_classes
                    .borrow_mut()
                    .insert(nested_decl.name.clone(), nested_class.clone());
                if nested_decl.is_private {
                    parent_class
                        .private_members
                        .borrow_mut()
                        .insert(nested_decl.name.clone());
                    crate::interpreter::domain::note_private_class(
                        &parent_class,
                        &nested_decl.name,
                    );
                }
            }

            // Restore the previous environment
            self.environment = previous_env;
        }

        Ok(())
    }

    /// `module Billing` declared a second time: its classes join the module's
    /// nested classes, its methods the module's methods.
    fn reopen_module(&mut self, module: Rc<Class>, decl: &ClassDecl) -> RuntimeResult<()> {
        for method_decl in &decl.methods {
            let func = Rc::new(Function::from_method(
                method_decl,
                self.environment.clone(),
                self.current_source_path
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned()),
            ));
            if method_decl.is_static {
                module
                    .mixin_static_methods
                    .borrow_mut()
                    .insert(method_decl.name.clone(), func);
            } else {
                module
                    .methods
                    .borrow_mut()
                    .insert(method_decl.name.clone(), func);
            }
        }
        module.invalidate_method_cache();
        self.execute_nested_classes(&decl.nested_classes, module)
    }

    /// Execute a statement with a custom environment (used for static blocks).
    fn execute_with_env(
        &mut self,
        stmt: &Stmt,
        env: Rc<RefCell<Environment>>,
    ) -> RuntimeResult<()> {
        let previous_env = std::mem::replace(&mut self.environment, env);
        let result = self.execute(stmt);
        self.environment = previous_env;
        result?;
        Ok(())
    }
}

thread_local! {
    /// The qualified names of the classes and modules being declared, so a
    /// nested one is named after them: `Billing::Invoice`.
    static NAMESPACE: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// The name a class declared here gets: qualified by the module or class it
/// is nested in.
pub(crate) fn qualified_class_name(name: &str) -> String {
    NAMESPACE.with(|ns| match ns.borrow().last() {
        Some(outer) => format!("{outer}::{name}"),
        None => name.to_string(),
    })
}

thread_local! {
    /// The environment each nested class was declared in, by qualified name.
    /// Weak: the class keeps its environment alive (its methods close over
    /// it), and nothing here keeps either.
    static NESTED_CLASS_ENVS: RefCell<HashMap<String, std::rc::Weak<RefCell<Environment>>>> =
        RefCell::new(HashMap::new());
}

/// The environment a nested class declares itself in.
///
/// It defines the class, and the class's methods close over it: a cycle,
/// which for a top-level class is harmless, because there the environment is
/// the global one and declaring the class again — a hot reload re-runs every
/// model file — rebinds the name and lets the old class go. A nested class
/// used to get a fresh environment each time, so the old one kept the old
/// class, and every method and closure of it, for the life of the process:
/// on an app whose models live in bounded contexts (`module Billing`), each
/// reload of a `--dev` server leaked a copy of all of them, per worker —
/// 15 GB in two days of editing one such app.
///
/// So the environment is reused when the same class is declared again in
/// the same scope: alive still, and enclosing the same environment (the same
/// interpreter, the same module). Declaring the class then rebinds it, as at
/// the top level.
fn nested_class_env(
    qualified: &str,
    enclosing: &Rc<RefCell<Environment>>,
) -> Rc<RefCell<Environment>> {
    let reused = NESTED_CLASS_ENVS.with(|envs| {
        envs.borrow()
            .get(qualified)
            .and_then(std::rc::Weak::upgrade)
            .filter(|env| {
                env.borrow()
                    .enclosing()
                    .is_some_and(|outer| Rc::ptr_eq(&outer, enclosing))
            })
    });
    if let Some(env) = reused {
        return env;
    }
    let env = Rc::new(RefCell::new(Environment::with_enclosing(enclosing.clone())));
    NESTED_CLASS_ENVS.with(|envs| {
        let mut envs = envs.borrow_mut();
        envs.retain(|_, weak| weak.strong_count() > 0);
        envs.insert(qualified.to_string(), Rc::downgrade(&env));
    });
    env
}

struct NamespaceGuard;

impl NamespaceGuard {
    fn enter(name: &str) -> Self {
        NAMESPACE.with(|ns| ns.borrow_mut().push(name.to_string()));
        NamespaceGuard
    }
}

impl Drop for NamespaceGuard {
    fn drop(&mut self) {
        NAMESPACE.with(|ns| {
            ns.borrow_mut().pop();
        });
    }
}

/// Whether a method body reads `this` (or `@field`, which parses to it).
/// Asked once per `static def` when its class is defined: the serialized
/// AST names the `This` node — a string literal spelling it errs toward
/// binding, which is harmless.
fn reads_this(body: &[crate::ast::Stmt]) -> bool {
    serde_json::to_string(body).map_or(true, |json| json.contains("\"This\""))
}

#[cfg(test)]
mod nested_class_reload_tests {
    use super::*;

    fn run(interpreter: &mut Interpreter, source: &str) {
        let tokens = crate::lexer::Scanner::new(source)
            .scan_tokens()
            .expect("lexer error");
        let program = crate::parser::Parser::new(tokens)
            .parse()
            .expect("parser error");
        interpreter.interpret(&program).expect("runtime error");
    }

    fn contact(interpreter: &Interpreter) -> Rc<Class> {
        let Some(Value::Class(module)) = interpreter.environment.borrow().get("Billing") else {
            panic!("Billing is not defined");
        };
        let nested = module.nested_classes.borrow();
        nested.get("Contact").cloned().expect("Billing::Contact")
    }

    /// Declaring a module's class again — what a hot reload does to every
    /// model file — lets the previous class go, as it does at the top level.
    /// It used to stay in a cycle with the environment it was declared in,
    /// its methods closing over it, and a `--dev` server editing an app of
    /// bounded contexts grew by a copy of every such class per reload.
    #[test]
    fn a_nested_class_declared_again_frees_the_old_one() {
        let source = "module Billing\n  class Contact\n    def label\n      \"contact\"\n    end\n  end\nend\n";
        let mut interpreter = Interpreter::new();
        run(&mut interpreter, source);
        let first = Rc::downgrade(&contact(&interpreter));

        run(&mut interpreter, source);
        let second = contact(&interpreter);
        assert!(
            first.upgrade().is_none(),
            "the first Billing::Contact is still alive"
        );

        // The class declared again keeps its methods.
        let label = second
            .methods
            .borrow()
            .get("label")
            .cloned()
            .expect("label");
        assert_eq!(label.body.len(), 1);
    }

    /// A model class declared again keeps one set of callbacks: its body's,
    /// with a name declared twice still there twice.
    #[test]
    fn a_model_declared_again_keeps_its_callbacks_once() {
        let source = "class ReloadedCallbacks < Model\n  before_save(\"stamp\")\n  before_save(\"stamp\")\n  after_create(\"announce\")\n\n  def stamp\n    true\n  end\n\n  def announce\n    true\n  end\nend\n";
        let mut interpreter = Interpreter::new();
        for _ in 0..3 {
            run(&mut interpreter, source);
        }
        let callbacks =
            crate::interpreter::builtins::model::callbacks::registered("ReloadedCallbacks");
        assert_eq!(callbacks.before_save, vec!["stamp", "stamp"]);
        assert_eq!(callbacks.after_create, vec!["announce"]);
    }
}
