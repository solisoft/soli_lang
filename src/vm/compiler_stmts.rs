//! Statement compilation — AST statements to bytecode.

use std::sync::Arc;

use crate::ast::stmt::{CatchClause, FunctionDecl, ImportDecl, StmtKind};
use crate::ast::Stmt;
use crate::error::CompileError;

use super::chunk::Constant;
use super::compiler::{CompileResult, Compiler, FunctionType};
use super::opcode::Op;

impl Compiler {
    /// Compile a statement.
    pub fn compile_stmt(&mut self, stmt: &Stmt) -> CompileResult<()> {
        // Every statement begins at the locals baseline (the previous statement
        // left the value stack holding exactly the live locals). Resync the
        // tracked height here so the comprehension clean-position gate is
        // correct regardless of any drift during the prior statement.
        self.resync_stack_height();
        let line = stmt.span.line as usize;
        match &stmt.kind {
            StmtKind::Expression(expr) => {
                self.compile_expr(expr)?;
                self.emit(Op::Pop, line);
            }
            StmtKind::Let {
                name,
                type_annotation: _,
                initializer,
            } => {
                self.compile_let(name, initializer.as_ref(), false, line, stmt.span)?;
            }
            StmtKind::Const {
                name,
                type_annotation: _,
                initializer,
            } => {
                self.compile_let(name, Some(initializer), true, line, stmt.span)?;
            }
            StmtKind::Block(stmts) => {
                self.begin_scope();
                for s in stmts {
                    self.compile_stmt(s)?;
                }
                self.end_scope(line);
            }
            StmtKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.compile_if_stmt(condition, then_branch, else_branch.as_deref(), line)?;
            }
            StmtKind::While { condition, body } => {
                self.compile_while(condition, body, line)?;
            }
            StmtKind::For {
                variable,
                index_variable,
                iterable,
                body,
            } => {
                self.compile_for(variable, index_variable.as_deref(), iterable, body, line)?;
            }
            StmtKind::Break => {
                // TODO: compile `break` natively. Doing it correctly means
                // unwinding, at the jump site, everything the loop body pushed:
                // body locals (Pop/CloseUpvalue above the loop's baseline), the
                // `for` iterator on `iter_stack`, and any live exception handler
                // when the `break` sits inside a `try`. Until that is in place,
                // refuse compilation so the handler falls back to the
                // tree-walking interpreter, which implements `break` fully.
                return Err(CompileError::new(
                    "`break` is not supported in compiled mode",
                    stmt.span,
                ));
            }
            StmtKind::Return(expr) => {
                if let Some(expr) = expr {
                    self.compile_expr(expr)?;
                } else {
                    self.emit(Op::Null, line);
                }
                // Every enclosing `finally` runs before the frame goes away.
                // The return value is already on the stack and each block is
                // stack-neutral (a block scopes and pops its own locals), so
                // `Op::Return` still pops the right value. Ruby's order too:
                // the value is computed first, then `ensure` runs.
                self.emit_pending_finallys(line)?;
                self.emit(Op::Return, line);
            }
            StmtKind::Throw(expr) => {
                self.compile_expr(expr)?;
                self.emit(Op::Throw, line);
            }
            StmtKind::Try {
                try_block,
                catch_clauses,
                finally_block,
            } => {
                self.compile_try(try_block, catch_clauses, finally_block.as_deref(), line)?;
            }
            StmtKind::Function(decl) => {
                self.compile_function_decl(decl, line)?;
            }
            StmtKind::Class(decl) => {
                self.compile_class_decl(decl, line)?;
            }
            StmtKind::Enum(decl) => {
                // Enums lower to an ordinary class the VM already compiles —
                // declaration, construction, and methods are all VM-native.
                self.compile_class_decl(&decl.lower_to_class(), line)?;
            }
            StmtKind::Interface(_) => {
                // Interfaces are type-only, no runtime representation needed
            }
            StmtKind::Import(decl) => {
                self.compile_import(decl, line)?;
            }
            StmtKind::Export(inner) => {
                // Export just compiles the inner statement — exports are handled at module level
                self.compile_stmt(inner)?;
            }
        }
        Ok(())
    }

    fn compile_let(
        &mut self,
        name: &str,
        initializer: Option<&crate::ast::Expr>,
        is_const: bool,
        line: usize,
        span: crate::span::Span,
    ) -> CompileResult<()> {
        if let Some(init) = initializer {
            self.compile_expr(init)?;
        } else {
            self.emit(Op::Null, line);
        }

        if self.scope_depth > 0 {
            // Local variable
            self.declare_variable(name, is_const, span)?;
            // The value is already on the stack at the right slot
        } else {
            // Global variable
            self.known_globals.borrow_mut().insert(name.to_string());
            let idx = self.add_string_constant(name);
            self.emit(Op::DefineGlobal(idx), line);
        }
        Ok(())
    }

    fn compile_if_stmt(
        &mut self,
        condition: &crate::ast::Expr,
        then_branch: &Stmt,
        else_branch: Option<&Stmt>,
        line: usize,
    ) -> CompileResult<()> {
        self.compile_expr(condition)?;
        let then_jump = self.emit_jump(Op::JumpIfFalse(0), line);

        self.compile_stmt(then_branch)?;

        if let Some(else_stmt) = else_branch {
            let else_jump = self.emit_jump(Op::Jump(0), line);
            self.patch_jump(then_jump);
            self.compile_stmt(else_stmt)?;
            self.patch_jump(else_jump);
        } else {
            self.patch_jump(then_jump);
        }
        Ok(())
    }

    fn compile_while(
        &mut self,
        condition: &crate::ast::Expr,
        body: &Stmt,
        line: usize,
    ) -> CompileResult<()> {
        let loop_start = self.current_offset();
        self.begin_loop(loop_start);

        self.compile_expr(condition)?;
        let exit_jump = self.emit_jump(Op::JumpIfFalse(0), line);

        self.compile_stmt(body)?;
        self.emit_loop(loop_start, line);
        self.patch_jump(exit_jump);

        self.end_loop();
        Ok(())
    }

    fn compile_for(
        &mut self,
        variable: &str,
        index_variable: Option<&str>,
        iterable: &crate::ast::Expr,
        body: &Stmt,
        line: usize,
    ) -> CompileResult<()> {
        // for x in iter { body } or for x, i in iter { body }
        self.begin_scope();

        // The index variable is a counter maintained by the compiler: `ForIter`
        // only yields the element value, never an index. Declare it *before* the
        // loop (so it persists across iterations) initialized to 0, and bump it
        // at the end of each iteration.
        let index_slot = if let Some(idx_var) = index_variable {
            self.emit_constant(Constant::Int(0), line);
            self.add_local(idx_var.to_string(), false);
            Some(
                self.resolve_local(idx_var)
                    .expect("index local just declared"),
            )
        } else {
            None
        };

        // Optimize: for x in a..b => GetIterRange + ForIterRange (zero allocation, inlined)
        let is_range = matches!(
            &iterable.kind,
            crate::ast::ExprKind::Binary {
                operator: crate::ast::BinaryOp::Range,
                ..
            }
        );
        if let crate::ast::ExprKind::Binary {
            left,
            operator: crate::ast::BinaryOp::Range,
            right,
        } = &iterable.kind
        {
            self.compile_expr(left)?;
            self.compile_expr(right)?;
            self.emit(Op::GetIterRange, line);
        } else {
            self.compile_expr(iterable)?;
            self.emit(Op::GetIter, line);
        }

        let loop_start = self.current_offset();
        self.begin_loop(loop_start);
        let exit_jump = if is_range {
            self.emit_jump(Op::ForIterRange(0), line)
        } else {
            self.emit_jump(Op::ForIter(0), line)
        };

        // Bind the loop variable to the freshly-yielded element value.
        self.add_local(variable.to_string(), false);

        self.compile_stmt(body)?;

        // Pop the loop variable (closing its upvalue if a body closure captured
        // it, so closures from different iterations don't share a binding).
        self.emit_pop_or_close_top(line);

        // Bump the index counter for the next iteration: idx = idx + 1.
        if let Some(slot) = index_slot {
            self.emit(Op::GetLocal(slot), line);
            self.emit_constant(Constant::Int(1), line);
            self.emit(Op::Add, line);
            self.emit(Op::SetLocal(slot), line);
            self.emit(Op::Pop, line);
        }

        self.emit_loop(loop_start, line);
        self.patch_jump(exit_jump);

        self.end_loop();

        // Pop the index counter (closing its upvalue if it was captured).
        if index_slot.is_some() {
            self.emit_pop_or_close_top(line);
        }

        // Note: no Pop needed for the iterator — GetIter moved it to iter_stack,
        // and ForIter pops it from iter_stack when exhausted.

        self.end_scope(line);
        Ok(())
    }

    /// Emit every enclosing `finally`, innermost first.
    ///
    /// While emitting block *i*, only the blocks outside it stay pending, so a
    /// `return` inside a `finally` runs the outer ones and not itself.
    fn emit_pending_finallys(&mut self, line: usize) -> CompileResult<()> {
        if self.finally_stack.is_empty() {
            return Ok(());
        }
        let _ = line;
        let pending = self.finally_stack.clone();
        for i in (0..pending.len()).rev() {
            let saved = std::mem::replace(&mut self.finally_stack, pending[..i].to_vec());
            let result = self.compile_stmt(&pending[i]);
            self.finally_stack = saved;
            result?;
        }
        Ok(())
    }

    /// `try`/`catch` with no `finally` — the original straight-line layout.
    fn compile_try_catch(
        &mut self,
        try_block: &Stmt,
        catch_clauses: &[CatchClause],
        line: usize,
    ) -> CompileResult<()> {
        let try_begin = self.emit(Op::TryBegin(0, 0), line);

        self.compile_stmt(try_block)?;
        self.emit(Op::TryEnd, line);

        let no_exception_jump = self.emit_jump(Op::Jump(0), line);

        let catch_start = self.current_offset();
        let catch_offset = catch_start - try_begin - 1;

        if catch_clauses.is_empty() {
            // No catch clause: discard the exception. Only reachable when
            // there is no `finally` either — with one, `compile_try` routes
            // the exception to the finally pad and rethrows instead.
            self.emit(Op::Pop, line);
        } else {
            let mut end_jumps = Vec::new();
            let mut next_clause_patches: Vec<usize> = Vec::new();

            for (i, clause) in catch_clauses.iter().enumerate() {
                for patch_idx in next_clause_patches.drain(..) {
                    let target = self.current_offset();
                    let jump_offset = target - patch_idx - 1;
                    if let Op::CatchMatch(_, ref mut off) = self.proto.chunk.code[patch_idx] {
                        *off = jump_offset as u16;
                    }
                }

                if let Some(ref type_name) = clause.type_name {
                    let name_idx = self.add_constant(Constant::String(type_name.clone().into()));
                    let catch_match_idx = self.emit(Op::CatchMatch(name_idx, 0), line);
                    next_clause_patches.push(catch_match_idx);
                }

                self.begin_scope();
                if let Some(ref var_name) = clause.var_name {
                    self.add_local(var_name.clone(), false);
                }
                self.compile_stmt(&clause.body)?;
                self.end_scope(line);

                if i < catch_clauses.len() - 1 {
                    end_jumps.push(self.emit_jump(Op::Jump(0), line));
                }
            }

            if !next_clause_patches.is_empty() {
                for patch_idx in next_clause_patches.drain(..) {
                    let target = self.current_offset();
                    let jump_offset = target - patch_idx - 1;
                    if let Op::CatchMatch(_, ref mut off) = self.proto.chunk.code[patch_idx] {
                        *off = jump_offset as u16;
                    }
                }
                self.emit(Op::Rethrow, line);
            }

            for j in end_jumps {
                self.patch_jump(j);
            }
        }

        let finally_start = self.current_offset();
        let finally_offset = finally_start - try_begin - 1;

        self.patch_jump(no_exception_jump);

        if let Op::TryBegin(ref mut co, ref mut fo) = self.proto.chunk.code[try_begin] {
            *co = catch_offset as u16;
            *fo = finally_offset as u16;
        }

        Ok(())
    }

    /// `try`/`catch`/`finally`.
    ///
    /// There is no runtime support for `finally` — `ExceptionHandler` carries a
    /// `finally_ip` that nothing ever reads — so the block is inlined on every
    /// edge that leaves the `try`. It used to be emitted only after the catch
    /// clauses, which meant it ran *only* when control fell off the end: a
    /// `return` skipped it (dropping the cleanup precisely when an early exit
    /// needed it) and, with no catch clause, the pending exception was popped
    /// and discarded.
    ///
    /// The layout wraps the ordinary try/catch in a second handler:
    ///
    /// ```text
    ///   TryBegin(PAD)          outer: anything the catch clauses do not take
    ///     <try/catch as usual> inner handler is pushed and popped inside here
    ///   PopHandler             normal path: drop the outer handler…
    ///   <finally>              …and run the block
    ///   Jump END
    /// PAD:                     exception in flight, its value on the stack
    ///   <finally>
    ///   Rethrow
    /// END:
    /// ```
    ///
    /// The inner handler is pushed *after* the outer one, so it wins for an
    /// exception raised in the try body; `throw_exception` pops it on the way
    /// to the catch clause, which leaves the outer one live for the duration of
    /// that clause — so a throw from inside `catch` reaches the pad too. The
    /// `return` edge is handled by `emit_pending_finallys`, and `break`/`next`
    /// are still refused, so they cannot skip a block.
    fn compile_try(
        &mut self,
        try_block: &Stmt,
        catch_clauses: &[CatchClause],
        finally_block: Option<&Stmt>,
        line: usize,
    ) -> CompileResult<()> {
        let Some(finally_body) = finally_block else {
            return self.compile_try_catch(try_block, catch_clauses, line);
        };

        let outer = self.emit(Op::TryBegin(0, 0), line);

        self.finally_stack.push(finally_body.clone());
        let inner = if catch_clauses.is_empty() {
            // No catch clause, so no inner handler: an exception must reach the
            // pad, run the block and carry on unwinding. Registering one would
            // hand the exception to `compile_try_catch`'s empty-clause path,
            // which pops and discards it — that is how a throw through a
            // `try`/`finally` used to vanish.
            self.compile_stmt(try_block)
        } else {
            self.compile_try_catch(try_block, catch_clauses, line)
        };
        self.finally_stack.pop();
        inner?;

        // Normal path: the outer handler is no longer wanted.
        self.emit(Op::PopHandler, line);
        self.compile_stmt(finally_body)?;
        let end_jump = self.emit_jump(Op::Jump(0), line);

        // Exception path. `throw_exception` truncated the stack to the depth
        // recorded at TryBegin and pushed the value, so it sits exactly where
        // the next local would go — bind it as one, or the finally block's own
        // locals are allocated one slot low.
        let pad = self.current_offset();
        let pad_offset = pad - outer - 1;
        self.begin_scope();
        self.add_local(String::new(), false);
        let exc_slot = (self.locals.len() - 1) as u16;
        self.compile_stmt(finally_body)?;
        self.emit(Op::GetLocal(exc_slot), line);
        self.emit(Op::Rethrow, line);
        // Rethrow always leaves; end_scope's pops are unreachable but keep the
        // compiler's local bookkeeping straight for whatever follows.
        self.end_scope(line);

        self.patch_jump(end_jump);

        if let Op::TryBegin(ref mut co, ref mut fo) = self.proto.chunk.code[outer] {
            *co = pad_offset as u16;
            *fo = pad_offset as u16;
        }

        Ok(())
    }

    fn compile_function_decl(&mut self, decl: &FunctionDecl, line: usize) -> CompileResult<()> {
        let name = decl.name.clone();

        // A top-level function declaration defines a global of that name. Record
        // it before compiling the body so the body (and any nested functions)
        // resolve a bare assignment to this name as the global, not a new local.
        if self.scope_depth == 0 {
            self.known_globals.borrow_mut().insert(name.clone());
        }

        // Start compiling the function body
        let _dummy = self.start_function(FunctionType::Function, name.clone(), &decl.params);

        self.begin_scope();
        self.emit_param_defaults(&decl.params)?;
        self.compile_function_body(&decl.body)?;
        self.end_scope(line);

        let proto = self.finish_function(line);
        let idx = self.add_constant(Constant::Function(Arc::new(proto)));
        self.emit(Op::Closure(idx), line);

        // Bind the function name
        if self.scope_depth > 0 {
            self.add_local(name, false);
        } else {
            let name_idx = self.add_string_constant(&decl.name);
            self.emit(Op::DefineGlobal(name_idx), line);
        }
        Ok(())
    }

    /// Compile a function body with implicit return support.
    /// If the last statement is an expression, its value is kept on the stack
    /// (not popped) and returned implicitly, matching tree-walking interpreter behavior.
    pub fn compile_function_body(&mut self, body: &[Stmt]) -> CompileResult<()> {
        if body.is_empty() {
            return Ok(());
        }

        // Declare locals introduced by bare assignment (optional-`let`) up front.
        self.hoist_locals(body, body[0].span.line as usize);

        let last_idx = body.len() - 1;
        for (i, stmt) in body.iter().enumerate() {
            if i == last_idx {
                // Last statement: if it's an expression, compile it without Pop
                // and emit Return so the value is returned implicitly
                if let StmtKind::Expression(expr) = &stmt.kind {
                    self.compile_expr(expr)?;
                    self.emit(Op::Return, stmt.span.line as usize);
                    return Ok(());
                }
            }
            self.compile_stmt(stmt)?;
        }
        Ok(())
    }

    fn compile_import(&mut self, decl: &ImportDecl, line: usize) -> CompileResult<()> {
        let idx = self.add_string_constant(&decl.path);
        self.emit(Op::Import(idx), line);
        Ok(())
    }
}
