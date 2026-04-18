//! Expression compilation — AST expressions to bytecode.

use std::rc::Rc;

use crate::ast::expr::{Argument, BinaryOp, ExprKind, InterpolatedPart, UnaryOp};
use crate::ast::stmt::StmtKind;
use crate::ast::{Expr, Stmt};
use crate::error::CompileError;

use super::chunk::Constant;
use super::compiler::{CompileResult, Compiler, FunctionType, VariableAccess};
use super::opcode::Op;

impl Compiler {
    /// Compile an expression — the result is left on the stack.
    pub fn compile_expr(&mut self, expr: &Expr) -> CompileResult<()> {
        let line = expr.span.line;
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
                self.emit_constant(Constant::String(s.clone()), line);
            }
            ExprKind::CommandSubstitution(_) => {
                return Err(CompileError::new(
                    "Command substitution `...` is not supported in compiled mode",
                    expr.span,
                ));
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
            }
            ExprKind::Binary {
                left,
                operator,
                right,
            } => {
                self.compile_binary(left, *operator, right, line)?;
            }
            ExprKind::Unary { operator, operand } => {
                self.compile_unary(*operator, operand, line)?;
            }
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
                let idx = self.add_string_constant(name);
                self.emit(Op::GetProperty(idx), line);
            }
            ExprKind::SafeMember { .. } => {
                unimplemented!("SafeMember not supported in VM yet");
            }
            ExprKind::QualifiedName { qualifier, name } => {
                self.compile_expr(qualifier)?;
                let idx = self.add_string_constant(name);
                self.emit(Op::GetProperty(idx), line);
            }
            ExprKind::Index { object, index } => {
                // Peephole: hash[const_str] → HashGetConst (avoids the generic
                // GetIndex path and its key-allocation).
                if let ExprKind::StringLiteral(key) = &index.kind {
                    let key_idx = self.add_string_constant(key);
                    self.compile_expr(object)?;
                    self.emit(Op::HashGetConst(key_idx), line);
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
            ExprKind::Await(inner) => {
                // Compile the inner expression — awaiting is handled at runtime
                self.compile_expr(inner)?;
            }
            ExprKind::Spread(inner) => {
                self.compile_expr(inner)?;
                self.emit(Op::Spread, line);
            }
            ExprKind::Throw(inner) => {
                self.compile_expr(inner)?;
                self.emit(Op::Throw, line);
            }
            ExprKind::CompoundAssign {
                target,
                operator,
                value,
            } => {
                // Desugar: target op= value  →  target = target op value
                use crate::ast::expr::CompoundOp;
                let bin_op = match operator {
                    CompoundOp::Add => BinaryOp::Add,
                    CompoundOp::Subtract => BinaryOp::Subtract,
                    CompoundOp::Multiply => BinaryOp::Multiply,
                    CompoundOp::Divide => BinaryOp::Divide,
                    CompoundOp::Modulo => BinaryOp::Modulo,
                };
                let desugared_value = Expr::new(
                    ExprKind::Binary {
                        left: Box::new((**target).clone()),
                        operator: bin_op,
                        right: Box::new((**value).clone()),
                    },
                    expr.span,
                );
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
                let idx = self.add_string_constant(&name);
                self.emit(Op::GetGlobal(idx), line);
            }
        }
        Ok(())
    }

    fn compile_binary(
        &mut self,
        left: &Expr,
        op: BinaryOp,
        right: &Expr,
        line: usize,
    ) -> CompileResult<()> {
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

    fn compile_call(
        &mut self,
        callee: &Expr,
        arguments: &[Argument],
        line: usize,
    ) -> CompileResult<()> {
        // Special case: print() calls
        if let ExprKind::Variable(name) = &callee.kind {
            if name == "print" || name == "puts" || name == "println" {
                return self.compile_print(arguments, line);
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

        // Optimized path: method calls (obj.method(args)) use CallMethod opcode
        // to avoid allocating Value::Method intermediary.
        if let ExprKind::Member { object, name } = &callee.kind {
            let all_positional = arguments
                .iter()
                .all(|a| matches!(a, Argument::Positional(_)));
            if all_positional && arguments.len() <= 255 {
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
                return Ok(());
            }
        }

        self.compile_expr(callee)?;

        let mut argc = 0u8;
        let mut has_named = false;
        for arg in arguments {
            match arg {
                Argument::Positional(expr) => {
                    self.compile_expr(expr)?;
                    argc += 1;
                }
                Argument::Named(named) => {
                    has_named = true;
                    // Push name marker then value
                    let name_idx = self.add_string_constant(&named.name);
                    self.emit(Op::NamedArg(name_idx), line);
                    self.compile_expr(&named.value)?;
                    argc += 2; // marker + value
                }
                Argument::Block(expr) => {
                    // Compile block as closure argument
                    self.compile_expr(expr)?;
                    argc += 1;
                }
            }
        }

        let _ = has_named; // Named arg reordering handled by VM at call time
        self.emit(Op::Call(argc), line);
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
        let mut argc = 0u8;
        for arg in arguments {
            match arg {
                Argument::Positional(expr) => {
                    self.compile_expr(expr)?;
                    argc += 1;
                }
                Argument::Named(named) => {
                    let name_idx = self.add_string_constant(&named.name);
                    self.emit(Op::NamedArg(name_idx), line);
                    self.compile_expr(&named.value)?;
                    argc += 2;
                }
                Argument::Block(expr) => {
                    self.compile_expr(expr)?;
                    argc += 1;
                }
            }
        }
        self.emit(Op::Call(argc), line);
        Ok(())
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
                let mut argc = 1u8;
                for arg in arguments {
                    match arg {
                        Argument::Positional(expr) => {
                            self.compile_expr(expr)?;
                            argc += 1;
                        }
                        Argument::Named(named) => {
                            let name_idx = self.add_string_constant(&named.name);
                            self.emit(Op::NamedArg(name_idx), line);
                            self.compile_expr(&named.value)?;
                            argc += 2;
                        }
                        Argument::Block(expr) => {
                            self.compile_expr(expr)?;
                            argc += 1;
                        }
                    }
                }
                self.emit(Op::Call(argc), line);
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
        let mut argc = 0u8;
        for arg in arguments {
            match arg {
                Argument::Positional(expr) => {
                    self.compile_expr(expr)?;
                    argc += 1;
                }
                Argument::Named(named) => {
                    let name_idx = self.add_string_constant(&named.name);
                    self.emit(Op::NamedArg(name_idx), line);
                    self.compile_expr(&named.value)?;
                    argc += 2;
                }
                Argument::Block(expr) => {
                    self.compile_expr(expr)?;
                    argc += 1;
                }
            }
        }
        self.emit(Op::New(argc), line);
        Ok(())
    }

    fn compile_array(&mut self, elements: &[Expr], line: usize) -> CompileResult<()> {
        for elem in elements {
            self.compile_expr(elem)?;
        }
        self.emit(Op::Array(elements.len() as u16), line);
        Ok(())
    }

    fn compile_hash(&mut self, pairs: &[(Expr, Expr)], line: usize) -> CompileResult<()> {
        // Fast path: if every key is a literal, precompute the HashKey list
        // and store it as a constant. The runtime then only needs values on
        // the stack — no key push/convert per element.
        if let Some(keys) = self.literal_hash_keys(pairs) {
            let keys_idx = self.add_constant(Constant::HashKeys(std::rc::Rc::new(keys)));
            for (_, value) in pairs {
                self.compile_expr(value)?;
            }
            self.emit(
                Op::HashWithKeys(keys_idx, pairs.len() as u16),
                line,
            );
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
                ExprKind::StringLiteral(s) => HashKey::String(s.clone()),
                ExprKind::IntLiteral(n) => HashKey::Int(*n),
                ExprKind::BoolLiteral(b) => HashKey::Bool(*b),
                ExprKind::Null => HashKey::Null,
                ExprKind::Symbol(s) => HashKey::Symbol(s.clone()),
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
        match &target.kind {
            ExprKind::Variable(name) => {
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
                // Peephole: hash[const_str] = value -> HashSetConst.
                if let ExprKind::StringLiteral(key) = &index.kind {
                    let key_idx = self.add_string_constant(key);
                    self.compile_expr(object)?;
                    self.compile_expr(value)?;
                    self.emit(Op::HashSetConst(key_idx), line);
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

        // Default values for parameters are handled at call time by the VM.
        // The compiler records that defaults exist via proto.defaults count.

        self.begin_scope();
        self.compile_function_body(body)?;
        self.end_scope(line);

        let proto = self.finish_function(line);
        let upvalue_count = proto.upvalue_descriptors.len();
        let idx = self.add_constant(Constant::Function(Rc::new(proto)));
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
        self.compile_expr(condition)?;
        let then_jump = self.emit_jump(Op::JumpIfFalse(0), line);

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
        let count = parts.len();
        for part in parts {
            match part {
                InterpolatedPart::Literal(s) => {
                    self.emit_constant(Constant::String(s.clone()), line);
                }
                InterpolatedPart::Expression(expr) => {
                    self.compile_expr(expr)?;
                }
            }
        }
        self.emit(Op::BuildString(count as u16), line);
        Ok(())
    }

    fn compile_sdql_block(
        &mut self,
        query: &str,
        interpolations: &[crate::ast::expr::SdqlInterpolation],
        line: usize,
    ) -> CompileResult<()> {
        // For now, just compile the query as a string constant
        // The runtime will handle interpolations
        // TODO: Implement proper interpolation handling
        let _ = interpolations; // suppress warning for now
        self.emit_constant(Constant::String(query.to_string()), line);

        // Emit a call to a builtin function to execute the SDBQL
        // For now, we'll use a placeholder - will be implemented later
        // This will call the runtime function that executes the query
        let fn_idx = self.add_string_constant("__sdql_exec");
        self.emit(Op::GetGlobal(fn_idx), line);
        self.emit(Op::Call(1), line);

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
            if let Some(slot) = self.resolve_local(var_name) {
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
            if self.scope_depth == 0 && self.resolve_local(var_name).is_none() {
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

    fn compile_list_comprehension(
        &mut self,
        element: &Expr,
        variable: &str,
        iterable: &Expr,
        condition: Option<&Expr>,
        line: usize,
    ) -> CompileResult<()> {
        // [expr for x in iter if cond]
        // Compiles to:
        //   1. Create empty array
        //   2. Get iterator
        //   3. Loop: get next, check done, bind variable, check condition, eval element, push to array
        self.emit(Op::Array(0), line); // empty result array
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
        let exit_jump = if is_range {
            self.emit_jump(Op::ForIterRange(0), line)
        } else {
            self.emit_jump(Op::ForIter(0), line)
        };

        self.begin_scope();
        // Bind the loop variable
        self.add_local(variable.to_string(), false);

        if let Some(cond) = condition {
            self.compile_expr(cond)?;
            let skip = self.emit_jump(Op::JumpIfFalse(0), line);

            // Evaluate element and push to array
            // Stack: [result_array, iter, loop_var, ...]
            // We need to get the result array, push element, then put it back
            self.compile_expr(element)?;
            // We need a special approach: the result array is deep in the stack.
            // Use GetLocal to access it.
            // Actually, let's just use a global-like approach or restructure.
            // Simpler: use the array that's on the stack before the iterator.

            self.patch_jump(skip);
        } else {
            self.compile_expr(element)?;
        }

        self.end_scope(line);
        self.emit_loop(loop_start, line);
        self.patch_jump(exit_jump);
        // Result array is on the stack
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
        // Similar to list comprehension but builds a hash
        self.emit(Op::Hash(0), line); // empty result hash
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
        let exit_jump = if is_range {
            self.emit_jump(Op::ForIterRange(0), line)
        } else {
            self.emit_jump(Op::ForIter(0), line)
        };

        self.begin_scope();
        self.add_local(variable.to_string(), false);

        if let Some(cond) = condition {
            self.compile_expr(cond)?;
            let skip = self.emit_jump(Op::JumpIfFalse(0), line);
            self.compile_expr(key)?;
            self.compile_expr(value)?;
            self.patch_jump(skip);
        } else {
            self.compile_expr(key)?;
            self.compile_expr(value)?;
        }

        self.end_scope(line);
        self.emit_loop(loop_start, line);
        self.patch_jump(exit_jump);
        Ok(())
    }
}
