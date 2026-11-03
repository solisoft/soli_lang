//! Expression compilation — AST expressions to bytecode.

use std::sync::Arc;

use crate::ast::expr::{Argument, BinaryOp, ExprKind, InterpolatedPart, UnaryOp};
use crate::ast::stmt::StmtKind;
use crate::ast::{Expr, Stmt};
use crate::error::CompileError;

use super::chunk::Constant;
use super::compiler::{CompileResult, Compiler, FunctionType, VariableAccess};
use super::opcode::Op;

impl Compiler {
    /// Compile an expression — the result is left on the stack.
    ///
    /// Every instruction it emits records the expression's column, so an
    /// error raised by one of them points where the tree-walker's would. A
    /// sub-expression sets its own and this one's comes back after it.
    pub fn compile_expr(&mut self, expr: &Expr) -> CompileResult<()> {
        if self.template_mode {
            if let Some(local) = template_ivar(expr) {
                return self.compile_expr(&local);
            }
            if matches!(expr.kind, ExprKind::This) {
                self.emit(Op::Null, expr.span.line as usize);
                return Ok(());
            }
        }
        let outer = std::mem::replace(&mut self.column, expr.span.column);
        let result = self.compile_expr_at(expr);
        self.column = outer;
        result
    }

    fn compile_expr_at(&mut self, expr: &Expr) -> CompileResult<()> {
        let line = expr.span.line as usize;
        self.cover_line(line);
        match &expr.kind {
            ExprKind::IntLiteral(n) => {
                self.emit_constant(Constant::Int(*n), line);
            }
            ExprKind::FloatLiteral(n) => {
                self.emit_constant(Constant::Float(*n), line);
            }
            ExprKind::DecimalLiteral(s) => {
                self.emit_constant(Constant::Decimal(s.clone()), line);
            }
            ExprKind::StringLiteral(s) => {
                self.emit_constant(Constant::String(s.clone().into()), line);
            }
            ExprKind::CommandSubstitution(cmd) => {
                // Same as `System.shell(cmd)`: `sh -c` plus a Future result.
                // Property access on that Future auto-resolves (see
                // `Vm::force_lazy_receiver`), matching the tree-walker.
                let sys = self.add_string_constant("System");
                self.emit(Op::GetGlobal(sys), line);
                self.emit_constant(Constant::String(cmd.clone().into()), line);
                let shell = self.add_string_constant("shell");
                self.emit(Op::CallMethod(shell, 1), line);
            }
            ExprKind::BoolLiteral(b) => {
                self.emit(if *b { Op::True } else { Op::False }, line);
            }
            ExprKind::Symbol(s) => {
                let idx = self.add_string_constant(s);
                self.emit(Op::Symbol(idx), line);
            }
            ExprKind::Null => {
                self.emit(Op::Null, line);
            }
            ExprKind::Variable(name) => {
                self.compile_variable_get(name, line)?;
                if self.calls_bare_builtin(name) {
                    self.emit(Op::Call(0), line);
                }
            }
            ExprKind::Binary {
                left,
                operator,
                right,
            } => match self.folded(expr) {
                Some(value) => self.emit_constant(value, line),
                None => self.compile_binary(left, *operator, right, line)?,
            },
            ExprKind::Unary { operator, operand } => match self.folded(expr) {
                Some(value) => self.emit_constant(value, line),
                None => self.compile_unary(*operator, operand, line)?,
            },
            ExprKind::Grouping(inner) => {
                self.compile_expr(inner)?;
            }
            ExprKind::Call { callee, arguments } => {
                self.compile_call(callee, arguments, line)?;
            }
            ExprKind::Pipeline { left, right } => {
                self.compile_pipeline(left, right, line)?;
            }
            ExprKind::Member { object, name } => {
                self.compile_expr(object)?;
                let op = self.get_property_op(name);
                self.emit(op, line);
            }
            ExprKind::SafeMember { object, name } => {
                // `a&.b` — null receiver yields null, otherwise an ordinary
                // member read. `JumpIfNull` *peeks*, so the null already on the
                // stack is the result and nothing has to be pushed back.
                //
                // This was refused before (and before that it was
                // `unimplemented!()`, which panicked — with panic="abort" in
                // release, core-dumping the server the moment a handler used
                // `&.`). Refusing was safe but demoted the whole handler.
                self.compile_expr(object)?;
                let skip = self.emit_jump(Op::JumpIfNull(0), line);
                let op = self.get_property_op(name);
                self.emit(op, line);
                self.patch_jump(skip);
            }
            ExprKind::QualifiedName { qualifier, name } => {
                self.compile_expr(qualifier)?;
                let op = self.get_property_op(name);
                self.emit(op, line);
            }
            ExprKind::Index { object, index } => {
                // `req["params"]["id"]` — two literal keys on a variable.
                // One dispatch instead of HashGetLocalConst + HashGetConst.
                if let ExprKind::StringLiteral(outer_key) = &index.kind {
                    if self.try_emit_nested_const_index(object, outer_key, line)? {
                        return Ok(());
                    }
                }
                // Peephole: hash[const_str] → HashGetConst (avoids the generic
                // GetIndex path and its key-allocation).
                if let ExprKind::StringLiteral(key) = &index.kind {
                    let key_idx = self.add_string_constant(key);
                    // Fuse the receiver load when it is a plain variable, so
                    // `params["name"]` — the most common hash read in real
                    // controller code — costs one dispatch instead of two.
                    // `.get("name")` already compiled this way; index syntax did
                    // not, which made the idiomatic spelling the slower one
                    // (measured 79 ns/iter vs 63 ns for `.get`).
                    //
                    // An upvalue receiver has no fused opcode and falls through
                    // to the generic path — emitting the global form for it
                    // would read the wrong binding.
                    let fused = match &object.kind {
                        ExprKind::Variable(var_name) => match self.resolve_variable(var_name) {
                            VariableAccess::Local(slot) => {
                                Some(Op::HashGetLocalConst(slot, key_idx))
                            }
                            VariableAccess::Global(_) => {
                                let gidx = self.add_string_constant(var_name);
                                Some(Op::HashGetGlobalConst(gidx, key_idx))
                            }
                            VariableAccess::Upvalue(_) => None,
                        },
                        _ => None,
                    };
                    match fused {
                        Some(op) => {
                            self.emit(op, line);
                        }
                        None => {
                            self.compile_expr(object)?;
                            self.emit(Op::HashGetConst(key_idx), line);
                        }
                    }
                } else {
                    self.compile_expr(object)?;
                    self.compile_expr(index)?;
                    self.emit(Op::GetIndex, line);
                }
            }
            ExprKind::This => {
                self.compile_this(line)?;
            }
            ExprKind::Super => {
                self.compile_super(line)?;
            }
            ExprKind::New {
                class_expr,
                arguments,
            } => {
                self.compile_new(class_expr, arguments, line)?;
            }
            ExprKind::Array(elements) => {
                self.compile_array(elements, line)?;
            }
            ExprKind::Hash(pairs) => {
                self.compile_hash(pairs, line)?;
            }
            ExprKind::Block(stmts) => {
                self.compile_block_expr(stmts, line)?;
            }
            ExprKind::Assign { target, value } => {
                self.compile_assign(target, value, line)?;
            }
            ExprKind::LogicalAnd { left, right } => {
                self.compile_expr(left)?;
                let jump = self.emit_jump(Op::JumpIfFalseNoPop(0), line);
                self.emit(Op::Pop, line);
                self.compile_expr(right)?;
                self.patch_jump(jump);
            }
            ExprKind::LogicalOr { left, right } => {
                self.compile_expr(left)?;
                let jump = self.emit_jump(Op::JumpIfTrueNoPop(0), line);
                self.emit(Op::Pop, line);
                self.compile_expr(right)?;
                self.patch_jump(jump);
            }
            ExprKind::NullishCoalescing { left, right } => {
                self.compile_expr(left)?;
                let jump = self.emit_jump(Op::NullishJump(0), line);
                self.emit(Op::Pop, line);
                self.compile_expr(right)?;
                self.patch_jump(jump);
            }
            ExprKind::Lambda {
                params,
                return_type: _,
                body,
            } => {
                self.compile_lambda(params, body, line)?;
            }
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.compile_if_expr(condition, then_branch, else_branch.as_deref(), line)?;
            }
            ExprKind::Match { expression, arms } => {
                self.compile_match(expression, arms, line)?;
            }
            ExprKind::ListComprehension {
                element,
                variable,
                iterable,
                condition,
            } => {
                self.compile_list_comprehension(
                    element,
                    variable,
                    iterable,
                    condition.as_deref(),
                    line,
                )?;
            }
            ExprKind::HashComprehension {
                key,
                value,
                variable,
                iterable,
                condition,
            } => {
                self.compile_hash_comprehension(
                    key,
                    value,
                    variable,
                    iterable,
                    condition.as_deref(),
                    line,
                )?;
            }
            ExprKind::InterpolatedString(parts) => {
                self.compile_interpolated_string(parts, line)?;
            }
            ExprKind::SdqlBlock {
                query,
                interpolations,
            } => {
                self.compile_sdql_block(query, interpolations, line)?;
            }
            ExprKind::Spread(_) => {
                // Only an array literal holds a spread (`compile_array`); the
                // parser produces it nowhere else.
                return Err(CompileError::new(
                    "spread (...) is only valid inside an array literal",
                    crate::span::Span::new(0, 0, line, 0),
                ));
            }
            ExprKind::Throw(inner) => {
                self.compile_expr(inner)?;
                self.emit(Op::Throw, line);
            }
            ExprKind::Rescue { expr, fallback } => {
                let try_begin = self.emit(Op::TryBegin(0, 0), line);
                self.compile_expr(expr)?;
                // An async builtin (`HTTP.request`, `HTTP.get`…) hands back a
                // Future whose failure would only surface at the first read,
                // past this rescue. Settle it while the handler is active.
                self.emit(Op::ForceFuture, line);
                self.emit(Op::TryEnd, line);
                let rescue_jump = self.emit(Op::Jump(0), line);
                // Catch target: the Pop that discards the pushed exception
                // value, then the fallback expression. The offset is measured
                // from the instruction after TryBegin (see the TryBegin
                // handler) — it used to be computed after the fallback, which
                // landed the handler past it and made the exception value the
                // rescue result.
                let catch_start = self.current_offset();
                self.emit(Op::Pop, line);
                self.compile_expr(fallback)?;
                self.patch_jump(rescue_jump);
                if let Op::TryBegin(ref mut co, _) = self.proto.chunk.code[try_begin] {
                    *co = (catch_start - try_begin - 1) as u16;
                }
            }
            ExprKind::CompoundAssign {
                target,
                operator,
                value,
            } => {
                // Desugar: target op= value  →  target = target op value.
                // For short-circuit operators (||=, &&=, ??=) we use the
                // matching short-circuit binary node so the existing VM logic
                // skips the RHS when the left operand already short-circuits.
                use crate::ast::expr::CompoundOp;

                // `obj.x ||= v` on a property nothing has set yet: the read
                // answers Null instead of raising, or `@items ||= []` could
                // never initialise an instance variable.
                //
                // A short-circuit leaves the read value as the result and
                // writes nothing, as on the tree-walker: `@total ||= 0` with
                // `@total` set is one read, and a frozen value object or a
                // getter (`b.label ||= "x"`) is not written back. Only the
                // assigning path evaluates the receiver a second time, as
                // `assign_to_target` does there:
                //
                //   obj; GetPropertyOrNull; jump-if-decided END;
                //   Pop; obj; value; SetProperty; END:
                if let ExprKind::Member { object, name } = &target.kind {
                    // `JumpIfNotNull` rather than `NullishJump` for `??=`:
                    // the same test, and the fast tier runs it.
                    let jump_op = match operator {
                        CompoundOp::Or => Some(Op::JumpIfTrueNoPop(0)),
                        CompoundOp::And => Some(Op::JumpIfFalseNoPop(0)),
                        CompoundOp::Coalesce => Some(Op::JumpIfNotNull(0)),
                        _ => None,
                    };
                    if let Some(jump_op) = jump_op {
                        let idx = self.add_string_constant(name);
                        self.compile_expr(object)?;
                        self.emit(Op::GetPropertyOrNull(idx), line);
                        let jump = self.emit_jump(jump_op, line);
                        self.emit(Op::Pop, line);
                        self.compile_expr(object)?;
                        self.compile_expr(value)?;
                        self.emit(Op::SetProperty(idx), line);
                        self.patch_jump(jump);
                        return Ok(());
                    }
                }

                let desugared_value = match operator {
                    CompoundOp::Or => Expr::new(
                        ExprKind::LogicalOr {
                            left: Box::new((**target).clone()),
                            right: Box::new((**value).clone()),
                        },
                        expr.span,
                    ),
                    CompoundOp::And => Expr::new(
                        ExprKind::LogicalAnd {
                            left: Box::new((**target).clone()),
                            right: Box::new((**value).clone()),
                        },
                        expr.span,
                    ),
                    CompoundOp::Coalesce => Expr::new(
                        ExprKind::NullishCoalescing {
                            left: Box::new((**target).clone()),
                            right: Box::new((**value).clone()),
                        },
                        expr.span,
                    ),
                    _ => {
                        let bin_op = match operator {
                            CompoundOp::Add => BinaryOp::Add,
                            CompoundOp::Subtract => BinaryOp::Subtract,
                            CompoundOp::Multiply => BinaryOp::Multiply,
                            CompoundOp::Divide => BinaryOp::Divide,
                            CompoundOp::Modulo => BinaryOp::Modulo,
                            CompoundOp::Or | CompoundOp::And | CompoundOp::Coalesce => {
                                unreachable!()
                            }
                        };
                        Expr::new(
                            ExprKind::Binary {
                                left: Box::new((**target).clone()),
                                operator: bin_op,
                                right: Box::new((**value).clone()),
                            },
                            expr.span,
                        )
                    }
                };
                self.compile_assign(target, &desugared_value, line)?;
            }
            ExprKind::PostfixIncrement(target) => {
                // Compile: push old value, then assign target = target + 1
                self.compile_expr(target)?; // old value on stack (return value)
                let one = Expr::new(ExprKind::IntLiteral(1), expr.span);
                let new_val = Expr::new(
                    ExprKind::Binary {
                        left: Box::new((**target).clone()),
                        operator: BinaryOp::Add,
                        right: Box::new(one),
                    },
                    expr.span,
                );
                self.compile_assign(target, &new_val, line)?;
                self.emit(Op::Pop, line); // pop the assign result, keeping old value
            }
            ExprKind::PostfixDecrement(target) => {
                self.compile_expr(target)?;
                let one = Expr::new(ExprKind::IntLiteral(1), expr.span);
                let new_val = Expr::new(
                    ExprKind::Binary {
                        left: Box::new((**target).clone()),
                        operator: BinaryOp::Subtract,
                        right: Box::new(one),
                    },
                    expr.span,
                );
                self.compile_assign(target, &new_val, line)?;
                self.emit(Op::Pop, line);
            }
        }
        Ok(())
    }

    fn compile_variable_get(&mut self, name: &str, line: usize) -> CompileResult<()> {
        match self.resolve_variable(name) {
            VariableAccess::Local(slot) => {
                self.emit(Op::GetLocal(slot), line);
            }
            VariableAccess::Upvalue(idx) => {
                self.emit(Op::GetUpvalue(idx), line);
            }
            VariableAccess::Global(name) => {
                // `debug()` signals through a sentinel `Value` that the
                // tree-walker recognises in a statement position and this
                // engine has no handling for: it evaluated the call and popped
                // the result, so the breakpoint never fired. Refuse it, so the
                // handler falls back to the interpreter — which also supplies
                // the captured environment the REPL needs.
                //
                // `next` was the other sentinel. It is compiled natively now
                // (see `is_builtin_next`), as the jump it means, so it is
                // refused *here* only in an expression position — `let x =
                // next` — where it is not control flow and there is nothing
                // sensible to emit.
                //
                // Reached only for a *global*, and a user-declared one is in
                // `known_globals`, so a program's own `let debug = 42`
                // compiles. Without that check a plausible variable name would
                // demote a handler for no reason.
                if (name == "next" || name == "debug")
                    && !self.program_globals.borrow().contains(name.as_str())
                {
                    return Err(CompileError::new(
                        format!("`{name}` is not supported in compiled mode"),
                        crate::span::Span::new(0, 0, line, 1),
                    ));
                }
                let idx = self.add_string_constant(&name);
                if self.implicit_self_candidate(&name) {
                    // A global of that name if one exists at run time, else
                    // the method on `this` read like `@name`.
                    self.compile_this(line)?;
                    self.emit(Op::SelfOrGlobal(idx), line);
                } else {
                    self.emit(Op::GetGlobal(idx), line);
                }
            }
        }
        Ok(())
    }

    /// Whether a bare read of `name` calls it: one of
    /// [`BARE_CALL_BUILTINS`](crate::interpreter::builtins::BARE_CALL_BUILTINS),
    /// resolving to the global builtin — no local, upvalue or program-defined
    /// global of that name.
    fn calls_bare_builtin(&mut self, name: &str) -> bool {
        crate::interpreter::builtins::is_bare_call_builtin(name)
            && matches!(self.resolve_variable(name), VariableAccess::Global(_))
            && !self.program_globals.borrow().contains(name)
    }

    /// Inside an instance method, or a block within one: could the bare `name`
    /// — no local, no upvalue, no global the compiler knows — be a method of
    /// `this`? Known globals (in `serve`, the worker's whole global table) and
    /// capitalised names (classes) keep their plain global read.
    fn implicit_self_candidate(&mut self, name: &str) -> bool {
        if self.class_context.is_none()
            || name == "this"
            || name.starts_with(|c: char| c.is_ascii_uppercase())
            || self.is_known_global(name)
        {
            return false;
        }
        if self.function_type == FunctionType::Method
            || self.function_type == FunctionType::Constructor
        {
            return true;
        }
        !matches!(self.resolve_variable("this"), VariableAccess::Global(_))
    }

    fn compile_binary(
        &mut self,
        left: &Expr,
        op: BinaryOp,
        right: &Expr,
        line: usize,
    ) -> CompileResult<()> {
        // `name << value` writes the variable back. A string has no `Rc`
        // identity, so the generic ArrayPush (which edits the value sitting on
        // the stack) would append to a copy and leave the variable behind. The
        // variable is still read first, as for any binary operator.
        if op == BinaryOp::Shovel {
            if let ExprKind::Variable(name) = &left.kind {
                let append = match self.resolve_variable(name) {
                    VariableAccess::Local(slot) => Op::AppendLocal(slot),
                    VariableAccess::Upvalue(idx) => Op::AppendUpvalue(idx),
                    VariableAccess::Global(global_name) => {
                        Op::AppendGlobal(self.add_string_constant(&global_name))
                    }
                };
                self.compile_expr(left)?;
                self.compile_expr(right)?;
                self.emit(append, line);
                return Ok(());
            }
        }
        self.compile_expr(left)?;
        self.compile_expr(right)?;
        match op {
            BinaryOp::Add => self.emit(Op::Add, line),
            BinaryOp::Subtract => self.emit(Op::Subtract, line),
            BinaryOp::Multiply => self.emit(Op::Multiply, line),
            BinaryOp::Divide => self.emit(Op::Divide, line),
            BinaryOp::Modulo => self.emit(Op::Modulo, line),
            BinaryOp::Equal => self.emit(Op::Equal, line),
            BinaryOp::NotEqual => self.emit(Op::NotEqual, line),
            BinaryOp::Less => self.emit(Op::Less, line),
            BinaryOp::LessEqual => self.emit(Op::LessEqual, line),
            BinaryOp::Greater => self.emit(Op::Greater, line),
            BinaryOp::GreaterEqual => self.emit(Op::GreaterEqual, line),
            BinaryOp::Range => self.emit(Op::Range, line),
            BinaryOp::Shovel => self.emit(Op::ArrayPush, line),
        };
        Ok(())
    }

    fn compile_unary(&mut self, op: UnaryOp, operand: &Expr, line: usize) -> CompileResult<()> {
        self.compile_expr(operand)?;
        match op {
            UnaryOp::Negate => self.emit(Op::Negate, line),
            UnaryOp::Not => self.emit(Op::Not, line),
        };
        Ok(())
    }

    /// Write a view's output: `arg`, escaped or raw, to the page. The ops push
    /// nothing; a literal raw text is written from its constant.
    pub(super) fn emit_template_write(
        &mut self,
        arg: &Expr,
        escaped: bool,
        line: usize,
    ) -> CompileResult<()> {
        if let (ExprKind::StringLiteral(text), false) = (&arg.kind, escaped) {
            let idx = self.add_constant(Constant::String(text.as_str().into()));
            self.emit(Op::TemplateWriteConst(idx), line);
            return Ok(());
        }
        self.compile_expr(arg)?;
        self.emit(Op::TemplateWrite(escaped), line);
        Ok(())
    }

    fn compile_call(
        &mut self,
        callee: &Expr,
        arguments: &[Argument],
        line: usize,
    ) -> CompileResult<()> {
        if self.template_mode {
            if let Some(local) = template_ivar(callee) {
                return self.compile_call(&local, arguments, line);
            }
        }
        // Special case: print() calls
        if let ExprKind::Variable(name) = &callee.kind {
            if name == "print" || name == "puts" || name == "println" {
                return self.compile_print(arguments, line);
            }
            // A compiled template's output: `__tpl_write(expr)` (escaped) and
            // `__tpl_write_raw(expr)`. Only the template compiler emits them.
            // A compiled template's captures (component and content_for
            // blocks): `__tpl_capture_start()`, `__tpl_capture_end()`,
            // `__tpl_component(name, props, body)`, `__tpl_content_for(name, body)`,
            // `__tpl_partial(name, data)`.
            let template_op = match (name.as_str(), arguments.len()) {
                ("__tpl_capture_start", 0) => Some(Op::TemplateCaptureStart),
                ("__tpl_capture_end", 0) => Some(Op::TemplateCaptureEnd),
                ("__tpl_component", 3) => Some(Op::TemplateComponent),
                ("__tpl_partial", 2) => Some(Op::TemplatePartial),
                ("__tpl_content_for", 2) => Some(Op::TemplateContentFor),
                ("__tpl_yield", 1) => Some(Op::TemplateYield),
                ("__tpl_yield_named", 1) => Some(Op::TemplateYieldNamed),
                _ => None,
            };
            if let Some(op) = template_op {
                if arguments
                    .iter()
                    .all(|a| matches!(a, Argument::Positional(_)))
                {
                    for arg in arguments {
                        if let Argument::Positional(expr) = arg {
                            self.compile_expr(expr)?;
                        }
                    }
                    self.emit(op, line);
                    return Ok(());
                }
            }
            if name == "__tpl_iter" {
                if let [Argument::Positional(arg)] = arguments {
                    self.compile_expr(arg)?;
                    self.emit(Op::TemplateIter, line);
                    return Ok(());
                }
            }
            if let Some((arg, escaped)) = template_write_args(name, arguments) {
                // Used as a value (a lambda's last statement, `x = …`): the
                // write pushes nothing, so the call's value is pushed here. A
                // write statement skips both (`compile_stmt`).
                self.emit_template_write(arg, escaped, line)?;
                self.emit(Op::Null, line);
                return Ok(());
            }
        }

        // Special case: JSON.parse() and JSON.stringify()
        if let ExprKind::Member { object, name } = &callee.kind {
            if let ExprKind::Variable(obj_name) = &object.kind {
                if obj_name == "JSON" {
                    if name == "parse" {
                        return self.compile_json_parse(arguments, line);
                    } else if name == "stringify" {
                        return self.compile_json_stringify(arguments, line);
                    }
                }
            }
        }

        // `super(...)` / `super.method(...)` — dispatch against the
        // *defining* class's superclass at runtime (the CallSuper* handlers
        // read the class recorded on the call frame). `this` goes in the
        // callee slot, per the method calling convention.
        if matches!(callee.kind, ExprKind::Super) {
            self.compile_this(line)?;
            let mut argc = 0u8;
            for arg in arguments {
                match arg {
                    Argument::Positional(expr) => {
                        self.compile_expr(expr)?;
                        argc += 1;
                    }
                    _ => {
                        return Err(CompileError::new(
                            "super(...) supports positional arguments only",
                            callee.span,
                        ))
                    }
                }
            }
            self.emit(Op::CallSuperInit(argc), line);
            return Ok(());
        }
        if let ExprKind::Member { object, name } = &callee.kind {
            if matches!(object.kind, ExprKind::Super) {
                self.compile_this(line)?;
                let mut argc = 0u8;
                for arg in arguments {
                    match arg {
                        Argument::Positional(expr) => {
                            self.compile_expr(expr)?;
                            argc += 1;
                        }
                        _ => {
                            return Err(CompileError::new(
                                "super.method(...) supports positional arguments only",
                                callee.span,
                            ))
                        }
                    }
                }
                let name_idx = self.add_string_constant(name);
                self.emit(Op::CallSuperMethod(name_idx, argc), line);
                return Ok(());
            }
        }

        // A method call with a labelled argument (`obj.m(k: v)`, `@m(k: v)`,
        // `obj&.m(k: v)`, the bare helper `m(k: v)` in a method) keeps its
        // receiver: `CallMethodNamed` binds the labels against the method it
        // resolves at run time, with the receiver as `this`. Reading `obj.m`
        // first and calling the result lost the receiver — and ran a method of
        // arity 0 on the read.
        if arguments.iter().any(|a| matches!(a, Argument::Named(_))) && arguments.len() <= 255 {
            match &callee.kind {
                ExprKind::Member { object, name } => {
                    self.compile_expr(object)?;
                    let (argc, names_idx) = self.compile_labelled_method_arguments(arguments)?;
                    let name_idx = self.add_string_constant(name);
                    self.emit(Op::CallMethodNamed(name_idx, argc, names_idx), line);
                    return Ok(());
                }
                ExprKind::SafeMember { object, name } => {
                    // A nil receiver evaluates no argument, as positionally.
                    self.compile_expr(object)?;
                    let skip = self.emit_jump(Op::JumpIfNull(0), line);
                    let (argc, names_idx) = self.compile_labelled_method_arguments(arguments)?;
                    let name_idx = self.add_string_constant(name);
                    self.emit(Op::CallMethodNamed(name_idx, argc, names_idx), line);
                    self.patch_jump(skip);
                    return Ok(());
                }
                ExprKind::Variable(name)
                    if matches!(self.resolve_variable(name), VariableAccess::Global(_))
                        && self.implicit_self_candidate(name) =>
                {
                    let idx = self.add_string_constant(name);
                    self.compile_this(line)?;
                    self.emit(Op::SelfOrGlobalCallee(idx), line);
                    let (argc, names_idx) = self.compile_labelled_method_arguments(arguments)?;
                    self.emit(Op::CallSelfOrGlobalNamed(idx, argc, names_idx), line);
                    return Ok(());
                }
                _ => {}
            }
        }

        // `a&.foo(args)` — a call whose callee is a safe member. A null
        // receiver yields null *without evaluating the arguments*, which is
        // what the tree-walker does and what Ruby does: `nil&.foo(bar())`
        // never runs `bar()`. Emitting the arguments before the null test
        // would run them.
        if let ExprKind::SafeMember { object, name } = &callee.kind {
            let all_positional = arguments
                .iter()
                .all(|a| matches!(a, Argument::Positional(_)));
            if all_positional && arguments.len() <= 255 {
                self.compile_expr(object)?;
                let skip = self.emit_jump(Op::JumpIfNull(0), line);
                let mut argc = 0u8;
                for arg in arguments {
                    if let Argument::Positional(expr) = arg {
                        self.compile_expr(expr)?;
                        argc += 1;
                    }
                }
                let name_idx = self.add_string_constant(name);
                let method_id = super::method_table::resolve_method_id(name);
                if method_id != super::method_table::METHOD_UNKNOWN {
                    self.emit(Op::CallMethodById(name_idx, argc, method_id), line);
                } else {
                    self.emit(Op::CallMethod(name_idx, argc), line);
                }
                self.patch_jump(skip);
                return Ok(());
            }
        }

        // Optimized path: method calls (obj.method(args)) use CallMethod opcode
        // to avoid allocating Value::Method intermediary.
        if let ExprKind::Member { object, name } = &callee.kind {
            let all_positional = arguments
                .iter()
                .all(|a| matches!(a, Argument::Positional(_) | Argument::Block(_)));
            if all_positional && arguments.len() <= 255 {
                // `["up", "late"].includes?(status)`: the argument tested
                // against a constant list, with no array built
                // (`compiler_literal_arrays.rs`).
                if self.try_compile_literal_membership(object, name, arguments, line)? {
                    return Ok(());
                }
                if let Some(op) =
                    self.try_compile_hash_const_string_call(object, name, arguments, line)?
                {
                    return Ok(op);
                }
                // Special optimization: arr.push(x) -> ArrayPush opcode
                // This avoids method dispatch overhead for the common push operation
                if name == "push" && arguments.len() == 1 {
                    if let Argument::Positional(expr) = &arguments[0] {
                        self.compile_expr(object)?;
                        self.compile_expr(expr)?;
                        self.emit(Op::ArrayPush, line);
                        return Ok(());
                    }
                }
                self.compile_expr(object)?;
                let mut argc = 0u8;
                for arg in arguments {
                    match arg {
                        Argument::Positional(expr) | Argument::Block(expr) => {
                            self.compile_expr(expr)?;
                            argc += 1;
                        }
                        Argument::Named(_) => unreachable!(),
                    }
                }
                let name_idx = self.add_string_constant(name);
                let method_id = super::method_table::resolve_method_id(name);
                if method_id != super::method_table::METHOD_UNKNOWN {
                    self.emit(Op::CallMethodById(name_idx, argc, method_id), line);
                } else {
                    self.emit(Op::CallMethod(name_idx, argc), line);
                }
                return Ok(());
            }
        }

        // `name(args)` inside an instance method, where `name` may be a method
        // of `this` rather than a global: the slot below the arguments holds
        // the global or, failing one, `this` as the receiver.
        if let ExprKind::Variable(name) = &callee.kind {
            let positional = arguments
                .iter()
                .all(|a| matches!(a, Argument::Positional(_) | Argument::Block(_)));
            if positional
                && arguments.len() <= 255
                && matches!(self.resolve_variable(name), VariableAccess::Global(_))
                && self.implicit_self_candidate(name)
            {
                let idx = self.add_string_constant(name);
                self.compile_this(line)?;
                self.emit(Op::SelfOrGlobalCallee(idx), line);
                for arg in arguments {
                    if let Argument::Positional(expr) | Argument::Block(expr) = arg {
                        self.compile_expr(expr)?;
                    }
                }
                self.emit(Op::CallSelfOrGlobal(idx, arguments.len() as u8), line);
                return Ok(());
            }
        }

        // The callee is read, not called: `clock()` calls `clock` once.
        match &callee.kind {
            ExprKind::Variable(name) => self.compile_variable_get(name, line)?,
            _ => self.compile_expr(callee)?,
        }
        self.compile_call_arguments(arguments, line, Op::Call, Op::CallNamed)
    }

    /// The arguments of a labelled method call, in source order, and the
    /// `Constant::ArgNames` labelling their slots. A block's slot is labelled
    /// [`super::chunk::BLOCK_LABEL`] so the binder can hand it to the method's
    /// `&block` parameter (or drop it, as the tree-walker does, when the
    /// method declares none). Only `CallMethodNamed` and
    /// `CallSelfOrGlobalNamed` carry that label; `CallNamed` never does.
    fn compile_labelled_method_arguments(
        &mut self,
        arguments: &[Argument],
    ) -> CompileResult<(u8, u16)> {
        let mut labels: Vec<Option<crate::interpreter::value::SoliStr>> =
            Vec::with_capacity(arguments.len());
        for arg in arguments {
            match arg {
                Argument::Positional(expr) => {
                    self.compile_expr(expr)?;
                    labels.push(None);
                }
                Argument::Named(named_arg) => {
                    self.compile_expr(&named_arg.value)?;
                    labels.push(Some(named_arg.name.clone().into()));
                }
                Argument::Block(expr) => {
                    self.compile_expr(expr)?;
                    labels.push(Some(super::chunk::BLOCK_LABEL.into()));
                }
            }
        }
        let argc = labels.len() as u8;
        let idx = self.add_constant(Constant::ArgNames(std::sync::Arc::new(labels)));
        Ok((argc, idx))
    }

    /// Compile a call's arguments (the callee is already on the stack) and emit
    /// the call instruction.
    ///
    /// Every argument is evaluated in source order regardless of labelling, so
    /// side effects happen in the order written. When no argument is labelled
    /// this emits the plain `positional` opcode; otherwise it records the
    /// per-slot labels as a constant and emits `named`, leaving the actual
    /// binding to run time where the callee's identity — and therefore which of
    /// the two calling conventions applies — is known.
    fn compile_call_arguments(
        &mut self,
        arguments: &[Argument],
        line: usize,
        positional: fn(u8) -> Op,
        named: fn(u8, u16) -> Op,
    ) -> CompileResult<()> {
        self.compile_call_arguments_after(arguments, line, 0, positional, named)
    }

    /// As [`Self::compile_call_arguments`], but with `leading` argument values
    /// already pushed ahead of `arguments` — the pipeline operator evaluates its
    /// left-hand side into the first slot before the written arguments follow.
    /// Those leading slots are always positional.
    fn compile_call_arguments_after(
        &mut self,
        arguments: &[Argument],
        line: usize,
        leading: u8,
        positional: fn(u8) -> Op,
        named: fn(u8, u16) -> Op,
    ) -> CompileResult<()> {
        if arguments.len() + leading as usize > u8::MAX as usize {
            return Err(CompileError::new(
                "too many arguments in a single call",
                crate::span::Span::new(0, 0, line, 0),
            ));
        }

        let mut argc = leading;
        let mut labels: Vec<Option<crate::interpreter::value::SoliStr>> =
            vec![None; leading as usize];
        labels.reserve(arguments.len());
        for arg in arguments {
            match arg {
                Argument::Positional(expr) => {
                    self.compile_expr(expr)?;
                    labels.push(None);
                }
                Argument::Named(named_arg) => {
                    self.compile_expr(&named_arg.value)?;
                    labels.push(Some(named_arg.name.clone().into()));
                }
                Argument::Block(expr) => {
                    // A block occupies a plain trailing slot, as it does for a
                    // fully positional call.
                    self.compile_expr(expr)?;
                    labels.push(None);
                }
            }
            argc += 1;
        }

        if labels.iter().any(Option::is_some) {
            let idx = self.add_constant(Constant::ArgNames(std::sync::Arc::new(labels)));
            self.emit(named(argc, idx), line);
        } else {
            self.emit(positional(argc), line);
        }
        Ok(())
    }

    fn compile_print(&mut self, arguments: &[Argument], line: usize) -> CompileResult<()> {
        let mut argc = 0u8;
        for arg in arguments {
            match arg {
                Argument::Positional(expr) => {
                    self.compile_expr(expr)?;
                    argc += 1;
                }
                Argument::Named(_) => {
                    // Named args to print don't make sense, but compile them anyway
                    return self.compile_print_fallback(arguments, line);
                }
                Argument::Block(_expr) => {
                    // Block args to print don't make sense, but compile them anyway
                    return self.compile_print_fallback(arguments, line);
                }
            }
        }
        self.emit(Op::Print(argc), line);
        Ok(())
    }

    fn compile_print_fallback(&mut self, arguments: &[Argument], line: usize) -> CompileResult<()> {
        // Fall back to calling print as a regular function
        let idx = self.add_string_constant("print");
        self.emit(Op::GetGlobal(idx), line);
        self.compile_call_arguments(arguments, line, Op::Call, Op::CallNamed)
    }

    fn compile_json_parse(&mut self, arguments: &[Argument], line: usize) -> CompileResult<()> {
        // JSON.parse expects exactly 1 argument
        if arguments.len() != 1 {
            return Err(CompileError::new(
                "JSON.parse() expects exactly 1 argument",
                crate::span::Span::new(0, 0, line, 0),
            ));
        }
        if let Argument::Positional(expr) = &arguments[0] {
            self.compile_expr(expr)?;
        } else {
            return Err(CompileError::new(
                "JSON.parse() expects positional argument",
                crate::span::Span::new(0, 0, line, 0),
            ));
        }
        self.emit(Op::JsonParse, line);
        Ok(())
    }

    fn compile_json_stringify(&mut self, arguments: &[Argument], line: usize) -> CompileResult<()> {
        // JSON.stringify expects exactly 1 argument
        if arguments.len() != 1 {
            return Err(CompileError::new(
                "JSON.stringify() expects exactly 1 argument",
                crate::span::Span::new(0, 0, line, 0),
            ));
        }
        if let Argument::Positional(expr) = &arguments[0] {
            self.compile_expr(expr)?;
        } else {
            return Err(CompileError::new(
                "JSON.stringify() expects positional argument",
                crate::span::Span::new(0, 0, line, 0),
            ));
        }
        self.emit(Op::JsonStringify, line);
        Ok(())
    }

    fn compile_pipeline(&mut self, left: &Expr, right: &Expr, line: usize) -> CompileResult<()> {
        // x |> f(a, b) compiles as f(x, a, b)
        match &right.kind {
            ExprKind::Call { callee, arguments } => {
                // Compile callee first, then left as first arg, then rest of args
                self.compile_expr(callee)?;
                self.compile_expr(left)?;
                self.compile_call_arguments_after(arguments, line, 1, Op::Call, Op::CallNamed)?;
            }
            _ => {
                // If right is just a function reference, call it with left as sole argument
                self.compile_expr(right)?;
                self.compile_expr(left)?;
                self.emit(Op::Call(1), line);
            }
        }
        Ok(())
    }

    fn compile_this(&mut self, line: usize) -> CompileResult<()> {
        // `this` is always in slot 0 of the current method's frame
        if self.function_type == FunctionType::Method
            || self.function_type == FunctionType::Constructor
        {
            self.emit(Op::GetLocal(0), line);
        } else {
            // Might be in a closure inside a method — resolve as variable
            match self.resolve_variable("this") {
                VariableAccess::Local(slot) => {
                    self.emit(Op::GetLocal(slot), line);
                }
                VariableAccess::Upvalue(idx) => {
                    self.emit(Op::GetUpvalue(idx), line);
                }
                VariableAccess::Global(_) => {
                    // `this` used outside a class — emit GetLocal(0) and let runtime error
                    self.emit(Op::GetLocal(0), line);
                }
            }
        }
        Ok(())
    }

    fn compile_super(&mut self, line: usize) -> CompileResult<()> {
        // Push `this` (the instance) for super method dispatch
        self.compile_this(line)?;
        Ok(())
    }

    fn compile_new(
        &mut self,
        class_expr: &Expr,
        arguments: &[Argument],
        line: usize,
    ) -> CompileResult<()> {
        self.compile_expr(class_expr)?;
        self.compile_call_arguments(arguments, line, Op::New, Op::NewNamed)
    }

    fn compile_array(&mut self, elements: &[Expr], line: usize) -> CompileResult<()> {
        // `["draft", "sent"]`: one op builds it from a constant list
        // (`compiler_literal_arrays.rs`).
        if self.try_compile_literal_array(elements, line) {
            return Ok(());
        }
        // `[a, ...xs, b]`: the elements before the first spread build the
        // array, then each later one is pushed onto it — or, spread, has its
        // elements appended (`Op::Spread`), as the tree-walker does.
        let leading = elements
            .iter()
            .position(|elem| matches!(elem.kind, ExprKind::Spread(_)))
            .unwrap_or(elements.len());
        for elem in &elements[..leading] {
            self.compile_expr(elem)?;
        }
        self.emit(Op::Array(leading as u16), line);
        for elem in &elements[leading..] {
            match &elem.kind {
                ExprKind::Spread(inner) => {
                    self.compile_expr(inner)?;
                    self.emit(Op::Spread, line);
                }
                _ => {
                    self.compile_expr(elem)?;
                    self.emit(Op::ArrayPush, line);
                }
            }
        }
        Ok(())
    }

    fn compile_hash(&mut self, pairs: &[(Expr, Expr)], line: usize) -> CompileResult<()> {
        // Fast path: if every key is a literal, precompute the HashKey list
        // and store it as a constant. The runtime then only needs values on
        // the stack — no key push/convert per element.
        // Only for distinct keys: `HashWithKeys` builds the entries without
        // looking for duplicates, so `{"a": 1, "a": 2}` takes the general path
        // below, where the last value wins at the first key's position.
        if let Some(keys) = self.literal_hash_keys(pairs).filter(|keys| {
            keys.iter().collect::<std::collections::HashSet<_>>().len() == keys.len()
        }) {
            let keys_idx = self.add_constant(Constant::HashKeys(std::sync::Arc::new(keys)));
            for (_, value) in pairs {
                self.compile_expr(value)?;
            }
            self.emit(Op::HashWithKeys(keys_idx, pairs.len() as u16), line);
            return Ok(());
        }
        for (key, value) in pairs {
            self.compile_expr(key)?;
            self.compile_expr(value)?;
        }
        self.emit(Op::Hash(pairs.len() as u16), line);
        Ok(())
    }

    /// Returns Some(keys) if every key in `pairs` is a literal that can be
    /// converted to a HashKey at compile time.
    fn literal_hash_keys(
        &self,
        pairs: &[(Expr, Expr)],
    ) -> Option<Vec<crate::interpreter::value::HashKey>> {
        use crate::interpreter::value::{DecimalValue, HashKey};
        let mut keys = Vec::with_capacity(pairs.len());
        for (key, _) in pairs {
            let hk = match &key.kind {
                ExprKind::StringLiteral(s) => HashKey::String(s.clone().into()),
                ExprKind::IntLiteral(n) => HashKey::Int(*n),
                ExprKind::BoolLiteral(b) => HashKey::Bool(*b),
                ExprKind::Null => HashKey::Null,
                ExprKind::Symbol(s) => HashKey::Symbol(s.clone().into()),
                ExprKind::DecimalLiteral(s) => {
                    let d: rust_decimal::Decimal = s.parse().ok()?;
                    let prec = s.split('.').nth(1).map(|p| p.len() as u32).unwrap_or(0);
                    HashKey::Decimal(DecimalValue(d, prec))
                }
                _ => return None,
            };
            keys.push(hk);
        }
        Some(keys)
    }

    fn compile_block_expr(&mut self, stmts: &[Stmt], line: usize) -> CompileResult<()> {
        self.begin_scope();
        if stmts.is_empty() {
            self.emit(Op::Null, line);
        } else {
            let last_idx = stmts.len() - 1;
            for (i, stmt) in stmts.iter().enumerate() {
                if i == last_idx {
                    // Last statement — if it's an expression, keep its value
                    if let StmtKind::Expression(expr) = &stmt.kind {
                        self.compile_expr(expr)?;
                    } else {
                        self.compile_stmt(stmt)?;
                        self.emit(Op::Null, line);
                    }
                } else {
                    self.compile_stmt(stmt)?;
                }
            }
        }
        self.end_scope(line);
        Ok(())
    }

    fn compile_assign(&mut self, target: &Expr, value: &Expr, line: usize) -> CompileResult<()> {
        if self.template_mode {
            if let Some(local) = template_ivar(target) {
                return self.compile_assign(&local, value, line);
            }
        }
        match &target.kind {
            ExprKind::Variable(name) => {
                // `name = name + <expr>` (and `name += <expr>`, desugared to
                // it): one op reads, adds and stores, so a string grows in
                // place instead of being copied per iteration. A variable or
                // literal right-hand side is left to the peepholes
                // (`AddLocalsInPlace`, `AppendLocalConst`, `IncrLocal`, …).
                if let ExprKind::Binary {
                    left,
                    operator: BinaryOp::Add,
                    right,
                } = &value.kind
                {
                    let self_add =
                        matches!(&left.kind, ExprKind::Variable(left_name) if left_name == name);
                    let simple_rhs = matches!(
                        right.kind,
                        ExprKind::Variable(_)
                            | ExprKind::IntLiteral(_)
                            | ExprKind::FloatLiteral(_)
                            | ExprKind::StringLiteral(_)
                    );
                    // `acc = acc + h[keys[k]]` over locals is `AddNestedIndex`'s.
                    let nested_index_of_variables = matches!(
                        &right.kind,
                        ExprKind::Index { object, index }
                            if matches!(object.kind, ExprKind::Variable(_))
                                && matches!(&index.kind, ExprKind::Index { object, index }
                                    if matches!(object.kind, ExprKind::Variable(_))
                                        && matches!(index.kind, ExprKind::Variable(_)))
                    );
                    if self_add && !simple_rhs && !nested_index_of_variables {
                        let add_assign = match self.resolve_variable(name) {
                            VariableAccess::Local(slot) => Op::AddAssignLocal(slot),
                            VariableAccess::Upvalue(idx) => Op::AddAssignUpvalue(idx),
                            VariableAccess::Global(global_name) => {
                                if self.scope_depth == 0 {
                                    self.known_globals.borrow_mut().insert(global_name.clone());
                                }
                                Op::AddAssignGlobal(self.add_string_constant(&global_name))
                            }
                        };
                        self.compile_expr(left)?;
                        self.compile_expr(right)?;
                        self.emit(add_assign, line);
                        return Ok(());
                    }
                }
                self.compile_expr(value)?;
                let name_clone = name.clone();
                match self.resolve_variable(&name_clone) {
                    VariableAccess::Local(slot) => {
                        self.emit(Op::SetLocal(slot), line);
                    }
                    VariableAccess::Upvalue(idx) => {
                        self.emit(Op::SetUpvalue(idx), line);
                    }
                    VariableAccess::Global(name) => {
                        // A bare assignment to a name that is neither a local nor
                        // an upvalue. New in-function locals are hoisted to
                        // function entry by `hoist_locals` before the body is
                        // compiled, or declared by their first assignment, so by
                        // the time we get here the name is either a real global
                        // (top level, or one we know exists) — define-or-update
                        // it. `SetGlobal` upserts. At top level it is the
                        // program's own binding, as a `let` would be: `clock = 7`
                        // shadows the builtin.
                        //
                        // A name still waiting for its in-place declaration is
                        // not a global: storing to one here means that statement
                        // was compiled some other way, and the function is
                        // refused rather than leak the local into the globals.
                        if self.in_place_pending(&name) {
                            return Err(CompileError::new(
                                format!(
                                    "`{name}` is assigned before the statement that declares it"
                                ),
                                target.span,
                            ));
                        }
                        if self.scope_depth == 0 {
                            self.known_globals.borrow_mut().insert(name.clone());
                            self.program_globals.borrow_mut().insert(name.clone());
                        }
                        let idx = self.add_string_constant(&name);
                        self.emit(Op::SetGlobal(idx), line);
                    }
                }
            }
            ExprKind::Member { object, name } => {
                self.compile_expr(object)?;
                self.compile_expr(value)?;
                let idx = self.add_string_constant(name);
                self.emit(Op::SetProperty(idx), line);
            }
            ExprKind::Index { object, index } => {
                // Peephole: hash[const_str] = value -> HashSetConst, fusing the
                // receiver load when it is a plain variable. Mirrors the read
                // path: `h.set("k", v)` already compiled to one dispatch while
                // `h["k"] = v` — the far more common spelling — took two.
                //
                // Safe to swap because both sequences leave exactly one value,
                // the assigned one: the generic form pops receiver+value and
                // pushes the value, the fused form pops the value and pushes it
                // back. Only a plain variable receiver is fused, so skipping
                // its evaluation cannot lose a side effect.
                if let ExprKind::StringLiteral(key) = &index.kind {
                    let key_idx = self.add_string_constant(key);
                    let fused = match &object.kind {
                        ExprKind::Variable(var_name) => match self.resolve_variable(var_name) {
                            VariableAccess::Local(slot) => {
                                Some(Op::HashSetLocalConst(slot, key_idx))
                            }
                            VariableAccess::Global(_) => {
                                let gidx = self.add_string_constant(var_name);
                                Some(Op::HashSetGlobalConst(gidx, key_idx))
                            }
                            VariableAccess::Upvalue(_) => None,
                        },
                        _ => None,
                    };
                    match fused {
                        Some(op) => {
                            self.compile_expr(value)?;
                            self.emit(op, line);
                        }
                        None => {
                            self.compile_expr(object)?;
                            self.compile_expr(value)?;
                            self.emit(Op::HashSetConst(key_idx), line);
                        }
                    }
                } else {
                    self.compile_expr(object)?;
                    self.compile_expr(index)?;
                    self.compile_expr(value)?;
                    self.emit(Op::SetIndex, line);
                }
            }
            _ => {
                return Err(CompileError::new("Invalid assignment target", target.span));
            }
        }
        Ok(())
    }

    fn compile_lambda(
        &mut self,
        params: &[crate::ast::stmt::Parameter],
        body: &[Stmt],
        line: usize,
    ) -> CompileResult<()> {
        // Compile lambda as a nested function
        let _dummy = self.start_function(FunctionType::Lambda, "<lambda>".to_string(), params);

        self.begin_scope();
        self.emit_param_defaults(params)?;
        self.compile_function_body(body)?;
        self.end_scope(line);

        let proto = self.finish_function(line);
        let upvalue_count = proto.upvalue_descriptors.len();
        let idx = self.add_constant(Constant::Function(Arc::new(proto)));
        self.emit(Op::Closure(idx), line);
        // Upvalue descriptors are read by the VM from the proto
        let _ = upvalue_count;
        Ok(())
    }

    fn compile_if_expr(
        &mut self,
        condition: &Expr,
        then_branch: &Expr,
        else_branch: Option<&Expr>,
        line: usize,
    ) -> CompileResult<()> {
        let then_jump = self.compile_condition_jump(condition, false, line)?;

        self.compile_expr(then_branch)?;

        if let Some(else_expr) = else_branch {
            let else_jump = self.emit_jump(Op::Jump(0), line);
            self.patch_jump(then_jump);
            self.compile_expr(else_expr)?;
            self.patch_jump(else_jump);
        } else {
            let else_jump = self.emit_jump(Op::Jump(0), line);
            self.patch_jump(then_jump);
            self.emit(Op::Null, line);
            self.patch_jump(else_jump);
        }
        Ok(())
    }

    fn compile_interpolated_string(
        &mut self,
        parts: &[InterpolatedPart],
        line: usize,
    ) -> CompileResult<()> {
        // Adjacent literals fold into one constant and empty ones vanish, so
        // `BuildString` only ever concatenates what the source can't know
        // statically; an all-literal string compiles to a plain constant.
        let mut count = 0usize;
        let mut pending = String::new();
        for part in parts {
            match part {
                InterpolatedPart::Literal(s) => pending.push_str(s),
                InterpolatedPart::Expression(expr) => {
                    if !pending.is_empty() {
                        let text = std::mem::take(&mut pending);
                        self.emit_constant(Constant::String(text.into()), line);
                        count += 1;
                    }
                    self.compile_expr(expr)?;
                    count += 1;
                }
            }
        }
        let only_literal = count == 0;
        if !pending.is_empty() || only_literal {
            self.emit_constant(Constant::String(pending.into()), line);
            count += 1;
        }
        if !only_literal {
            self.emit(Op::BuildString(count as u16), line);
        }
        Ok(())
    }

    fn compile_sdql_block(
        &mut self,
        query: &str,
        interpolations: &[crate::ast::expr::SdqlInterpolation],
        line: usize,
    ) -> CompileResult<()> {
        // Lower `@sdbql{ ... #{var} ... }` to `__sdql_exec(query, binds)`. The
        // raw query string and a `{ "var": value }` binds hash are passed to
        // the global builtin, which rewrites `#{var}` -> `@var` and runs the
        // query (see `register_builtins` / `run_sdql_block`). We can't inline
        // the interpolations here the way the tree-walker does because their
        // values aren't known until runtime — so we resolve each as a normal
        // variable load and hand the lot to the builtin.
        //
        // Stack discipline for the trailing `Call(2)`: callee first, then the
        // two args (query, binds), so the binds hash must be built *after* the
        // query is pushed.
        let fn_idx = self.add_string_constant("__sdql_exec");
        self.emit(Op::GetGlobal(fn_idx), line);

        self.emit_constant(Constant::String(query.to_string().into()), line);

        let mut pair_count: u16 = 0;
        for interp in interpolations {
            let var_name = interp
                .expr
                .trim_start_matches('{')
                .trim_end_matches('}')
                .trim();
            if var_name.is_empty() {
                continue;
            }
            // key: the bare variable name
            self.emit_constant(Constant::String(var_name.to_string().into()), line);
            // value: the variable's current binding
            self.compile_variable_get(var_name, line)?;
            pair_count += 1;
        }
        self.emit(Op::Hash(pair_count), line);

        self.emit(Op::Call(2), line);

        Ok(())
    }

    fn try_compile_hash_const_string_call(
        &mut self,
        object: &Expr,
        name: &str,
        arguments: &[Argument],
        line: usize,
    ) -> CompileResult<Option<()>> {
        if let ExprKind::Variable(var_name) = &object.kind {
            // Resolve the receiver exactly the way an ordinary variable read
            // does. The global arm below used to be gated on `scope_depth == 0`
            // instead, which is a proxy for "cannot be a local" — but it also
            // meant the global super-instructions never fired inside a function,
            // i.e. never in real handler code. `resolve_variable` is the honest
            // test: it reports Upvalue for a captured binding, which has no
            // fused form and must fall through to the generic path rather than
            // be mistaken for a global.
            let access = self.resolve_variable(var_name);
            if let VariableAccess::Local(slot) = access {
                match (name, arguments) {
                    ("get", [Argument::Positional(arg)])
                    | ("has_key", [Argument::Positional(arg)]) => {
                        let ExprKind::StringLiteral(key) = &arg.kind else {
                            return Ok(None);
                        };
                        let key_idx = self.add_string_constant(key);
                        match name {
                            "get" => self.emit(Op::HashGetLocalConst(slot, key_idx), line),
                            "has_key" => self.emit(Op::HashHasKeyLocalConst(slot, key_idx), line),
                            _ => unreachable!(),
                        };
                        return Ok(Some(()));
                    }
                    ("set", [Argument::Positional(key), Argument::Positional(value)]) => {
                        let ExprKind::StringLiteral(key) = &key.kind else {
                            return Ok(None);
                        };
                        let key_idx = self.add_string_constant(key);
                        self.compile_expr(value)?;
                        self.emit(Op::HashSetLocalConst(slot, key_idx), line);
                        return Ok(Some(()));
                    }
                    _ => {}
                }
            }
            if matches!(access, VariableAccess::Global(_)) {
                match (name, arguments) {
                    ("get", [Argument::Positional(arg)])
                    | ("has_key", [Argument::Positional(arg)]) => {
                        let ExprKind::StringLiteral(key) = &arg.kind else {
                            return Ok(None);
                        };
                        let global_idx = self.add_string_constant(var_name);
                        let key_idx = self.add_string_constant(key);
                        match name {
                            "get" => self.emit(Op::HashGetGlobalConst(global_idx, key_idx), line),
                            "has_key" => {
                                self.emit(Op::HashHasKeyGlobalConst(global_idx, key_idx), line)
                            }
                            _ => unreachable!(),
                        };
                        return Ok(Some(()));
                    }
                    ("set", [Argument::Positional(key), Argument::Positional(value)]) => {
                        let ExprKind::StringLiteral(key) = &key.kind else {
                            return Ok(None);
                        };
                        let global_idx = self.add_string_constant(var_name);
                        let key_idx = self.add_string_constant(key);
                        self.compile_expr(value)?;
                        self.emit(Op::HashSetGlobalConst(global_idx, key_idx), line);
                        return Ok(Some(()));
                    }
                    _ => {}
                }
            }
        }

        match (name, arguments) {
            ("get", [Argument::Positional(arg)]) | ("has_key", [Argument::Positional(arg)]) => {
                let ExprKind::StringLiteral(key) = &arg.kind else {
                    return Ok(None);
                };
                let key_idx = self.add_string_constant(key);
                self.compile_expr(object)?;
                match name {
                    "get" => self.emit(Op::HashGetConst(key_idx), line),
                    "has_key" => self.emit(Op::HashHasKeyConst(key_idx), line),
                    _ => unreachable!(),
                };
                Ok(Some(()))
            }
            ("set", [Argument::Positional(key), Argument::Positional(value)]) => {
                let ExprKind::StringLiteral(key) = &key.kind else {
                    return Ok(None);
                };
                let key_idx = self.add_string_constant(key);
                self.compile_expr(object)?;
                self.compile_expr(value)?;
                self.emit(Op::HashSetConst(key_idx), line);
                Ok(Some(()))
            }
            _ => Ok(None),
        }
    }

    /// `receiver["a"]["b"]` with literal keys. Returns true when an opcode was
    /// emitted so the caller must not compile the index again.
    fn try_emit_nested_const_index(
        &mut self,
        object: &Expr,
        outer_key: &str,
        line: usize,
    ) -> CompileResult<bool> {
        let ExprKind::Index {
            object: inner,
            index: inner_index,
        } = &object.kind
        else {
            return Ok(false);
        };
        let ExprKind::StringLiteral(inner_key) = &inner_index.kind else {
            return Ok(false);
        };
        let k1 = self.add_string_constant(inner_key);
        let k2 = self.add_string_constant(outer_key);
        match &inner.kind {
            ExprKind::Variable(var_name) => match self.resolve_variable(var_name) {
                VariableAccess::Local(slot) => {
                    self.emit(Op::HashGetLocalConst2(slot, k1, k2), line);
                    Ok(true)
                }
                VariableAccess::Global(_) => {
                    let gidx = self.add_string_constant(var_name);
                    self.emit(Op::HashGetGlobalConst2(gidx, k1, k2), line);
                    Ok(true)
                }
                VariableAccess::Upvalue(_) => Ok(false),
            },
            _ => {
                self.compile_expr(inner)?;
                self.emit(Op::HashGetConst2(k1, k2), line);
                Ok(true)
            }
        }
    }

    fn compile_list_comprehension(
        &mut self,
        element: &Expr,
        variable: &str,
        iterable: &Expr,
        condition: Option<&Expr>,
        line: usize,
    ) -> CompileResult<()> {
        // Result slot is `locals.len()`, which equals the array's stack
        // position only at the locals baseline. As a sub-expression (call
        // argument, element of an enclosing literal) wrap in a lambda so the
        // inner compiler is at a clean frame — same value, no wrong-slot
        // GetLocal. See `Compiler::wrap_in_lambda`.
        if self.stack_height != self.locals.len() {
            return self.wrap_in_lambda(line, |c| {
                c.compile_list_comprehension(element, variable, iterable, condition, line)
            });
        }

        let result_slot = self.locals.len() as u16;
        self.emit(Op::Array(0), line); // empty result array, at result_slot
        self.add_local(String::new(), false); // anonymous result-array local

        // Iterator setup, as `compile_for`'s: a range is counted, not built.
        let is_range = self.compile_iterable(iterable, line)?;
        // The iterable/range now lives on the iter_stack; value stack is back at
        // the result-array baseline.
        self.resync_stack_height();

        let loop_start = self.current_offset();
        let exit_jump = if is_range {
            self.emit_jump(Op::ForIterRange(0), line)
        } else {
            self.emit_jump(Op::ForIter(0), line)
        };

        self.begin_scope();
        self.add_local(variable.to_string(), false); // loop var = yielded element
        self.resync_stack_height(); // body entry: height == locals.len()

        // Append `element` to the result array: GetLocal(result); element;
        // ArrayPush (mutates in place, leaves the array copy); Pop.
        if let Some(cond) = condition {
            let skip = self.compile_condition_jump(cond, false, line)?;
            self.emit(Op::GetLocal(result_slot), line);
            self.compile_expr(element)?;
            self.emit(Op::ArrayPush, line);
            self.emit(Op::Pop, line);
            self.patch_jump(skip);
        } else {
            self.emit(Op::GetLocal(result_slot), line);
            self.compile_expr(element)?;
            self.emit(Op::ArrayPush, line);
            self.emit(Op::Pop, line);
        }

        self.end_scope(line); // pop loop var (closes upvalue if a closure captured it)
        self.emit_loop(loop_start, line);
        self.patch_jump(exit_jump);

        // The result array stays on the stack as the expression's value; drop
        // only its compiler-side local bookkeeping (no Pop op).
        self.locals.pop();
        self.stack_height = result_slot as usize + 1;
        Ok(())
    }

    fn compile_hash_comprehension(
        &mut self,
        key: &Expr,
        value: &Expr,
        variable: &str,
        iterable: &Expr,
        condition: Option<&Expr>,
        line: usize,
    ) -> CompileResult<()> {
        // Same baseline requirement as list comprehensions.
        if self.stack_height != self.locals.len() {
            return self.wrap_in_lambda(line, |c| {
                c.compile_hash_comprehension(key, value, variable, iterable, condition, line)
            });
        }

        let result_slot = self.locals.len() as u16;
        self.emit(Op::Hash(0), line); // empty result hash, at result_slot
        self.add_local(String::new(), false); // anonymous result-hash local

        let is_range = self.compile_iterable(iterable, line)?;
        self.resync_stack_height();

        let loop_start = self.current_offset();
        let exit_jump = if is_range {
            self.emit_jump(Op::ForIterRange(0), line)
        } else {
            self.emit_jump(Op::ForIter(0), line)
        };

        self.begin_scope();
        self.add_local(variable.to_string(), false);
        self.resync_stack_height();

        // Insert key=>value into the result hash: GetLocal(result); key; value;
        // SetIndex (hash[key]=value, leaves the hash); Pop. SetIndex (not
        // HashSetConst) because the key is a runtime value, not a constant.
        if let Some(cond) = condition {
            let skip = self.compile_condition_jump(cond, false, line)?;
            self.emit(Op::GetLocal(result_slot), line);
            self.compile_expr(key)?;
            self.compile_expr(value)?;
            self.emit(Op::SetIndex, line);
            self.emit(Op::Pop, line);
            self.patch_jump(skip);
        } else {
            self.emit(Op::GetLocal(result_slot), line);
            self.compile_expr(key)?;
            self.compile_expr(value)?;
            self.emit(Op::SetIndex, line);
            self.emit(Op::Pop, line);
        }

        self.end_scope(line);
        self.emit_loop(loop_start, line);
        self.patch_jump(exit_jump);

        self.locals.pop();
        self.stack_height = result_slot as usize + 1;
        Ok(())
    }
}

#[cfg(test)]
mod subexpr_compile_tests {
    use crate::vm::compiler::Compiler;

    fn compiles(src: &str) -> bool {
        let tokens = crate::lexer::Scanner::new(src).scan_tokens().expect("lex");
        let program = crate::parser::Parser::new(tokens).parse().expect("parse");
        Compiler::compile(&program).is_ok()
    }

    #[test]
    fn list_comprehension_as_array_element_compiles() {
        assert!(
            compiles("let r = [[1, 2], [x for x in [3, 4]]]"),
            "nested list comprehension must compile via wrap_in_lambda"
        );
    }

    #[test]
    fn list_comprehension_as_call_arg_compiles() {
        assert!(compiles(
            "fn total(a) { a }\nprint(total([x * 2 for x in [1, 2, 3]]))"
        ));
    }

    #[test]
    fn binding_match_as_call_argument_compiles() {
        assert!(compiles(
            "let out = []\nfor i in [1, 2, 3] { out.push(match i { 1 => \"a\", n => \"n\" + str(n) }) }"
        ));
    }

    #[test]
    fn command_substitution_compiles() {
        assert!(
            compiles("let files = `echo hello`"),
            "backticks must compile as System.shell"
        );
    }

    #[test]
    fn command_substitution_runs_and_resolves_stdout() {
        use crate::interpreter::environment::Environment;
        use crate::interpreter::value::Value;
        use crate::vm::Vm;

        let src = "let out = `printf hello`.stdout";
        let tokens = crate::lexer::Scanner::new(src).scan_tokens().expect("lex");
        let program = crate::parser::Parser::new(tokens).parse().expect("parse");
        let module = Compiler::compile(&program).expect("compile");

        let mut env = Environment::new();
        crate::interpreter::builtins::system::register_system_builtins(&mut env);
        let mut vm = Vm::new();
        vm.globals.insert(
            "System".to_string(),
            env.get("System").expect("System builtin"),
        );
        vm.execute(&module.main).expect("vm");
        assert_eq!(vm.globals.get("out"), Some(&Value::String("hello".into())));
    }
}

#[cfg(test)]
mod transaction_compile_tests {
    //! The block form compiles to `CallMethod`; the VM intercepts a model-class
    //! receiver and runs begin/commit/rollback around `invoke_callable`.
    use crate::vm::compiler::Compiler;

    fn compiles(src: &str) -> bool {
        let tokens = crate::lexer::Scanner::new(src).scan_tokens().expect("lex");
        let program = crate::parser::Parser::new(tokens).parse().expect("parse");
        Compiler::compile(&program).is_ok()
    }

    #[test]
    fn block_form_compiles_on_vm() {
        assert!(
            compiles("User.transaction(fn() { User.create({}) })"),
            "the literal block form must compile; the VM native path runs the tx"
        );
        assert!(compiles("User.transaction do User.create({}) end"));
    }

    #[test]
    fn string_and_handle_forms_still_compile() {
        assert!(
            compiles("User.transaction(\"FOR u IN users RETURN u\")"),
            "the AQL-string form is an ordinary native call and must compile"
        );
        assert!(
            compiles("let tx = User.transaction()"),
            "the no-arg handle form is an ordinary native call and must compile"
        );
    }
}

#[cfg(test)]
mod grouped_compile_tests {
    //! `grouped(fn() { ... })` compiles as an ordinary call; the VM intercepts
    //! the native and runs the batch around `invoke_callable`.
    use crate::vm::compiler::Compiler;

    fn compiles(src: &str) -> bool {
        let tokens = crate::lexer::Scanner::new(src).scan_tokens().expect("lex");
        let program = crate::parser::Parser::new(tokens).parse().expect("parse");
        Compiler::compile(&program).is_ok()
    }

    #[test]
    fn block_form_compiles_on_vm() {
        assert!(
            compiles("grouped(fn() { Post.all })"),
            "grouped() must compile; the VM native path owns the batch"
        );
    }
}

#[cfg(test)]
mod break_in_try_compile_tests {
    use crate::vm::compiler::Compiler;

    fn compiles(src: &str) -> bool {
        let tokens = crate::lexer::Scanner::new(src).scan_tokens().expect("lex");
        let program = crate::parser::Parser::new(tokens).parse().expect("parse");
        Compiler::compile(&program).is_ok()
    }

    #[test]
    fn break_inside_try_in_loop_compiles() {
        assert!(compiles("for x in [1, 2, 3] { try { break } catch e { } }"));
    }

    #[test]
    fn next_inside_try_in_loop_compiles() {
        assert!(compiles("for x in [1, 2, 3] { try { next } catch e { } }"));
    }

    #[test]
    fn break_inside_try_finally_in_loop_compiles() {
        assert!(compiles("for x in [1, 2, 3] { try { break } finally { } }"));
    }
}

#[cfg(test)]
mod named_args_compile_tests {
    //! Named-argument calls compile to `Op::CallNamed` / `Op::NewNamed`, which
    //! carry the per-slot labels as a constant and bind them at call time —
    //! where the callee is known and its convention (reorder into parameter
    //! slots for a user function, collapse into a trailing options hash for a
    //! native) can be chosen. They previously failed to compile, which was
    //! correct but demoted the whole enclosing handler to the tree-walking
    //! interpreter for the rest of the worker's life. These tests pin that all
    //! four call-compilation paths (plain call, print fallback, pipeline,
    //! `new`) both compile and bind correctly.
    use crate::interpreter::value::Value;
    use crate::vm::compiler::Compiler;

    fn compiles(src: &str) -> bool {
        let tokens = crate::lexer::Scanner::new(src).scan_tokens().expect("lex");
        let program = crate::parser::Parser::new(tokens).parse().expect("parse");
        Compiler::compile(&program).is_ok()
    }

    /// Run `src` on the VM and read back a global — the engines must agree, so
    /// these expectations mirror the tree-walking interpreter's results.
    fn run_and_get(src: &str, global: &str) -> Value {
        let tokens = crate::lexer::Scanner::new(src).scan_tokens().expect("lex");
        let program = crate::parser::Parser::new(tokens).parse().expect("parse");
        let module = Compiler::compile(&program).expect("compile");
        let mut vm = crate::vm::Vm::new();
        vm.execute(&module.main).expect("vm error");
        vm.globals.get(global).cloned().expect("missing global")
    }

    #[test]
    fn named_arg_call_binds_by_parameter_name() {
        assert!(compiles(
            "def add(a, b) { return a + b }\nprint(add(b: 2, a: 1))"
        ));
        // Labels bind by name, so the written order is irrelevant.
        assert_eq!(
            run_and_get(
                "def sub(a, b) { return a - b }\nlet x = sub(b: 1, a: 10);",
                "x"
            ),
            Value::Int(9)
        );
    }

    #[test]
    fn named_arg_mixes_with_positional_and_defaults() {
        assert_eq!(
            run_and_get(
                "def f(a, b = 10, c = 100) { return a + b + c }\nlet x = f(1, c: 5);",
                "x"
            ),
            Value::Int(16)
        );
    }

    #[test]
    fn named_arg_to_print_is_accepted() {
        // print's fallback path routes through the generic call machinery,
        // where print is a native and the label collapses into an options hash.
        assert!(compiles("print(msg: \"hi\")"));
    }

    #[test]
    fn named_arg_in_pipeline_binds_by_name() {
        assert!(compiles(
            "def scale(x, factor) { return x * factor }\nprint(5 |> scale(factor: 3))"
        ));
        // The piped value fills the first slot positionally; the label fills the rest.
        assert_eq!(
            run_and_get(
                "def scale(x, factor) { return x * factor }\nlet x = 5 |> scale(factor: 3);",
                "x"
            ),
            Value::Int(15)
        );
    }

    #[test]
    fn positional_calls_still_compile() {
        assert!(
            compiles("def add(a, b) { return a + b }\nprint(add(1, 2))"),
            "positional calls must still compile to bytecode"
        );
        assert!(
            compiles("print(\"hi\")"),
            "positional print must still compile to bytecode"
        );
        assert!(
            compiles("def scale(x, factor) { return x * factor }\nprint(5 |> scale(3))"),
            "positional pipeline calls must still compile"
        );
    }
}

/// In a view, `@name` is the local `name` (see `Compiler::template_mode`).
/// The argument of a `__tpl_write(arg)` / `__tpl_write_raw(arg)` intrinsic
/// call and whether it is escaped (`__tpl_write`, `<%= %>`).
fn template_write_args<'a>(name: &str, arguments: &'a [Argument]) -> Option<(&'a Expr, bool)> {
    let escaped = match name {
        "__tpl_write" => true,
        "__tpl_write_raw" => false,
        _ => return None,
    };
    match arguments {
        [Argument::Positional(arg)] => Some((arg, escaped)),
        _ => None,
    }
}

/// [`template_write_args`] for a whole expression: a direct call of either
/// intrinsic, as `write_stmt` builds it.
pub(super) fn template_write_call(expr: &Expr) -> Option<(&Expr, bool)> {
    match &expr.kind {
        ExprKind::Call { callee, arguments } => match &callee.kind {
            ExprKind::Variable(name) => template_write_args(name, arguments),
            _ => None,
        },
        _ => None,
    }
}

fn template_ivar(expr: &Expr) -> Option<Expr> {
    match &expr.kind {
        ExprKind::Member { object, name } if matches!(object.kind, ExprKind::This) => {
            Some(Expr::new(ExprKind::Variable(name.clone()), expr.span))
        }
        _ => None,
    }
}

#[cfg(test)]
mod labelled_method_call_compile_tests {
    //! C583: a method call with a labelled argument compiles to
    //! `CallMethodNamed` (or `CallSelfOrGlobalNamed` for the bare helper
    //! call), which keeps the receiver. It used to read `obj.m` and call the
    //! result with `CallNamed`, losing `this` — and a method of arity 0 ran on
    //! that read.
    use crate::interpreter::value::Value;
    use crate::vm::chunk::{Constant, FunctionProto};
    use crate::vm::compiler::Compiler;
    use crate::vm::opcode::Op;

    fn compile(src: &str) -> std::sync::Arc<FunctionProto> {
        let tokens = crate::lexer::Scanner::new(src).scan_tokens().expect("lex");
        let program = crate::parser::Parser::new(tokens).parse().expect("parse");
        Compiler::compile(&program).expect("compile").main
    }

    /// Every op of `proto` and of the functions compiled inside it.
    fn all_ops(proto: &FunctionProto, out: &mut Vec<Op>) {
        out.extend(proto.chunk.code.iter().copied());
        for constant in &proto.chunk.constants {
            if let Constant::Function(inner) = constant {
                all_ops(inner, out);
            }
        }
    }

    fn find_proto<'a>(proto: &'a FunctionProto, name: &str) -> Option<&'a FunctionProto> {
        if proto.name == name {
            return Some(proto);
        }
        proto
            .chunk
            .constants
            .iter()
            .find_map(|constant| match constant {
                Constant::Function(inner) => find_proto(inner, name),
                _ => None,
            })
    }

    const CALC: &str = "class Calc\n  def plus(a, b = 5)\n    a + b\n  end\n\n  def via_self\n    \"#{this.plus(a: 1)} #{@plus(a: 2)} #{_half(n: 8)}\"\n  end\n\n  def _half(n)\n    n / 2\n  end\nend\nc = Calc.new()\nx = c.plus(b: 2, a: 1)\ny = c&.plus(a: 3)\n";

    #[test]
    fn a_labelled_method_call_keeps_its_receiver() {
        let main = compile(CALC);
        let mut ops = Vec::new();
        all_ops(&main, &mut ops);
        let method_named = ops
            .iter()
            .filter(|op| matches!(op, Op::CallMethodNamed(..)))
            .count();
        let self_named = ops
            .iter()
            .filter(|op| matches!(op, Op::CallSelfOrGlobalNamed(..)))
            .count();
        // `this.plus(a:)`, `@plus(a:)`, `c.plus(b:, a:)`, `c&.plus(a:)`.
        assert_eq!(method_named, 4, "{ops:?}");
        // `_half(n: 8)`.
        assert_eq!(self_named, 1, "{ops:?}");
        assert!(
            !ops.iter().any(|op| matches!(op, Op::CallNamed(..))),
            "no labelled method call may read the method and call it: {ops:?}"
        );
    }

    #[test]
    fn a_safe_navigation_labelled_call_tests_the_receiver_before_its_arguments() {
        let main = compile("p = nil\nx = p&.m(a: 1)\n");
        let code = &main.chunk.code;
        let jump = code
            .iter()
            .position(|op| matches!(op, Op::JumpIfNull(_)))
            .expect("a null test");
        let call = code
            .iter()
            .position(|op| matches!(op, Op::CallMethodNamed(..)))
            .expect("a labelled method call");
        assert!(jump < call, "{code:?}");
        let Op::JumpIfNull(offset) = code[jump] else {
            unreachable!()
        };
        // The jump lands just past the call: a nil receiver skips the
        // arguments and the call alike.
        assert_eq!(jump + 1 + offset as usize, call + 1, "{code:?}");
    }

    #[test]
    fn a_labelled_method_call_labels_its_block() {
        let main = compile("def run(c)\n  c.amp(a: 4) do |x|\n    x + 1\n  end\nend\n");
        let run = find_proto(&main, "run").expect("run");
        let names_idx = run
            .chunk
            .code
            .iter()
            .find_map(|op| match op {
                Op::CallMethodNamed(_, 2, names) => Some(*names),
                _ => None,
            })
            .expect("a two-slot labelled method call");
        let Constant::ArgNames(labels) = &run.chunk.constants[names_idx as usize] else {
            panic!("labels")
        };
        let labels: Vec<Option<&str>> = labels.iter().map(|l| l.as_deref()).collect();
        assert_eq!(labels, vec![Some("a"), Some(crate::vm::chunk::BLOCK_LABEL)]);
    }

    #[test]
    fn a_block_parameter_is_recorded_on_the_proto() {
        let main = compile(
            "class C\n  def amp(a, &blk)\n    blk(a)\n  end\n\n  def plain(a)\n    a\n  end\nend\n",
        );
        assert_eq!(find_proto(&main, "amp").expect("amp").block_param, Some(1));
        assert_eq!(find_proto(&main, "plain").expect("plain").block_param, None);
    }

    #[test]
    fn a_labelled_method_call_runs_once_with_its_receiver() {
        let src = "class Calc\n  calls: Int\n\n  new()\n    @calls = 0\n  end\n\n  def opt(a = 1, b = 2)\n    @calls = @calls + 1\n    a * 100 + b\n  end\n\n  def me(flag)\n    this\n  end\nend\nc = Calc.new()\nx = c.opt(b: 7)\nn = c.calls\nsame = c.me(flag: true) == c\n";
        let main = compile(src);
        let mut vm = crate::vm::Vm::new();
        vm.execute(&main).expect("vm error");
        assert_eq!(vm.globals.get("x").cloned(), Some(Value::Int(107)));
        assert_eq!(vm.globals.get("n").cloned(), Some(Value::Int(1)));
        assert_eq!(vm.globals.get("same").cloned(), Some(Value::Bool(true)));
    }
}
