//! Class declaration compilation.

use std::rc::Rc;

use crate::ast::stmt::{ClassDecl, ConstructorDecl, FieldDecl, MethodDecl};

use super::chunk::Constant;
use super::compiler::{CompileResult, Compiler, FunctionType};
use super::opcode::Op;

impl Compiler {
    /// Compile a class declaration.
    pub fn compile_class_decl(&mut self, decl: &ClassDecl, line: usize) -> CompileResult<()> {
        let name_idx = self.add_string_constant(&decl.name);

        // Create the class
        self.emit(Op::Class(name_idx), line);

        // Handle superclass
        if let Some(ref superclass_name) = decl.superclass {
            let super_idx = self.add_string_constant(superclass_name);
            self.emit(Op::GetGlobal(super_idx), line);
            self.emit(Op::Inherit, line);
        }

        // Store class context for this/super resolution
        let prev_class_ctx = self.class_context.take();
        self.class_context = Some(super::compiler::ClassContext {
            has_superclass: decl.superclass.is_some(),
        });

        // Compile fields with initializers
        for field in &decl.fields {
            self.compile_field(field, line)?;
        }

        // Compile methods
        for method in &decl.methods {
            self.compile_method(method, line)?;
        }

        // Compile constructor
        if let Some(ref ctor) = decl.constructor {
            self.compile_constructor(ctor, &decl.fields, line)?;
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

        // Compile nested classes
        for nested in &decl.nested_classes {
            self.emit(Op::Dup, line); // parent class on stack
            self.compile_class_decl(nested, line)?;
            // Store nested class as property of parent
            let nested_name_idx = self.add_string_constant(&nested.name);
            self.emit(Op::SetProperty(nested_name_idx), line);
        }

        // Restore class context
        self.class_context = prev_class_ctx;

        // Bind the class name
        if self.scope_depth > 0 {
            self.add_local(decl.name.clone(), false);
        } else {
            let gname = self.add_string_constant(&decl.name);
            self.emit(Op::DefineGlobal(gname), line);
        }

        Ok(())
    }

    fn compile_field(&mut self, field: &FieldDecl, line: usize) -> CompileResult<()> {
        let name_idx = self.add_string_constant(&field.name);

        if let Some(ref init) = field.initializer {
            self.compile_expr(init)?;
        } else {
            self.emit(Op::Null, line);
        }

        if field.is_static {
            self.emit(Op::StaticField(name_idx), line);
        } else {
            self.emit(Op::Field(name_idx), line);
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
        for stmt in &method.body {
            self.compile_stmt(stmt)?;
        }
        self.end_scope(line);

        let proto = self.finish_function(line);
        let fn_idx = self.add_constant(Constant::Function(Rc::new(proto)));
        self.emit(Op::Closure(fn_idx), line);

        let name_idx = self.add_string_constant(&method.name);
        if method.is_static {
            self.emit(Op::StaticMethod(name_idx), line);
        } else {
            self.emit(Op::Method(name_idx), line);
        }
        Ok(())
    }

    fn compile_constructor(
        &mut self,
        ctor: &ConstructorDecl,
        fields: &[FieldDecl],
        line: usize,
    ) -> CompileResult<()> {
        let _dummy = self.start_function(
            FunctionType::Constructor,
            "init".to_string(),
            &ctor.params,
        );

        self.begin_scope();

        // Initialize instance fields that have initializers
        for field in fields {
            if !field.is_static {
                if let Some(ref init) = field.initializer {
                    self.emit(Op::GetLocal(0), line); // this
                    self.compile_expr(init)?;
                    let name_idx = self.add_string_constant(&field.name);
                    self.emit(Op::SetProperty(name_idx), line);
                    self.emit(Op::Pop, line);
                }
            }
        }

        for stmt in &ctor.body {
            self.compile_stmt(stmt)?;
        }
        self.end_scope(line);

        // Constructor always returns `this`
        let proto = self.finish_constructor(line);
        let fn_idx = self.add_constant(Constant::Function(Rc::new(proto)));
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

        let mut proto = std::mem::replace(&mut self.proto, super::chunk::FunctionProto::new(String::new()));
        proto.upvalue_descriptors = std::mem::take(&mut self.upvalues);
        proto.is_method = true;

        if let Some(enclosing) = self.enclosing.take() {
            *self = *enclosing;
        }

        proto
    }
}
