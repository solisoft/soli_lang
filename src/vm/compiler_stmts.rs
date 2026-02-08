//! Statement compilation — AST statements to bytecode.

use std::rc::Rc;

use crate::ast::stmt::{FunctionDecl, ImportDecl, StmtKind};
use crate::ast::Stmt;

use super::chunk::Constant;
use super::compiler::{CompileResult, Compiler, FunctionType};
use super::opcode::Op;

impl Compiler {
    /// Compile a statement.
    pub fn compile_stmt(&mut self, stmt: &Stmt) -> CompileResult<()> {
        let line = stmt.span.line;
        match &stmt.kind {
            StmtKind::Expression(expr) => {
                self.compile_expr(expr)?;
                self.emit(Op::Pop, line);
            }
            StmtKind::Let { name, type_annotation: _, initializer } => {
                self.compile_let(name, initializer.as_ref(), false, line, stmt.span)?;
            }
            StmtKind::Const { name, type_annotation: _, initializer } => {
                self.compile_let(name, Some(initializer), true, line, stmt.span)?;
            }
            StmtKind::Block(stmts) => {
                self.begin_scope();
                for s in stmts {
                    self.compile_stmt(s)?;
                }
                self.end_scope(line);
            }
            StmtKind::If { condition, then_branch, else_branch } => {
                self.compile_if_stmt(condition, then_branch, else_branch.as_deref(), line)?;
            }
            StmtKind::While { condition, body } => {
                self.compile_while(condition, body, line)?;
            }
            StmtKind::For { variable, iterable, body } => {
                self.compile_for(variable, iterable, body, line)?;
            }
            StmtKind::Return(expr) => {
                if let Some(expr) = expr {
                    self.compile_expr(expr)?;
                } else {
                    self.emit(Op::Null, line);
                }
                self.emit(Op::Return, line);
            }
            StmtKind::Throw(expr) => {
                self.compile_expr(expr)?;
                self.emit(Op::Throw, line);
            }
            StmtKind::Try { try_block, catch_var, catch_block, finally_block } => {
                self.compile_try(try_block, catch_var.as_deref(), catch_block.as_deref(), finally_block.as_deref(), line)?;
            }
            StmtKind::Function(decl) => {
                self.compile_function_decl(decl, line)?;
            }
            StmtKind::Class(decl) => {
                self.compile_class_decl(decl, line)?;
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
        iterable: &crate::ast::Expr,
        body: &Stmt,
        line: usize,
    ) -> CompileResult<()> {
        // for x in iter { body }
        self.begin_scope();

        self.compile_expr(iterable)?;
        self.emit(Op::GetIter, line);

        let loop_start = self.current_offset();
        self.begin_loop(loop_start);
        let exit_jump = self.emit_jump(Op::ForIter(0), line);

        // Bind the loop variable
        self.add_local(variable.to_string(), false);

        self.compile_stmt(body)?;

        // Pop the loop variable
        self.emit(Op::Pop, line);
        self.locals.pop();

        self.emit_loop(loop_start, line);
        self.patch_jump(exit_jump);

        self.end_loop();

        // Pop the iterator
        self.emit(Op::Pop, line);

        self.end_scope(line);
        Ok(())
    }

    fn compile_try(
        &mut self,
        try_block: &Stmt,
        catch_var: Option<&str>,
        catch_block: Option<&Stmt>,
        finally_block: Option<&Stmt>,
        line: usize,
    ) -> CompileResult<()> {
        // Emit TryBegin with placeholder offsets
        let try_begin = self.emit(Op::TryBegin(0, 0), line);

        // Compile try body
        self.compile_stmt(try_block)?;
        self.emit(Op::TryEnd, line);

        // Jump over catch/finally if no exception
        let no_exception_jump = self.emit_jump(Op::Jump(0), line);

        // Patch catch offset
        let catch_start = self.current_offset();
        let catch_offset = catch_start - try_begin - 1;

        // Compile catch block
        if let Some(catch_body) = catch_block {
            self.begin_scope();
            if let Some(var_name) = catch_var {
                // The exception value is on the stack
                self.add_local(var_name.to_string(), false);
            }
            self.compile_stmt(catch_body)?;
            self.end_scope(line);
        } else {
            // No catch block — pop the exception
            self.emit(Op::Pop, line);
        }

        // Patch the finally offset
        let finally_start = self.current_offset();
        let finally_offset = finally_start - try_begin - 1;

        self.patch_jump(no_exception_jump);

        // Compile finally block
        if let Some(finally_body) = finally_block {
            self.compile_stmt(finally_body)?;
        }

        // Patch TryBegin offsets
        if let Op::TryBegin(ref mut co, ref mut fo) = self.proto.chunk.code[try_begin] {
            *co = catch_offset as u16;
            *fo = finally_offset as u16;
        }

        Ok(())
    }

    fn compile_function_decl(&mut self, decl: &FunctionDecl, line: usize) -> CompileResult<()> {
        let name = decl.name.clone();

        // Start compiling the function body
        let _dummy = self.start_function(FunctionType::Function, name.clone(), &decl.params);

        self.begin_scope();
        self.compile_function_body(&decl.body)?;
        self.end_scope(line);

        let proto = self.finish_function(line);
        let idx = self.add_constant(Constant::Function(Rc::new(proto)));
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

        let last_idx = body.len() - 1;
        for (i, stmt) in body.iter().enumerate() {
            if i == last_idx {
                // Last statement: if it's an expression, compile it without Pop
                // and emit Return so the value is returned implicitly
                if let StmtKind::Expression(expr) = &stmt.kind {
                    self.compile_expr(expr)?;
                    self.emit(Op::Return, stmt.span.line);
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
