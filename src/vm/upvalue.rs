//! Upvalue and closure types for the Soli VM.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use crate::interpreter::value::Value;

use super::chunk::FunctionProto;

/// A VM closure: a function prototype paired with captured upvalues.
#[derive(Clone)]
pub struct VmClosure {
    pub proto: Rc<FunctionProto>,
    pub upvalues: Vec<Rc<RefCell<Upvalue>>>,
}

impl VmClosure {
    pub fn new(proto: Rc<FunctionProto>, upvalues: Vec<Rc<RefCell<Upvalue>>>) -> Self {
        Self { proto, upvalues }
    }
}

impl fmt::Debug for VmClosure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<vm fn {}>", self.proto.name)
    }
}

/// An upvalue captures a variable from an enclosing scope.
///
/// While the variable is still on the stack (the enclosing function hasn't returned),
/// the upvalue is "open" and points to a stack slot.
/// Once the enclosing function returns, the upvalue is "closed" — the value is
/// moved out of the stack and into the upvalue itself.
#[derive(Debug, Clone)]
pub enum Upvalue {
    /// Points to a live stack slot.
    Open(usize),
    /// Holds the captured value after the enclosing scope exits.
    Closed(Value),
}

/// Descriptor emitted by the compiler for each upvalue a closure captures.
/// Used at runtime when creating the closure to wire up the upvalue references.
#[derive(Debug, Clone, Copy)]
pub struct UpvalueDescriptor {
    /// If true, the upvalue captures a local from the immediately enclosing function.
    /// If false, it captures an upvalue from the enclosing function's upvalue list.
    pub is_local: bool,
    /// Index: either a stack slot (if is_local) or an upvalue index in the enclosing closure.
    pub index: u16,
}
