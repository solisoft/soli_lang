//! Bytecode compiler: transforms AST into bytecode.

use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::expr::{BinaryOp, Expr, ExprKind, MatchPattern, UnaryOp};
use crate::ast::stmt::{
    ClassDecl, ConstructorDecl, FunctionDecl, MethodDecl, Parameter, Program, Stmt, StmtKind,
};
use crate::bytecode::chunk::{CompiledClass, CompiledFunction, Constant};
use crate::bytecode::instruction::{OpCode, UpvalueInfo};
use crate::error::CompileError;
use crate::span::Span;

/// Result type for compilation.
pub type CompileResult<T> = Result<T, CompileError>;

/// The bytecode compiler.
#[allow(dead_code)]
pub struct Compiler {
    /// Current function being compiled
    current: FunctionCompiler,
    /// Stack of enclosing function compilers (for nested functions)
    enclosing: Vec<FunctionCompiler>,
    /// Current class being compiled (if any)
    current_class: Option<ClassContext>,
    /// Global variables defined so far
    globals: HashMap<String, u16>,
    /// Native function registry (name -> index)
    natives: HashMap<String, u16>,
}

/// Context for compiling a single function.
#[allow(dead_code)]
struct FunctionCompiler {
    /// The function being compiled
    function: CompiledFunction,
    /// Local variables in current scope
    locals: Vec<Local>,
    /// Upvalues captured by this function
    upvalues: Vec<UpvalueInfo>,
    /// Current scope depth (0 = global)
    scope_depth: u32,
    /// Type of function being compiled
    function_type: FunctionType,
}

/// A local variable in a scope.
#[derive(Debug, Clone)]
struct Local {
    /// Variable name
    name: String,
    /// Scope depth where defined
    depth: u32,
    /// Whether captured by a closure
    is_captured: bool,
}

/// Type of function being compiled.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FunctionType {
    Script,
    Function,
    Method,
    Constructor,
    Lambda,
    Async,
}

/// Context for class compilation.
#[allow(dead_code)]
#[derive(Debug, Clone)]
struct ClassContext {
    /// Class name
    name: String,
    /// Whether this class has a superclass
    has_superclass: bool,
}

impl Compiler {
    /// Create a new compiler.
    pub fn new() -> Self {
        let mut compiler = Self {
            current: FunctionCompiler::new("<script>".to_string(), FunctionType::Script),
            enclosing: Vec::new(),
            current_class: None,
            globals: HashMap::new(),
            natives: HashMap::new(),
        };

        // Register built-in native functions
        compiler.register_natives();

        compiler
    }

    /// Register built-in native functions.
    fn register_natives(&mut self) {
        let natives = [
            "print",
            "println",
            "input",
            "len",
            "push",
            "pop",
            "shift",
            "unshift",
            "slice",
            "to_string",
            "str",
            "to_int",
            "to_float",
            "upcase",
            "downcase",
            "trim",
            "split",
            "join",
            "contains",
            "index_of",
            "substring",
            "map",
            "filter",
            "fold",
            "reverse",
            "sort",
            "type_of",
            "is_null",
            "now",
            "clock",
            "range",
            "abs",
            "min",
            "max",
            "floor",
            "ceil",
            "round",
            "sqrt",
            "pow",
            "keys",
            "values",
            "entries",
            "from_entries",
            "has_key",
            "delete",
            "merge",
            "clear",
            // HTTP functions
            "http_get",
            "http_post",
            "http_get_json",
            "http_post_json",
            "http_request",
            "json_parse",
            "json_stringify",
            "http_ok",
            "http_success",
            "http_redirect",
            "http_client_error",
            "http_server_error",
            // File I/O functions
            "barf",
            "slurp",
            // HTML functions
            "html_escape",
            "html_unescape",
            "sanitize_html",
            "strip_html",
            // Regex functions
            "regex_match",
            "regex_find",
            "regex_find_all",
            "regex_replace",
            "regex_replace_all",
            "regex_split",
            "regex_capture",
            "regex_escape",
            // DateTime functions
            "datetime_now",
            "datetime_parse",
            "datetime_utc",
            "duration_between",
            "duration_seconds",
            "duration_minutes",
            "duration_hours",
            "duration_days",
            "duration_weeks",
            // Async functions
            "await",
        ];

        for (i, name) in natives.iter().enumerate() {
            self.natives.insert(name.to_string(), i as u16);
        }
    }

    /// Compile a program into bytecode.
    pub fn compile(&mut self, program: &Program) -> CompileResult<CompiledFunction> {
        for stmt in &program.statements {
            self.compile_statement(stmt)?;
        }

        // Emit implicit return null at end of script
        self.emit_op(OpCode::Null, 0);
        self.emit_op(OpCode::Return, 0);

        // Set upvalue count
        self.current.function.upvalue_count = self.current.upvalues.len();

        Ok(self.current.function.clone())
    }

    /// Compile a statement.
    fn compile_statement(&mut self, stmt: &Stmt) -> CompileResult<()> {
        let line = stmt.span.line as u32;

        match &stmt.kind {
            StmtKind::Expression(expr) => {
                self.compile_expression(expr)?;
                self.emit_op(OpCode::Pop, line);
            }

            StmtKind::Let {
                name,
                initializer,
                type_annotation: _,
            } => {
                // Compile initializer or default to null
                if let Some(init) = initializer {
                    self.compile_expression(init)?;
                } else {
                    self.emit_op(OpCode::Null, line);
                }

                // Define variable
                if self.current.scope_depth > 0 {
                    // Local variable
                    self.declare_local(name.clone())?;
                    self.mark_initialized();
                } else {
                    // Global variable
                    let name_idx = self.identifier_constant(name);
                    self.emit_op(OpCode::DefineGlobal, line);
                    self.emit_u16(name_idx, line);
                }
            }

            StmtKind::Block(statements) => {
                self.begin_scope();
                for stmt in statements {
                    self.compile_statement(stmt)?;
                }
                self.end_scope(line);
            }

            StmtKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.compile_expression(condition)?;

                // Jump over then branch if false
                let then_jump = self.emit_jump(OpCode::JumpIfFalse, line);
                self.emit_op(OpCode::Pop, line); // Pop condition

                self.compile_statement(then_branch)?;

                if let Some(else_stmt) = else_branch {
                    // Jump over else branch
                    let else_jump = self.emit_jump(OpCode::Jump, line);
                    self.patch_jump(then_jump);
                    self.emit_op(OpCode::Pop, line); // Pop condition

                    self.compile_statement(else_stmt)?;
                    self.patch_jump(else_jump);
                } else {
                    self.patch_jump(then_jump);
                    self.emit_op(OpCode::Pop, line); // Pop condition
                }
            }

            StmtKind::While { condition, body } => {
                let loop_start = self.current_offset();

                self.compile_expression(condition)?;
                let exit_jump = self.emit_jump(OpCode::JumpIfFalse, line);
                self.emit_op(OpCode::Pop, line);

                self.compile_statement(body)?;

                self.emit_loop(loop_start, line);
                self.patch_jump(exit_jump);
                self.emit_op(OpCode::Pop, line);
            }

            StmtKind::For {
                variable,
                iterable,
                body,
            } => {
                // Compile: for (x in iterable) { body }
                // Into:
                //   <iterable>
                //   GET_ITERATOR
                //   LOOP_START:
                //   ITERATOR_NEXT <exit_jump>
                //   SET_LOCAL x
                //   <body>
                //   POP (value)
                //   LOOP -> LOOP_START
                //   EXIT:
                //   POP (iterator)

                self.begin_scope();

                // Compile iterable and get iterator
                self.compile_expression(iterable)?;
                self.emit_op(OpCode::GetIterator, line);

                // Declare loop variable
                self.declare_local(variable.clone())?;
                self.emit_op(OpCode::Null, line); // Placeholder for loop variable
                self.mark_initialized();
                let var_slot = self.current.locals.len() - 1;

                let loop_start = self.current_offset();

                // ITERATOR_NEXT pushes next value or jumps if done
                let exit_jump = self.emit_jump(OpCode::IteratorNext, line);

                // Store value in loop variable
                self.emit_op(OpCode::SetLocal, line);
                self.emit_u16(var_slot as u16, line);
                self.emit_op(OpCode::Pop, line);

                // Compile body
                self.compile_statement(body)?;

                // Loop back
                self.emit_loop(loop_start, line);

                // Exit point
                self.patch_jump(exit_jump);

                self.end_scope(line);
            }

            StmtKind::Return(expr) => {
                if let Some(e) = expr {
                    self.compile_expression(e)?;
                } else {
                    self.emit_op(OpCode::Null, line);
                }
                self.emit_op(OpCode::Return, line);
            }

            StmtKind::Function(decl) => {
                self.compile_function_decl(decl)?;
            }

            StmtKind::Class(decl) => {
                self.compile_class_decl(decl)?;
            }

            StmtKind::Interface(_) => {
                // Interfaces are only for type checking, no runtime representation needed
            }

            StmtKind::Import(_) => {
                // Imports are resolved before compilation by the ModuleResolver
                // The imported definitions are already in scope at compile time
            }

            StmtKind::Export(inner) => {
                // Export just compiles the inner declaration
                // The module system tracks what's exported separately
                self.compile_statement(inner)?;
            }

            StmtKind::Throw(value) => {
                let line = stmt.span.line as u32;
                self.compile_expression(value)?;
                self.emit_op(OpCode::Throw, line);
            }

            StmtKind::Try {
                try_block,
                catch_var,
                catch_block,
                finally_block,
            } => {
                let line = stmt.span.line as u32;

                // Mark try block start
                self.current_offset();

                // Compile try block
                self.compile_statement(try_block)?;

                // Emit TRY instruction with placeholder offsets
                // TRY <catch_offset_offset:u16> <finally_offset_offset:u16>
                // We store the offsets to where the jump targets will be written
                self.emit_op(OpCode::Try, line);
                self.emit_u16(0xFFFF, line); // catch_offset placeholder
                self.emit_u16(0xFFFF, line); // finally_offset placeholder

                // Record position to patch later
                let try_instruction_offset = self.current_offset() - 4;

                // Emit try end marker
                self.emit_op(OpCode::TryEnd, line);

                // Patch catch offset if there's a catch block
                if let Some(catch_blk) = catch_block {
                    let catch_offset = self.current_offset();
                    let chunk = &mut self.current.function.chunk;
                    chunk.patch_u16(try_instruction_offset + 2, catch_offset as u16);

                    // Declare catch variable if present
                    if let Some(var_name) = catch_var {
                        self.declare_local(var_name.clone())?;
                        self.mark_initialized();
                    }

                    self.compile_statement(catch_blk)?;

                    // Pop the try block
                    self.emit_op(OpCode::PopTry, line);
                }

                // Patch finally offset if there's a finally block
                if let Some(finally_blk) = finally_block {
                    let finally_offset = self.current_offset();
                    {
                        let chunk = &mut self.current.function.chunk;
                        chunk.patch_u16(try_instruction_offset + 4, finally_offset as u16);
                    }

                    // Compile finally block
                    self.compile_statement(finally_blk)?;
                } else if catch_block.is_none() {
                    // If only finally, patch it to current position
                    let current_offset = self.current_offset();
                    let chunk = &mut self.current.function.chunk;
                    chunk.patch_u16(try_instruction_offset + 4, current_offset as u16);
                }

                // If there's a catch block and also a finally, we need to jump over finally after catch
                if catch_block.is_some() && finally_block.is_some() {
                    let end_jump = self.emit_jump(OpCode::Jump, line);
                    self.patch_jump(end_jump);
                    self.emit_op(OpCode::PopTry, line);
                } else if catch_block.is_none() {
                    // Only finally - need to pop try
                    self.emit_op(OpCode::PopTry, line);
                }
            }
        }

        Ok(())
    }

    /// Compile an expression.
    fn compile_expression(&mut self, expr: &Expr) -> CompileResult<()> {
        let line = expr.span.line as u32;

        match &expr.kind {
            ExprKind::IntLiteral(n) => {
                let idx = self.add_constant(Constant::Int(*n));
                self.emit_op(OpCode::Constant, line);
                self.emit_u16(idx, line);
            }

            ExprKind::FloatLiteral(n) => {
                let idx = self.add_constant(Constant::Float(*n));
                self.emit_op(OpCode::Constant, line);
                self.emit_u16(idx, line);
            }

            ExprKind::StringLiteral(s) => {
                let idx = self.add_constant(Constant::String(s.clone()));
                self.emit_op(OpCode::Constant, line);
                self.emit_u16(idx, line);
            }

            ExprKind::BoolLiteral(b) => {
                if *b {
                    self.emit_op(OpCode::True, line);
                } else {
                    self.emit_op(OpCode::False, line);
                }
            }

            ExprKind::Null => {
                self.emit_op(OpCode::Null, line);
            }

            ExprKind::Variable(name) => {
                self.compile_variable_get(name, line)?;
            }

            ExprKind::Binary {
                left,
                operator,
                right,
            } => {
                self.compile_expression(left)?;
                self.compile_expression(right)?;

                match operator {
                    BinaryOp::Add => self.emit_op(OpCode::Add, line),
                    BinaryOp::Subtract => self.emit_op(OpCode::Subtract, line),
                    BinaryOp::Multiply => self.emit_op(OpCode::Multiply, line),
                    BinaryOp::Divide => self.emit_op(OpCode::Divide, line),
                    BinaryOp::Modulo => self.emit_op(OpCode::Modulo, line),
                    BinaryOp::Equal => self.emit_op(OpCode::Equal, line),
                    BinaryOp::NotEqual => self.emit_op(OpCode::NotEqual, line),
                    BinaryOp::Less => self.emit_op(OpCode::Less, line),
                    BinaryOp::LessEqual => self.emit_op(OpCode::LessEqual, line),
                    BinaryOp::Greater => self.emit_op(OpCode::Greater, line),
                    BinaryOp::GreaterEqual => self.emit_op(OpCode::GreaterEqual, line),
                    BinaryOp::Range => self.emit_op(OpCode::Range, line),
                }
            }

            ExprKind::Unary { operator, operand } => {
                self.compile_expression(operand)?;

                match operator {
                    UnaryOp::Negate => self.emit_op(OpCode::Negate, line),
                    UnaryOp::Not => self.emit_op(OpCode::Not, line),
                }
            }

            ExprKind::Grouping(inner) => {
                self.compile_expression(inner)?;
            }

            ExprKind::Call { callee, arguments } => {
                // Check if this is a native function call
                if let ExprKind::Variable(name) = &callee.kind {
                    if let Some(&native_idx) = self.natives.get(name) {
                        // Compile arguments
                        for arg in arguments {
                            self.compile_expression(arg)?;
                        }
                        // Emit native call
                        self.emit_op(OpCode::NativeCall, line);
                        self.emit_u16(native_idx, line);
                        self.emit_byte(arguments.len() as u8, line);
                        return Ok(());
                    }
                }

                // Regular function call
                self.compile_expression(callee)?;
                for arg in arguments {
                    self.compile_expression(arg)?;
                }
                self.emit_op(OpCode::Call, line);
                self.emit_byte(arguments.len() as u8, line);
            }

            ExprKind::Pipeline { left, right } => {
                // x |> f(a, b) becomes f(x, a, b)
                // x |> f becomes f(x)
                if let ExprKind::Call { callee, arguments } = &right.kind {
                    self.compile_expression(callee)?;
                    self.compile_expression(left)?;
                    for arg in arguments {
                        self.compile_expression(arg)?;
                    }
                    self.emit_op(OpCode::Call, line);
                    self.emit_byte((arguments.len() + 1) as u8, line);
                } else {
                    // Try to compile right as a function value
                    self.compile_expression(right)?;
                    self.compile_expression(left)?;
                    self.emit_op(OpCode::Call, line);
                    self.emit_byte(1, line);
                }
            }

            ExprKind::Member { object, name } => {
                self.compile_expression(object)?;
                let name_idx = self.identifier_constant(name);
                self.emit_op(OpCode::GetProperty, line);
                self.emit_u16(name_idx, line);
            }

            ExprKind::Index { object, index } => {
                self.compile_expression(object)?;
                self.compile_expression(index)?;
                self.emit_op(OpCode::Index, line);
            }

            ExprKind::This => {
                if self.current_class.is_none() {
                    return Err(CompileError::new(
                        "Cannot use 'this' outside of a class".to_string(),
                        expr.span,
                    ));
                }
                self.emit_op(OpCode::GetThis, line);
            }

            ExprKind::Super => {
                match &self.current_class {
                    None => {
                        return Err(CompileError::new(
                            "Cannot use 'super' outside of a class".to_string(),
                            expr.span,
                        ));
                    }
                    Some(ctx) if !ctx.has_superclass => {
                        return Err(CompileError::new(
                            "Cannot use 'super' in a class without a superclass".to_string(),
                            expr.span,
                        ));
                    }
                    _ => {}
                }
                self.emit_op(OpCode::GetSuper, line);
            }

            ExprKind::New {
                class_name,
                arguments,
            } => {
                // Compile arguments
                for arg in arguments {
                    self.compile_expression(arg)?;
                }
                // Emit new instance instruction
                let name_idx = self.identifier_constant(class_name);
                self.emit_op(OpCode::New, line);
                self.emit_u16(name_idx, line);
                self.emit_byte(arguments.len() as u8, line);
            }

            ExprKind::Array(elements) => {
                // For arrays with spread, we need special handling
                let has_spread = elements
                    .iter()
                    .any(|e| matches!(e.kind, ExprKind::Spread(_)));

                if has_spread {
                    // Build array with spread: [...a, b, ...c]
                    // Strategy: build a new array by iterating and spreading
                    // For simplicity, we collect non-spread elements, then spread the rest

                    // First, create an empty array
                    self.emit_op(OpCode::BuildArray, line);
                    self.emit_u16(0, line);

                    for elem in elements {
                        match &elem.kind {
                            ExprKind::Spread(inner) => {
                                // Compile the spread expression
                                self.compile_expression(inner)?;
                                // Spread the array onto the stack and append to our array
                                self.emit_op(OpCode::SpreadArray, line);
                            }
                            _ => {
                                // Regular element - push it
                                self.compile_expression(elem)?;
                                // We need to add this element to the array
                                // For now, let's just store it on stack and rebuild
                            }
                        }
                    }
                    // This is a simplified version - for full support we'd need
                    // a different opcode that builds arrays dynamically
                } else {
                    // Compile elements in order
                    for elem in elements {
                        self.compile_expression(elem)?;
                    }
                    self.emit_op(OpCode::BuildArray, line);
                    self.emit_u16(elements.len() as u16, line);
                }
            }

            ExprKind::Hash(pairs) => {
                // Compile key-value pairs
                for (key, value) in pairs {
                    self.compile_expression(key)?;
                    self.compile_expression(value)?;
                }
                self.emit_op(OpCode::BuildHash, line);
                self.emit_u16(pairs.len() as u16, line);
            }

            ExprKind::Assign { target, value } => match &target.kind {
                ExprKind::Variable(name) => {
                    self.compile_expression(value)?;
                    self.compile_variable_set(name, line)?;
                }
                ExprKind::Member { object, name } => {
                    self.compile_expression(object)?;
                    self.compile_expression(value)?;
                    let name_idx = self.identifier_constant(name);
                    self.emit_op(OpCode::SetProperty, line);
                    self.emit_u16(name_idx, line);
                }
                ExprKind::Index { object, index } => {
                    self.compile_expression(object)?;
                    self.compile_expression(index)?;
                    self.compile_expression(value)?;
                    self.emit_op(OpCode::IndexSet, line);
                }
                _ => {
                    return Err(CompileError::new(
                        "Invalid assignment target".to_string(),
                        target.span,
                    ));
                }
            },

            ExprKind::LogicalAnd { left, right } => {
                self.compile_expression(left)?;
                // Short-circuit: if left is false, skip right
                let jump = self.emit_jump(OpCode::JumpIfFalseNoPop, line);
                self.emit_op(OpCode::Pop, line);
                self.compile_expression(right)?;
                self.patch_jump(jump);
            }

            ExprKind::LogicalOr { left, right } => {
                self.compile_expression(left)?;
                // Short-circuit: if left is true, skip right
                let jump = self.emit_jump(OpCode::JumpIfTrueNoPop, line);
                self.emit_op(OpCode::Pop, line);
                self.compile_expression(right)?;
                self.patch_jump(jump);
            }

            ExprKind::Lambda {
                params,
                return_type: _,
                body,
            } => {
                self.compile_lambda(params, body)?;
            }

            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.compile_expression(condition)?;

                let else_jump = self.emit_jump(OpCode::JumpIfFalse, line);
                self.patch_jump(else_jump);

                self.compile_expression(then_branch)?;

                if else_branch.is_some() {
                    let end_jump = self.emit_jump(OpCode::Jump, line);

                    self.patch_jump(else_jump);
                    self.compile_expression(else_branch.as_ref().unwrap())?;

                    self.patch_jump(end_jump);
                } else {
                    self.patch_jump(else_jump);
                }
            }

            ExprKind::InterpolatedString(parts) => {
                // Compile each part and concatenate
                let mut first = true;
                for part in parts {
                    match part {
                        crate::ast::expr::InterpolatedPart::Literal(s) => {
                            if !s.is_empty() {
                                if !first {
                                    // Add string concatenation
                                    self.emit_op(OpCode::Add, line);
                                }
                                let idx = self.add_constant(Constant::String(s.clone()));
                                self.emit_op(OpCode::Constant, line);
                                self.emit_u16(idx, line);
                                first = false;
                            }
                        }
                        crate::ast::expr::InterpolatedPart::Expression(expr) => {
                            self.compile_expression(expr)?;
                            if !first {
                                self.emit_op(OpCode::Add, line);
                            }
                            first = false;
                        }
                    }
                }
                // If the string was empty or started with interpolation
                if first {
                    // Push empty string
                    let idx = self.add_constant(Constant::String(String::new()));
                    self.emit_op(OpCode::Constant, line);
                    self.emit_u16(idx, line);
                }
            }
            ExprKind::Match { expression, arms } => {
                let line = expr.span.line as u32;

                self.compile_expression(expression)?;

                let mut pattern_jump_offsets = Vec::new();
                let mut end_jump_offsets = Vec::new();

                for arm in arms {
                    self.compile_match_pattern(&arm.pattern, line)?;

                    let jump_offset = self.emit_jump(OpCode::JumpIfFalse, line);
                    pattern_jump_offsets.push(jump_offset);

                    if let Some(guard) = &arm.guard {
                        self.compile_expression(guard)?;
                    }

                    self.compile_expression(&arm.body)?;

                    if arm != arms.last().unwrap() {
                        let next_jump = self.emit_jump(OpCode::Jump, line);
                        end_jump_offsets.push(next_jump);
                    }
                }

                for offset in pattern_jump_offsets {
                    self.patch_jump(offset);
                }

                for offset in end_jump_offsets {
                    self.patch_jump(offset);
                }
            }
            ExprKind::ListComprehension { .. } => {
                unimplemented!("List comprehensions not yet implemented in compiler")
            }
            ExprKind::HashComprehension { .. } => {
                unimplemented!("Hash comprehensions not yet implemented in compiler")
            }
            ExprKind::Await(_) => {
                unimplemented!("Await expressions not yet implemented in compiler")
            }
            ExprKind::Spread(_) => {
                unimplemented!("Spread expressions not yet implemented in compiler")
            }
            ExprKind::Throw(value) => {
                let line = expr.span.line as u32;
                self.compile_expression(value)?;
                self.emit_op(OpCode::Throw, line);
            }
        }

        Ok(())
    }

    fn compile_match_pattern(&mut self, pattern: &MatchPattern, line: u32) -> CompileResult<()> {
        match pattern {
            MatchPattern::Wildcard => {
                self.emit_op(OpCode::Pop, line);
                self.emit_op(OpCode::True, line);
                Ok(())
            }

            MatchPattern::Variable(_name) => {
                self.emit_op(OpCode::Dup, line);
                self.emit_op(OpCode::True, line);
                Ok(())
            }

            MatchPattern::Typed { type_name, .. } => {
                let type_idx = self.add_constant(Constant::String(type_name.clone()));
                self.emit_op(OpCode::TypeCheck, line);
                self.emit_u16(type_idx, line);
                Ok(())
            }

            MatchPattern::Array { .. } => {
                self.emit_op(OpCode::Dup, line);
                let type_idx = self.add_constant(Constant::String("Array".to_string()));
                self.emit_op(OpCode::TypeCheck, line);
                self.emit_u16(type_idx, line);
                let fail_jump = self.emit_jump(OpCode::JumpIfFalse, line);
                self.emit_op(OpCode::Pop, line);
                self.emit_op(OpCode::False, line);
                self.patch_jump(fail_jump);
                Ok(())
            }

            MatchPattern::Literal(literal) => self.compile_literal_comparison(literal, line),

            MatchPattern::Hash { .. } => {
                self.emit_op(OpCode::Dup, line);
                let type_idx = self.add_constant(Constant::String("Hash".to_string()));
                self.emit_op(OpCode::TypeCheck, line);
                self.emit_u16(type_idx, line);
                let fail_jump = self.emit_jump(OpCode::JumpIfFalse, line);
                self.emit_op(OpCode::Pop, line);
                self.emit_op(OpCode::False, line);
                self.patch_jump(fail_jump);
                Ok(())
            }

            MatchPattern::Destructuring { type_name, .. } => {
                let type_idx = self.add_constant(Constant::String(type_name.clone()));
                self.emit_op(OpCode::TypeCheck, line);
                self.emit_u16(type_idx, line);
                Ok(())
            }

            MatchPattern::And(patterns) => {
                for p in patterns {
                    self.compile_match_pattern(p, line)?;
                }
                Ok(())
            }

            MatchPattern::Or(patterns) => {
                for p in patterns {
                    self.compile_match_pattern(p, line)?;
                }
                Ok(())
            }
        }
    }

    fn compile_literal_comparison(&mut self, literal: &ExprKind, line: u32) -> CompileResult<()> {
        match literal {
            ExprKind::IntLiteral(n) => {
                let idx = self.add_constant(Constant::Int(*n));
                self.emit_op(OpCode::Constant, line);
                self.emit_u16(idx, line);
                self.emit_op(OpCode::Equal, line);
            }
            ExprKind::FloatLiteral(n) => {
                let idx = self.add_constant(Constant::Float(*n));
                self.emit_op(OpCode::Constant, line);
                self.emit_u16(idx, line);
                self.emit_op(OpCode::Equal, line);
            }
            ExprKind::StringLiteral(s) => {
                let idx = self.add_constant(Constant::String(s.clone()));
                self.emit_op(OpCode::Constant, line);
                self.emit_u16(idx, line);
                self.emit_op(OpCode::Equal, line);
            }
            ExprKind::BoolLiteral(b) => {
                if *b {
                    self.emit_op(OpCode::True, line);
                } else {
                    self.emit_op(OpCode::False, line);
                }
                self.emit_op(OpCode::Equal, line);
            }
            ExprKind::Null => {
                self.emit_op(OpCode::Null, line);
                self.emit_op(OpCode::Equal, line);
            }
            _ => {
                self.emit_op(OpCode::True, line);
            }
        }
        Ok(())
    }

    /// Compile a variable get operation.
    fn compile_variable_get(&mut self, name: &str, line: u32) -> CompileResult<()> {
        // Check for local variable
        if let Some(slot) = self.resolve_local(name) {
            self.emit_op(OpCode::GetLocal, line);
            self.emit_u16(slot as u16, line);
            return Ok(());
        }

        // Check for upvalue
        if let Some(idx) = self.resolve_upvalue(name)? {
            self.emit_op(OpCode::GetUpvalue, line);
            self.emit_byte(idx, line);
            return Ok(());
        }

        // Global variable
        let name_idx = self.identifier_constant(name);
        self.emit_op(OpCode::GetGlobal, line);
        self.emit_u16(name_idx, line);
        Ok(())
    }

    /// Compile a variable set operation.
    fn compile_variable_set(&mut self, name: &str, line: u32) -> CompileResult<()> {
        // Check for local variable
        if let Some(slot) = self.resolve_local(name) {
            self.emit_op(OpCode::SetLocal, line);
            self.emit_u16(slot as u16, line);
            return Ok(());
        }

        // Check for upvalue
        if let Some(idx) = self.resolve_upvalue(name)? {
            self.emit_op(OpCode::SetUpvalue, line);
            self.emit_byte(idx, line);
            return Ok(());
        }

        // Global variable
        let name_idx = self.identifier_constant(name);
        self.emit_op(OpCode::SetGlobal, line);
        self.emit_u16(name_idx, line);
        Ok(())
    }

    /// Compile a function declaration.
    fn compile_function_decl(&mut self, decl: &FunctionDecl) -> CompileResult<()> {
        let line = decl.span.line as u32;

        // Define the function name in current scope first (for recursion)
        if self.current.scope_depth > 0 {
            self.declare_local(decl.name.clone())?;
            self.mark_initialized();
        }

        // Calculate required arity (params without defaults)
        let required_arity = decl
            .params
            .iter()
            .filter(|p| p.default_value.is_none())
            .count();
        let full_arity = decl.params.len();

        // Compile the function body
        self.begin_function(&decl.name, required_arity as u8, FunctionType::Function);
        self.current.function.full_arity = full_arity as u8;

        // Store default value constant indices
        let mut default_value_indices: Vec<Option<u16>> = Vec::with_capacity(full_arity);

        // First, add all default values as constants (before compiling body)
        // This ensures they're at the end of the constant pool
        for param in &decl.params {
            if let Some(ref default_expr) = param.default_value {
                // Compile the default expression to get its value
                // We need to evaluate it at compile time for simple constants
                let const_idx = self.compile_default_value(default_expr)?;
                default_value_indices.push(Some(const_idx));
            } else {
                default_value_indices.push(None);
            }
        }
        self.current.function.default_values = default_value_indices;

        // Add parameters as locals
        for param in &decl.params {
            self.declare_local(param.name.clone())?;
            self.mark_initialized();
        }

        // Compile body
        for stmt in &decl.body {
            self.compile_statement(stmt)?;
        }

        // Implicit return null
        self.emit_op(OpCode::Null, line);
        self.emit_op(OpCode::Return, line);

        let (function, upvalues) = self.end_function();

        // Emit closure
        let func_idx = self.add_constant(Constant::Function(Rc::new(function)));
        self.emit_op(OpCode::Closure, line);
        self.emit_u16(func_idx, line);

        // Emit upvalue info
        for upvalue in &upvalues {
            self.emit_byte(if upvalue.is_local { 1 } else { 0 }, line);
            self.emit_byte(upvalue.index, line);
        }

        // Store in global or leave on stack for local
        if self.current.scope_depth == 0 {
            let name_idx = self.identifier_constant(&decl.name);
            self.emit_op(OpCode::DefineGlobal, line);
            self.emit_u16(name_idx, line);
        }

        Ok(())
    }

    /// Compile a default value expression and return its constant index
    fn compile_default_value(&mut self, expr: &Expr) -> CompileResult<u16> {
        match &expr.kind {
            ExprKind::IntLiteral(n) => {
                let idx = self.add_constant(Constant::Int(*n));
                Ok(idx)
            }
            ExprKind::FloatLiteral(n) => {
                let idx = self.add_constant(Constant::Float(*n));
                Ok(idx)
            }
            ExprKind::StringLiteral(s) => {
                let idx = self.add_constant(Constant::String(s.clone()));
                Ok(idx)
            }
            ExprKind::BoolLiteral(b) => {
                // Booleans aren't stored as constants, but we can use Int
                let idx = self.add_constant(Constant::Int(if *b { 1 } else { 0 }));
                Ok(idx)
            }
            ExprKind::Null => {
                let idx = self.add_constant(Constant::Null);
                Ok(idx)
            }
            // For complex expressions, we'll need runtime evaluation
            // For now, use null as a placeholder
            _ => {
                let idx = self.add_constant(Constant::Null);
                Ok(idx)
            }
        }
    }

    /// Compile a lambda expression.
    fn compile_lambda(&mut self, params: &[Parameter], body: &[Stmt]) -> CompileResult<()> {
        let line = if !body.is_empty() {
            body[0].span.line as u32
        } else {
            0
        };

        self.begin_function("<lambda>", params.len() as u8, FunctionType::Lambda);

        // Add parameters as locals
        for param in params {
            self.declare_local(param.name.clone())?;
            self.mark_initialized();
        }

        // Compile body statements
        for stmt in body {
            self.compile_statement(stmt)?;
        }

        // Implicit return null
        self.emit_op(OpCode::Null, line);
        self.emit_op(OpCode::Return, line);

        let (function, upvalues) = self.end_function();

        // Emit closure
        let func_idx = self.add_constant(Constant::Function(Rc::new(function)));
        self.emit_op(OpCode::Closure, line);
        self.emit_u16(func_idx, line);

        // Emit upvalue info
        for upvalue in &upvalues {
            self.emit_byte(if upvalue.is_local { 1 } else { 0 }, line);
            self.emit_byte(upvalue.index, line);
        }

        Ok(())
    }

    /// Compile a class declaration.
    fn compile_class_decl(&mut self, decl: &ClassDecl) -> CompileResult<()> {
        let line = decl.span.line as u32;
        let class_name = decl.name.clone();

        // Define class name first (for self-reference)
        if self.current.scope_depth > 0 {
            self.declare_local(class_name.clone())?;
            self.mark_initialized();
        }

        // Create class constant
        let mut compiled_class = CompiledClass::new(class_name.clone());
        compiled_class.superclass = decl.superclass.clone();

        // Set up class context
        let has_superclass = decl.superclass.is_some();
        let old_class = self.current_class.replace(ClassContext {
            name: class_name.clone(),
            has_superclass,
        });

        // Compile superclass
        if let Some(ref superclass_name) = decl.superclass {
            // Load superclass
            self.compile_variable_get(superclass_name, line)?;
        }

        // Emit class creation
        let class_name_idx = self.identifier_constant(&class_name);
        self.emit_op(OpCode::Class, line);
        self.emit_u16(class_name_idx, line);

        // Handle inheritance
        if has_superclass {
            self.emit_op(OpCode::Inherit, line);
        }

        // Compile constructor
        if let Some(ref constructor) = decl.constructor {
            self.compile_constructor(constructor)?;
        }

        // Compile methods
        for method in &decl.methods {
            self.compile_method(method)?;
        }

        // Compile field initializers (if any default values)
        for _field in &decl.fields {
            // Fields with default values are handled in constructor
        }

        // Restore class context
        self.current_class = old_class;

        // Store in global or leave on stack for local
        if self.current.scope_depth == 0 {
            let name_idx = self.identifier_constant(&decl.name);
            self.emit_op(OpCode::DefineGlobal, line);
            self.emit_u16(name_idx, line);
        }

        Ok(())
    }

    /// Compile a method.
    fn compile_method(&mut self, method: &MethodDecl) -> CompileResult<()> {
        let line = method.span.line as u32;
        let func_type = FunctionType::Method;

        self.begin_function(&method.name, method.params.len() as u8, func_type);
        self.current.function.is_method = true;

        // 'this' is local 0 in methods
        self.declare_local("this".to_string())?;
        self.mark_initialized();

        // Add parameters
        for param in &method.params {
            self.declare_local(param.name.clone())?;
            self.mark_initialized();
        }

        // Compile body
        for stmt in &method.body {
            self.compile_statement(stmt)?;
        }

        // Implicit return null
        self.emit_op(OpCode::Null, line);
        self.emit_op(OpCode::Return, line);

        let (function, upvalues) = self.end_function();

        // Emit closure
        let func_idx = self.add_constant(Constant::Function(Rc::new(function)));
        self.emit_op(OpCode::Closure, line);
        self.emit_u16(func_idx, line);

        for upvalue in &upvalues {
            self.emit_byte(if upvalue.is_local { 1 } else { 0 }, line);
            self.emit_byte(upvalue.index, line);
        }

        // Define method on class
        let method_name_idx = self.identifier_constant(&method.name);
        if method.is_static {
            self.emit_op(OpCode::StaticMethod, line);
        } else {
            self.emit_op(OpCode::Method, line);
        }
        self.emit_u16(method_name_idx, line);

        Ok(())
    }

    /// Compile a constructor.
    fn compile_constructor(&mut self, constructor: &ConstructorDecl) -> CompileResult<()> {
        let line = constructor.span.line as u32;

        self.begin_function(
            "constructor",
            constructor.params.len() as u8,
            FunctionType::Constructor,
        );
        self.current.function.is_method = true;

        // 'this' is local 0
        self.declare_local("this".to_string())?;
        self.mark_initialized();

        // Add parameters
        for param in &constructor.params {
            self.declare_local(param.name.clone())?;
            self.mark_initialized();
        }

        // Compile body
        for stmt in &constructor.body {
            self.compile_statement(stmt)?;
        }

        // Constructors return 'this'
        self.emit_op(OpCode::GetLocal, line);
        self.emit_u16(0, line); // slot 0 = this
        self.emit_op(OpCode::Return, line);

        let (function, upvalues) = self.end_function();

        // Emit closure
        let func_idx = self.add_constant(Constant::Function(Rc::new(function)));
        self.emit_op(OpCode::Closure, line);
        self.emit_u16(func_idx, line);

        for upvalue in &upvalues {
            self.emit_byte(if upvalue.is_local { 1 } else { 0 }, line);
            self.emit_byte(upvalue.index, line);
        }

        // Set as constructor
        let name_idx = self.identifier_constant("constructor");
        self.emit_op(OpCode::Method, line);
        self.emit_u16(name_idx, line);

        Ok(())
    }

    // ===== Scope management =====

    fn begin_scope(&mut self) {
        self.current.scope_depth += 1;
    }

    fn end_scope(&mut self, line: u32) {
        self.current.scope_depth -= 1;

        // Pop locals going out of scope
        while let Some(local) = self.current.locals.last() {
            if local.depth <= self.current.scope_depth {
                break;
            }

            if local.is_captured {
                self.emit_op(OpCode::CloseUpvalue, line);
            } else {
                self.emit_op(OpCode::Pop, line);
            }
            self.current.locals.pop();
        }
    }

    fn declare_local(&mut self, name: String) -> CompileResult<()> {
        // Check for duplicate in current scope
        for local in self.current.locals.iter().rev() {
            if local.depth < self.current.scope_depth {
                break;
            }
            if local.name == name {
                return Err(CompileError::new(
                    format!("Variable '{}' already declared in this scope", name),
                    Span::default(),
                ));
            }
        }

        self.current.locals.push(Local {
            name,
            depth: u32::MAX, // Not yet initialized
            is_captured: false,
        });
        Ok(())
    }

    fn mark_initialized(&mut self) {
        if let Some(local) = self.current.locals.last_mut() {
            local.depth = self.current.scope_depth;
        }
    }

    fn resolve_local(&self, name: &str) -> Option<usize> {
        for (i, local) in self.current.locals.iter().enumerate().rev() {
            if local.name == name && local.depth != u32::MAX {
                return Some(i);
            }
        }
        None
    }

    fn resolve_upvalue(&mut self, name: &str) -> CompileResult<Option<u8>> {
        if self.enclosing.is_empty() {
            return Ok(None);
        }

        // Check enclosing function's locals
        let enclosing = self.enclosing.last_mut().unwrap();
        for (i, local) in enclosing.locals.iter_mut().enumerate().rev() {
            if local.name == name && local.depth != u32::MAX {
                local.is_captured = true;
                return Ok(Some(self.add_upvalue(i as u8, true)?));
            }
        }

        // Check enclosing function's upvalues (recursively)
        // Note: this is a simplified version; full implementation would be recursive
        for (i, upvalue) in enclosing.upvalues.iter().enumerate() {
            // This is a simplification; full closure support would need more work
            let _ = upvalue;
            let _ = i;
        }

        Ok(None)
    }

    fn add_upvalue(&mut self, index: u8, is_local: bool) -> CompileResult<u8> {
        // Check if already captured
        for (i, upvalue) in self.current.upvalues.iter().enumerate() {
            if upvalue.index == index && upvalue.is_local == is_local {
                return Ok(i as u8);
            }
        }

        let upvalue_count = self.current.upvalues.len();
        if upvalue_count >= 256 {
            return Err(CompileError::new(
                "Too many upvalues in function".to_string(),
                Span::default(),
            ));
        }

        self.current
            .upvalues
            .push(UpvalueInfo::new(is_local, index));
        Ok(upvalue_count as u8)
    }

    // ===== Function compilation =====

    fn begin_function(&mut self, name: &str, arity: u8, function_type: FunctionType) {
        let new_compiler = FunctionCompiler::new(name.to_string(), function_type);
        let old_compiler = std::mem::replace(&mut self.current, new_compiler);
        self.enclosing.push(old_compiler);
        self.current.function.arity = arity;
    }

    fn end_function(&mut self) -> (CompiledFunction, Vec<UpvalueInfo>) {
        self.current.function.upvalue_count = self.current.upvalues.len();
        let upvalues = std::mem::take(&mut self.current.upvalues);
        let function = self.current.function.clone();

        if let Some(enclosing) = self.enclosing.pop() {
            self.current = enclosing;
        }

        (function, upvalues)
    }

    // ===== Bytecode emission =====

    fn emit_op(&mut self, op: OpCode, line: u32) {
        self.current.function.chunk.write_op(op, line);
    }

    fn emit_byte(&mut self, byte: u8, line: u32) {
        self.current.function.chunk.write_byte(byte, line);
    }

    fn emit_u16(&mut self, value: u16, line: u32) {
        self.current.function.chunk.write_u16(value, line);
    }

    fn emit_jump(&mut self, op: OpCode, line: u32) -> usize {
        self.emit_op(op, line);
        let offset = self.current.function.chunk.current_offset();
        self.emit_u16(0xFFFF, line); // Placeholder
        offset
    }

    fn patch_jump(&mut self, offset: usize) {
        self.current.function.chunk.patch_jump(offset);
    }

    fn emit_loop(&mut self, loop_start: usize, line: u32) {
        self.emit_op(OpCode::Loop, line);
        let offset = self.current.function.chunk.current_offset() + 2 - loop_start;
        assert!(offset < 65536, "Loop body too large");
        self.emit_u16(offset as u16, line);
    }

    fn current_offset(&self) -> usize {
        self.current.function.chunk.current_offset()
    }

    fn add_constant(&mut self, constant: Constant) -> u16 {
        self.current.function.chunk.add_constant(constant)
    }

    fn identifier_constant(&mut self, name: &str) -> u16 {
        self.add_constant(Constant::String(name.to_string()))
    }
}

impl Default for Compiler {
    fn default() -> Self {
        Self::new()
    }
}

impl FunctionCompiler {
    fn new(name: String, function_type: FunctionType) -> Self {
        Self {
            function: CompiledFunction::new(name, 0),
            locals: Vec::new(),
            upvalues: Vec::new(),
            scope_depth: if function_type == FunctionType::Script {
                0
            } else {
                1
            },
            function_type,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::stmt::Program;

    fn compile_source(source: &str) -> CompileResult<CompiledFunction> {
        let tokens = crate::lexer::Scanner::new(source)
            .scan_tokens()
            .map_err(|e| CompileError::new(e.to_string(), Span::default()))?;
        let program = crate::parser::Parser::new(tokens)
            .parse()
            .map_err(|e| CompileError::new(e.to_string(), Span::default()))?;

        let mut compiler = Compiler::new();
        compiler.compile(&program)
    }

    #[test]
    fn test_compile_simple_expression() {
        let result = compile_source("1 + 2;");
        assert!(result.is_ok());
    }

    #[test]
    fn test_compile_variable() {
        let result = compile_source("let x = 42;");
        assert!(result.is_ok());
    }

    #[test]
    fn test_compile_function() {
        let result = compile_source("fn add(a: Int, b: Int) -> Int { return a + b; }");
        assert!(result.is_ok());
    }
}
