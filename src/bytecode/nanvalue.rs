//! NaN-boxed value representation for high-performance VM execution.
//!
//! This module provides a compact value representation using IEEE 754 NaN signaling
//! to pack different value types into a single 64-bit word. Simple types (integers,
//! booleans, null) are stored directly, while complex types use tagged pointers.
//!
//! # Value Encoding (64 bits)
//!
//! - **Float**: Direct IEEE 754 representation (canonical NaN avoided)
//! - **Int**: Sign bit + 52-bit signed integer, tag = 0
//! - **Bool**: Tag = 1, value in lowest bit
//! - **Null**: Tag = 2
//! - **String**: Canonical NaN | tag(3) | pointer to Rc<String>
//! - **Array**: Canonical NaN | tag(4) | pointer to Rc<RefCell<Vec<VMValue>>>
//! - **Hash**: Canonical NaN | tag(5) | pointer to Vec<(VMValue, VMValue)>
//! - **Object**: Canonical NaN | tag(6) | pointer to Instance
//! - **Function**: Canonical NaN | tag(7) | pointer to Closure

use std::cell::RefCell;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ptr::NonNull;
use std::rc::Rc;

use crate::bytecode::chunk::{Closure, VMClass, VMInstance, VMIterator};

const QNAN_MASK: u64 = 0x7FF8_0000_0000_0000u64;
const TAG_BITS: u64 = 0x7;
const TAG_SHIFT: usize = 48;
const CANONICAL_NAN: u64 = 0x7FF8_0000_0000_0000u64;

const TAG_INT: u64 = 0;
const TAG_BOOL: u64 = 1;
const TAG_NULL: u64 = 2;
const TAG_STRING: u64 = 3;
const TAG_ARRAY: u64 = 4;
const TAG_HASH: u64 = 5;
const TAG_OBJECT: u64 = 6;
const TAG_FUNCTION: u64 = 7;

#[repr(transparent)]
#[derive(Copy, Clone)]
pub struct NaNValue(u64);

impl NaNValue {
    #[inline]
    pub fn from_f64(val: f64) -> Self {
        let bits = val.to_bits();
        if bits & QNAN_MASK == QNAN_MASK {
            Self(bits | 1)
        } else {
            Self(bits)
        }
    }

    #[inline]
    pub fn from_int(val: i64) -> Self {
        Self(((val as u64) & 0x00FF_FFFF_FFFF_FFFFu64) | TAG_INT)
    }

    #[inline]
    pub fn from_bool(val: bool) -> Self {
        Self(TAG_BOOL | (val as u64))
    }

    #[inline]
    pub fn from_null() -> Self {
        Self(TAG_NULL)
    }

    #[inline]
    pub fn from_string(ptr: NonNull<Rc<String>>) -> Self {
        let addr = ptr.as_ptr() as u64 & 0x0000_FFFF_FFFF_FFFFu64;
        Self(CANONICAL_NAN | (addr << TAG_SHIFT) | TAG_STRING)
    }

    #[inline]
    pub fn from_array(ptr: NonNull<Rc<RefCell<Vec<super::chunk::VMValue>>>>) -> Self {
        let addr = ptr.as_ptr() as u64 & 0x0000_FFFF_FFFF_FFFFu64;
        Self(CANONICAL_NAN | (addr << TAG_SHIFT) | TAG_ARRAY)
    }

    #[inline]
    pub fn from_hash(
        ptr: NonNull<Rc<RefCell<Vec<(super::chunk::VMValue, super::chunk::VMValue)>>>>,
    ) -> Self {
        let addr = ptr.as_ptr() as u64 & 0x0000_FFFF_FFFF_FFFFu64;
        Self(CANONICAL_NAN | (addr << TAG_SHIFT) | TAG_HASH)
    }

    #[inline]
    pub fn from_object(ptr: NonNull<Rc<RefCell<VMInstance>>>) -> Self {
        let addr = ptr.as_ptr() as u64 & 0x0000_FFFF_FFFF_FFFFu64;
        Self(CANONICAL_NAN | (addr << TAG_SHIFT) | TAG_OBJECT)
    }

    #[inline]
    pub fn from_class(ptr: NonNull<Rc<RefCell<VMClass>>>) -> Self {
        let addr = ptr.as_ptr() as u64 & 0x0000_FFFF_FFFF_FFFFu64;
        Self(CANONICAL_NAN | (addr << TAG_SHIFT) | TAG_OBJECT)
    }

    #[inline]
    pub fn from_closure(ptr: NonNull<Rc<RefCell<Closure>>>) -> Self {
        let addr = ptr.as_ptr() as u64 & 0x0000_FFFF_FFFF_FFFFu64;
        Self(CANONICAL_NAN | (addr << TAG_SHIFT) | TAG_FUNCTION)
    }

    #[inline]
    pub fn from_native_function(id: u16) -> Self {
        Self(CANONICAL_NAN | ((id as u64) << TAG_SHIFT) | TAG_FUNCTION)
    }

    #[inline]
    pub fn from_bound_method(ptr: NonNull<BoundMethod>) -> Self {
        let addr = ptr.as_ptr() as u64 & 0x0000_FFFF_FFFF_FFFFu64;
        Self(CANONICAL_NAN | (addr << TAG_SHIFT) | TAG_OBJECT)
    }

    #[inline]
    pub fn from_iterator(ptr: NonNull<Rc<RefCell<VMIterator>>>) -> Self {
        let addr = ptr.as_ptr() as u64 & 0x0000_FFFF_FFFF_FFFFu64;
        Self(CANONICAL_NAN | (addr << TAG_SHIFT) | TAG_OBJECT)
    }

    #[inline]
    pub fn from_upvalue(value: NaNValue) -> Self {
        Self(CANONICAL_NAN | (value.0 << TAG_SHIFT) & 0x0000_FFFF_FFFF_FFFFu64 | TAG_OBJECT)
    }

    #[inline]
    pub fn to_f64(self) -> f64 {
        f64::from_bits(self.0 & !1u64)
    }

    #[inline]
    pub fn is_float(self) -> bool {
        self.0 & QNAN_MASK != QNAN_MASK
    }

    #[inline]
    pub fn is_int(self) -> bool {
        (self.0 & QNAN_MASK) == 0 && (self.0 & TAG_BITS) == TAG_INT
    }

    #[inline]
    pub fn is_bool(self) -> bool {
        (self.0 & QNAN_MASK) == 0 && (self.0 & TAG_BITS) == TAG_BOOL
    }

    #[inline]
    pub fn is_null(self) -> bool {
        (self.0 & QNAN_MASK) == 0 && (self.0 & TAG_BITS) == TAG_NULL
    }

    #[inline]
    pub fn is_string(self) -> bool {
        (self.0 & QNAN_MASK) == CANONICAL_NAN && (self.0 & TAG_BITS) == TAG_STRING
    }

    #[inline]
    pub fn is_array(self) -> bool {
        (self.0 & QNAN_MASK) == CANONICAL_NAN && (self.0 & TAG_BITS) == TAG_ARRAY
    }

    #[inline]
    pub fn is_hash(self) -> bool {
        (self.0 & QNAN_MASK) == CANONICAL_NAN && (self.0 & TAG_BITS) == TAG_HASH
    }

    #[inline]
    pub fn is_object(self) -> bool {
        (self.0 & QNAN_MASK) == CANONICAL_NAN && (self.0 & TAG_BITS) == TAG_OBJECT
    }

    #[inline]
    pub fn is_function(self) -> bool {
        (self.0 & QNAN_MASK) == CANONICAL_NAN && (self.0 & TAG_BITS) == TAG_FUNCTION
    }

    #[inline]
    pub fn as_int(self) -> Option<i64> {
        if self.is_int() {
            Some((self.0 >> TAG_SHIFT) as i64)
        } else {
            None
        }
    }

    #[inline]
    pub fn as_bool(self) -> Option<bool> {
        if self.is_bool() {
            Some((self.0 & 1) != 0)
        } else {
            None
        }
    }

    #[inline]
    pub fn as_string(self) -> Option<NonNull<Rc<String>>> {
        if self.is_string() {
            let addr = (self.0 >> TAG_SHIFT) & 0x0000_FFFF_FFFF_FFFFu64;
            NonNull::new(addr as *mut Rc<String>)
        } else {
            None
        }
    }

    #[inline]
    pub fn as_array(self) -> Option<NonNull<Rc<RefCell<Vec<super::chunk::VMValue>>>>> {
        if self.is_array() {
            let addr = (self.0 >> TAG_SHIFT) & 0x0000_FFFF_FFFF_FFFFu64;
            NonNull::new(addr as *mut Rc<RefCell<Vec<super::chunk::VMValue>>>)
        } else {
            None
        }
    }

    #[inline]
    pub fn as_hash(
        self,
    ) -> Option<NonNull<Rc<RefCell<Vec<(super::chunk::VMValue, super::chunk::VMValue)>>>>> {
        if self.is_hash() {
            let addr = (self.0 >> TAG_SHIFT) & 0x0000_FFFF_FFFF_FFFFu64;
            NonNull::new(
                addr as *mut Rc<RefCell<Vec<(super::chunk::VMValue, super::chunk::VMValue)>>>,
            )
        } else {
            None
        }
    }

    #[inline]
    pub fn as_object(self) -> Option<NonNull<Rc<RefCell<VMInstance>>>> {
        if self.is_object() {
            let addr = (self.0 >> TAG_SHIFT) & 0x0000_FFFF_FFFF_FFFFu64;
            NonNull::new(addr as *mut Rc<RefCell<VMInstance>>)
        } else {
            None
        }
    }

    #[inline]
    pub fn as_class(self) -> Option<NonNull<Rc<RefCell<VMClass>>>> {
        if self.is_object() {
            let addr = (self.0 >> TAG_SHIFT) & 0x0000_FFFF_FFFF_FFFFu64;
            NonNull::new(addr as *mut Rc<RefCell<VMClass>>)
        } else {
            None
        }
    }

    #[inline]
    pub fn as_closure(self) -> Option<NonNull<Rc<RefCell<Closure>>>> {
        if self.is_function() {
            let addr = (self.0 >> TAG_SHIFT) & 0x0000_FFFF_FFFF_FFFFu64;
            NonNull::new(addr as *mut Rc<RefCell<Closure>>)
        } else {
            None
        }
    }

    #[inline]
    pub fn as_native_function(self) -> Option<u16> {
        if self.is_function() {
            let addr = (self.0 >> TAG_SHIFT) & 0x0000_FFFF_FFFF_FFFFu64;
            if addr < 0x7FF8_0000_0000u64 {
                Some(addr as u16)
            } else {
                None
            }
        } else {
            None
        }
    }

    #[inline]
    pub fn as_iterator(self) -> Option<NonNull<Rc<RefCell<VMIterator>>>> {
        if self.is_object() {
            let addr = (self.0 >> TAG_SHIFT) & 0x0000_FFFF_FFFF_FFFFu64;
            NonNull::new(addr as *mut Rc<RefCell<VMIterator>>)
        } else {
            None
        }
    }

    #[inline]
    pub fn tag(self) -> u64 {
        self.0 & TAG_BITS
    }

    #[inline]
    pub fn bits(self) -> u64 {
        self.0
    }
}

impl Default for NaNValue {
    fn default() -> Self {
        Self::from_null()
    }
}

impl PartialEq for NaNValue {
    fn eq(&self, other: &Self) -> bool {
        if self.is_float() && other.is_float() {
            self.to_f64() == other.to_f64()
        } else if self.is_int() && other.is_int() {
            self.as_int() == other.as_int()
        } else if self.is_bool() && other.is_bool() {
            self.as_bool() == other.as_bool()
        } else if self.is_null() && other.is_null() {
            true
        } else if self.is_string() && other.is_string() {
            unsafe { **self.as_string().unwrap().as_ref() == **other.as_string().unwrap().as_ref() }
        } else {
            false
        }
    }
}

impl Eq for NaNValue {}

impl Hash for NaNValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        if self.is_float() {
            self.to_f64().to_bits().hash(state);
        } else if self.is_int() {
            self.as_int().unwrap().hash(state);
        } else if self.is_bool() {
            self.as_bool().unwrap().hash(state);
        } else if self.is_null() {
            0u8.hash(state);
        } else if self.is_string() {
            unsafe {
                (**self.as_string().unwrap().as_ref()).hash(state);
            }
        } else {
            self.0.hash(state);
        }
    }
}

impl fmt::Debug for NaNValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_float() {
            write!(f, "{:?}", self.to_f64())
        } else if let Some(n) = self.as_int() {
            write!(f, "{}", n)
        } else if self.is_bool() {
            write!(f, "{}", self.as_bool().unwrap())
        } else if self.is_null() {
            write!(f, "null")
        } else if self.is_string() {
            write!(f, "\"{}\"", unsafe {
                self.as_string().unwrap().as_ref().clone()
            })
        } else if self.is_array() {
            write!(f, "[...]")
        } else if self.is_hash() {
            write!(f, "{{...}}")
        } else {
            write!(f, "<value {:016x}>", self.0)
        }
    }
}

impl fmt::Display for NaNValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(n) = self.as_int() {
            write!(f, "{}", n)
        } else if self.is_float() {
            write!(f, "{}", self.to_f64())
        } else if self.is_bool() {
            write!(f, "{}", self.as_bool().unwrap())
        } else if self.is_null() {
            write!(f, "null")
        } else if let Some(s) = self.as_string() {
            write!(f, "{}", unsafe { s.as_ref().clone() })
        } else if self.is_array() {
            write!(f, "[...]")
        } else if self.is_hash() {
            write!(f, "{{...}}")
        } else {
            write!(f, "<value>")
        }
    }
}

impl NaNValue {
    pub fn type_name(&self) -> &'static str {
        if self.is_float() {
            "Float"
        } else if self.is_int() {
            "Int"
        } else if self.is_bool() {
            "Bool"
        } else if self.is_null() {
            "Null"
        } else if self.is_string() {
            "String"
        } else if self.is_array() {
            "Array"
        } else if self.is_hash() {
            "Hash"
        } else if self.is_object() {
            "Instance"
        } else if self.is_function() {
            "Function"
        } else {
            "Unknown"
        }
    }

    pub fn is_truthy(&self) -> bool {
        if self.is_int() {
            self.as_int().unwrap() != 0
        } else if self.is_float() {
            self.to_f64() != 0.0
        } else if self.is_bool() {
            self.as_bool().unwrap()
        } else {
            !self.is_null()
        }
    }
}

#[derive(Debug, Clone)]
pub struct BoundMethod {
    pub instance: Rc<RefCell<VMInstance>>,
    pub method: Rc<RefCell<Closure>>,
}

impl NaNValue {
    pub fn to_vm_value(&self) -> super::chunk::VMValue {
        if self.is_float() {
            super::chunk::VMValue::Float(self.to_f64())
        } else if let Some(n) = self.as_int() {
            super::chunk::VMValue::Int(n)
        } else if let Some(b) = self.as_bool() {
            super::chunk::VMValue::Bool(b)
        } else if self.is_null() {
            super::chunk::VMValue::Null
        } else if let Some(s) = self.as_string() {
            super::chunk::VMValue::String(unsafe { Rc::clone(s.as_ref()) })
        } else if let Some(arr) = self.as_array() {
            super::chunk::VMValue::Array(unsafe { Rc::clone(arr.as_ref()) })
        } else if let Some(hash) = self.as_hash() {
            super::chunk::VMValue::Hash(unsafe { Rc::clone(hash.as_ref()) })
        } else if let Some(inst) = self.as_object() {
            super::chunk::VMValue::Instance(unsafe { Rc::clone(inst.as_ref()) })
        } else if let Some(closure) = self.as_closure() {
            super::chunk::VMValue::Closure(unsafe { Rc::clone(closure.as_ref()) })
        } else if let Some(id) = self.as_native_function() {
            super::chunk::VMValue::NativeFunction(id)
        } else {
            super::chunk::VMValue::Null
        }
    }

    pub fn from_vm_value(value: &super::chunk::VMValue) -> Self {
        match value {
            super::chunk::VMValue::Int(n) => Self::from_int(*n),
            super::chunk::VMValue::Float(f) => Self::from_f64(*f),
            super::chunk::VMValue::String(s) => Self::from_string(NonNull::from(s)),
            super::chunk::VMValue::Bool(b) => Self::from_bool(*b),
            super::chunk::VMValue::Null => Self::from_null(),
            super::chunk::VMValue::Array(arr) => Self::from_array(NonNull::from(arr)),
            super::chunk::VMValue::Hash(hash) => Self::from_hash(NonNull::from(hash)),
            super::chunk::VMValue::Closure(c) => Self::from_closure(NonNull::from(c)),
            super::chunk::VMValue::NativeFunction(id) => Self::from_native_function(*id),
            super::chunk::VMValue::Class(c) => Self::from_class(NonNull::from(c)),
            super::chunk::VMValue::Instance(i) => Self::from_object(NonNull::from(i)),
            super::chunk::VMValue::BoundMethod(..) => Self::from_null(),
            super::chunk::VMValue::BoundNativeMethod(..) => Self::from_null(),
            super::chunk::VMValue::Iterator(iter) => Self::from_iterator(NonNull::from(iter)),
        }
    }
}
