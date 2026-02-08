//! Class operations for the VM: property access, inheritance, instantiation.

use std::cell::RefCell;
use std::rc::Rc;

use crate::error::RuntimeError;
use crate::interpreter::value::{Class, HashKey, Instance, Value, ValueMethod};
use crate::span::Span;

use super::vm::Vm;

impl Vm {
    /// Get a property from a value.
    pub fn op_get_property(
        &self,
        object: &Value,
        name: &str,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        match object {
            Value::Instance(inst) => {
                let inst = inst.borrow();
                // Check instance fields first
                if let Some(val) = inst.fields.get(name) {
                    return Ok(val.clone());
                }
                // Check class methods
                if let Some(method) = inst.class.find_method(name) {
                    return Ok(Value::Function(method));
                }
                // Check native methods
                if let Some(native) = inst.class.find_native_method(name) {
                    return Ok(Value::NativeFunction((*native).clone()));
                }
                Err(RuntimeError::NoSuchProperty {
                    value_type: inst.class.name.clone(),
                    property: name.to_string(),
                    span,
                })
            }
            Value::Class(class) => {
                // Static field access
                if let Some(val) = class.static_fields.borrow().get(name) {
                    return Ok(val.clone());
                }
                // Static method access
                if let Some(method) = class.find_static_method(name) {
                    return Ok(Value::Function(method));
                }
                // Native static method
                if let Some(native) = class.find_native_static_method(name) {
                    return Ok(Value::NativeFunction((*native).clone()));
                }
                // Nested class
                if let Some(nested) = class.nested_classes.borrow().get(name) {
                    return Ok(Value::Class(nested.clone()));
                }
                Err(RuntimeError::NoSuchProperty {
                    value_type: class.name.clone(),
                    property: name.to_string(),
                    span,
                })
            }
            Value::Hash(hash) => {
                let hash = hash.borrow();
                if let Some(val) = hash.get(&HashKey::String(name.to_string())) {
                    Ok(val.clone())
                } else {
                    // Hash methods like .keys, .values, .length, etc.
                    Ok(Value::Method(ValueMethod {
                        receiver: Box::new(object.clone()),
                        method_name: name.to_string(),
                    }))
                }
            }
            Value::Array(_) => {
                // Array methods like .length, .map, .filter, etc.
                Ok(Value::Method(ValueMethod {
                    receiver: Box::new(object.clone()),
                    method_name: name.to_string(),
                }))
            }
            Value::String(s) => {
                // String properties
                if name == "length" {
                    Ok(Value::Int(s.len() as i64))
                } else {
                    Ok(Value::Method(ValueMethod {
                        receiver: Box::new(object.clone()),
                        method_name: name.to_string(),
                    }))
                }
            }
            Value::Super(superclass) => {
                // super.method() — look up method in superclass
                if let Some(method) = superclass.find_method(name) {
                    return Ok(Value::Function(method));
                }
                if let Some(native) = superclass.find_native_method(name) {
                    return Ok(Value::NativeFunction((*native).clone()));
                }
                Err(RuntimeError::NoSuchProperty {
                    value_type: format!("super({})", superclass.name),
                    property: name.to_string(),
                    span,
                })
            }
            _ => Err(RuntimeError::NoSuchProperty {
                value_type: object.type_name(),
                property: name.to_string(),
                span,
            }),
        }
    }

    /// Set a property on a value.
    pub fn op_set_property(
        &self,
        object: &Value,
        name: &str,
        value: Value,
        span: Span,
    ) -> Result<(), RuntimeError> {
        match object {
            Value::Instance(inst) => {
                inst.borrow_mut().fields.insert(name.to_string(), value);
                Ok(())
            }
            Value::Class(class) => {
                class
                    .static_fields
                    .borrow_mut()
                    .insert(name.to_string(), value);
                Ok(())
            }
            Value::Hash(hash) => {
                hash.borrow_mut()
                    .insert(HashKey::String(name.to_string()), value);
                Ok(())
            }
            _ => Err(RuntimeError::type_error(
                format!("Cannot set property on {}", object.type_name()),
                span,
            )),
        }
    }

    /// Set up inheritance between subclass and superclass.
    pub fn op_inherit(
        &mut self,
        subclass_val: &Value,
        superclass_val: &Value,
        span: Span,
    ) -> Result<(), RuntimeError> {
        let superclass = match superclass_val {
            Value::Class(c) => c.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    format!(
                        "Superclass must be a class, got {}",
                        superclass_val.type_name()
                    ),
                    span,
                ));
            }
        };

        // We need to reconstruct the subclass with the superclass set.
        // Since Class is in an Rc, we need to create a new one.
        // The class was just created by Op::Class, so we replace the top of stack.
        if let Value::Class(sub) = subclass_val {
            let new_class = Class::new(
                sub.name.clone(),
                Some(superclass.clone()),
                sub.methods.clone(),
                sub.static_methods.clone(),
                sub.native_static_methods.clone(),
                sub.native_methods.clone(),
                sub.static_fields.clone(),
                sub.fields.clone(),
                sub.constructor.clone(),
                sub.nested_classes.clone(),
            );
            // Replace the class on top of the stack
            let top = self.stack.len() - 1;
            self.stack[top] = Value::Class(Rc::new(new_class));
            Ok(())
        } else {
            Err(RuntimeError::type_error(
                format!("Expected class, got {}", subclass_val.type_name()),
                span,
            ))
        }
    }

    /// Add a method to a class on top of the stack.
    pub fn op_add_method(
        &mut self,
        class_val: &Value,
        name: &str,
        method: Value,
        is_static: bool,
        span: Span,
    ) -> Result<(), RuntimeError> {
        if let Value::Class(_class) = class_val {
            // Since Class is behind Rc, we need to reconstruct with the new method.
            // For mutability, we'll use a different approach: store VM methods separately.
            // For now, we use the approach of rebuilding the class.
            // This is fine since class setup only happens once at startup.

            // Get the current class from the stack
            let top = self.stack.len() - 1;
            if let Value::Class(current) = &self.stack[top] {
                let mut methods = current.methods.clone();
                let mut static_methods = current.static_methods.clone();

                match method {
                    Value::VmClosure(_closure) => {
                        // VmClosure methods will be dispatched by the VM at call time.
                        // TODO: Add vm_methods field to Class for native VM method storage
                    }
                    Value::Function(func) => {
                        if is_static {
                            static_methods.insert(name.to_string(), func);
                        } else {
                            methods.insert(name.to_string(), func);
                        }
                    }
                    _ => {}
                }

                let new_class = Class::new(
                    current.name.clone(),
                    current.superclass.clone(),
                    methods,
                    static_methods,
                    current.native_static_methods.clone(),
                    current.native_methods.clone(),
                    current.static_fields.clone(),
                    current.fields.clone(),
                    current.constructor.clone(),
                    current.nested_classes.clone(),
                );
                self.stack[top] = Value::Class(Rc::new(new_class));
            }
            Ok(())
        } else {
            Err(RuntimeError::type_error(
                format!("Expected class, got {}", class_val.type_name()),
                span,
            ))
        }
    }

    /// Instantiate a class with constructor arguments.
    pub fn op_new(&mut self, argc: usize, span: Span) -> Result<(), RuntimeError> {
        let callee_idx = self.stack.len() - 1 - argc;
        let class_val = self.stack[callee_idx].clone();

        match class_val {
            Value::Class(class) => {
                let instance = Instance::new(class.clone());
                let instance_val = Value::Instance(Rc::new(RefCell::new(instance)));

                // Replace class with instance on the stack
                self.stack[callee_idx] = instance_val.clone();

                // Initialize fields from field declarations
                // (Field initializers are compiled into the constructor)

                // Call constructor if it exists
                if let Some(_constructor) = class.find_constructor() {
                    // Constructor dispatch — VM constructors stored as VmClosures
                }

                // Pop unused arguments
                for _ in 0..argc {
                    self.pop();
                }

                Ok(())
            }
            _ => Err(RuntimeError::NotAClass(class_val.type_name(), span)),
        }
    }
}
