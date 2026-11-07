//! An array literal made only of literals, kept as one constant.
//!
//! `["draft", "sent"]` compiled to a `Constant` per element — each string a
//! round trip out of the fast tier — and an `Op::Array`, on every evaluation.
//! `["draft", "sent"].includes?(status)`, the membership test the style guide
//! recommends, then dispatched `includes?` by name on the array it had just
//! built, and dropped it: slower than the `==` chain it replaces. The
//! compiler now keeps such a literal's elements in one `Constant::LiteralList`
//! (`compiler_literal_arrays.rs`). `Op::LiteralArray` builds the array from it
//! in one dispatch, and `Op::LiteralListIncludes` answers the membership test
//! against the constant, with no array at all.
//!
//! Both answer what the code they replace answered: the array holds the values
//! the element literals evaluate to, and membership compares each element with
//! `==` (`Value`'s `PartialEq`), element on the left, as `Array#includes?` does
//! on both engines.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use crate::error::RuntimeError;
use crate::interpreter::value::{SoliStr, Value};

use super::chunk::Constant;
use super::opcode::NO_USER_METHOD;
use super::vm::Vm;

/// One element of a [`LiteralList`]: a literal, as the value it evaluates to.
///
/// Not a `Constant` (which cannot say "symbol") and not a `Value` (an `Rc` is
/// not `Send`, and compiled modules are shared between workers through
/// `compiled_cache`).
#[derive(Debug, Clone, PartialEq)]
pub enum ListLiteral {
    Int(i64),
    Float(f64),
    String(SoliStr),
    Symbol(SoliStr),
    Bool(bool),
    Null,
}

impl ListLiteral {
    /// The value the literal evaluates to, as `Op::Constant`, `Op::Symbol`,
    /// `Op::True`/`Op::False` or `Op::Null` pushes it. A string shares the
    /// constant's buffer, as a loaded string constant does.
    pub fn to_value(&self) -> Value {
        match self {
            ListLiteral::Int(n) => Value::Int(*n),
            ListLiteral::Float(f) => Value::Float(*f),
            ListLiteral::String(s) => Value::String(s.clone()),
            ListLiteral::Symbol(s) => Value::Symbol(s.clone()),
            ListLiteral::Bool(b) => Value::Bool(*b),
            ListLiteral::Null => Value::Null,
        }
    }
}

/// The elements of an all-literal array literal, in source order.
#[derive(Debug, Clone, PartialEq)]
pub struct LiteralList {
    pub items: Vec<ListLiteral>,
}

impl LiteralList {
    /// The elements' values, in order: what the array literal holds.
    pub fn values(&self) -> impl Iterator<Item = Value> + '_ {
        self.items.iter().map(ListLiteral::to_value)
    }

    /// A new array of the elements.
    pub fn to_array(&self) -> Value {
        Value::Array(Rc::new(RefCell::new(self.values().collect())))
    }

    /// `[…].includes?(needle)` for a needle that `==` judges by type and
    /// value alone — a string, a number, a symbol, a boolean or nil — without
    /// building a value per element. `None` for any other needle: a class
    /// equals the string of its name, a `grouped {}` placeholder compares as
    /// what it resolves to, and [`Self::contains`] compares those the long way.
    ///
    /// Each arm is `Value::eq` (`interpreter/value.rs`) with the element on
    /// the left, as `Array#includes?` has it: `Int` and `Float` compare by
    /// value across the two, and any other pair of different types is
    /// unequal. `literal_list_matches_value_equality` holds the two to it.
    #[inline]
    pub fn contains_scalar(&self, needle: &Value) -> Option<bool> {
        let items = &self.items;
        Some(match needle {
            Value::String(n) => items
                .iter()
                .any(|item| matches!(item, ListLiteral::String(s) if s == n)),
            Value::Int(n) => items.iter().any(|item| match item {
                ListLiteral::Int(a) => a == n,
                ListLiteral::Float(a) => *a == *n as f64,
                _ => false,
            }),
            Value::Float(n) => items.iter().any(|item| match item {
                ListLiteral::Float(a) => a == n,
                ListLiteral::Int(a) => *a as f64 == *n,
                _ => false,
            }),
            Value::Symbol(n) => items
                .iter()
                .any(|item| matches!(item, ListLiteral::Symbol(s) if s == n)),
            Value::Bool(b) => items
                .iter()
                .any(|item| matches!(item, ListLiteral::Bool(a) if a == b)),
            Value::Null => items.iter().any(|item| matches!(item, ListLiteral::Null)),
            _ => return None,
        })
    }

    /// `[…].includes?(needle)` for any needle: what `Array#includes?` answers
    /// on the array the literal builds, `element == needle` for each element.
    pub fn contains(&self, needle: &Value) -> bool {
        match self.contains_scalar(needle) {
            Some(found) => found,
            None => self.items.iter().any(|item| item.to_value() == *needle),
        }
    }
}

impl Vm {
    /// `Op::LiteralArray`: a fresh array of the list's elements. Fills an
    /// emptied array from `array_pool` when there is one, as `Op::Array` does.
    pub(crate) fn literal_array(&mut self, list: &LiteralList) -> Value {
        match self.array_pool.pop() {
            Some(array) => {
                array.borrow_mut().extend(list.values());
                Value::Array(array)
            }
            None => list.to_array(),
        }
    }

    /// `Op::LiteralArray` in the general tier, which the fast tier never
    /// hands it to (the constant is always a list).
    pub(crate) fn op_literal_array(&mut self, list_idx: u16) {
        let list: *const LiteralList = self.literal_list_constant(list_idx);
        // SAFETY: see `literal_list_constant`; nothing here pops the frame.
        let array = self.literal_array(unsafe { &*list });
        self.stack.push(array);
    }

    /// `Op::LiteralListIncludes` in the general tier: the needle is on top
    /// of the stack. The fast tier answers a scalar needle when no user
    /// method can take the call and hands everything else here.
    ///
    /// First what `CallMethod` does first, for a name it would have
    /// dispatched (`user_method` is not [`NO_USER_METHOD`]): a user-defined
    /// Array method of that name takes the call, on the array the literal
    /// builds. `CallMethodById`, which `include?` and `contains` compile to,
    /// has no such gate, and neither has the op for them. Then the
    /// comparison, `==` per element.
    pub(crate) fn op_literal_list_includes(
        &mut self,
        list_idx: u16,
        user_method: u16,
    ) -> Result<(), RuntimeError> {
        use crate::interpreter::executor::access::member::bind_user_method_to_receiver;
        use crate::interpreter::executor::calls::user_methods::{
            has_user_methods, lookup_user_method, PrimType,
        };
        let list: *const LiteralList = self.literal_list_constant(list_idx);
        // SAFETY: see `literal_list_constant`. `list` is last used before
        // `call_value`, the one thing here that can run other frames.
        let list = unsafe { &*list };
        if user_method != NO_USER_METHOD && has_user_methods(PrimType::Array) {
            let name = match &self
                .frames
                .last()
                .expect("a running frame")
                .closure
                .proto
                .chunk
                .constants[user_method as usize]
            {
                Constant::String(name) => name.clone(),
                _ => unreachable!("LiteralListIncludes names its method with a string constant"),
            };
            if let Some(method) = lookup_user_method(PrimType::Array, &name) {
                let span = self.current_span();
                let bound = bind_user_method_to_receiver(list.to_array(), method);
                let needle_idx = self.stack.len() - 1;
                self.stack.insert(needle_idx, bound);
                return self.call_value(1, span);
            }
        }
        let found = list.contains(self.stack.last().expect("the needle"));
        *self.stack.last_mut().expect("the needle") = Value::Bool(found);
        Ok(())
    }

    /// The `Constant::LiteralList` at `list_idx` in the running frame's pool,
    /// as a pointer: borrowing it would hold `self.frames` for the whole op.
    ///
    /// The pool belongs to the frame's prototype, which its closure keeps
    /// alive while the frame runs, and compiled chunks are never mutated; a
    /// caller must not use the pointer once something could pop the frame.
    /// The general arms of `HashWithKeys` and `CallMethod` read their
    /// constants the same way.
    fn literal_list_constant(&self, list_idx: u16) -> *const LiteralList {
        let constants = &self
            .frames
            .last()
            .expect("a running frame")
            .closure
            .proto
            .chunk
            .constants;
        match &constants[list_idx as usize] {
            Constant::LiteralList(list) => Arc::as_ptr(list),
            _ => unreachable!("a literal-list op reads a LiteralList constant"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::value::{Class, DecimalValue};

    fn list(items: Vec<ListLiteral>) -> LiteralList {
        LiteralList { items }
    }

    fn every_literal() -> Vec<ListLiteral> {
        vec![
            ListLiteral::Int(1),
            ListLiteral::Int(0),
            ListLiteral::Int(-3),
            ListLiteral::Int(i64::MAX),
            ListLiteral::Float(1.0),
            ListLiteral::Float(2.5),
            ListLiteral::Float(-0.0),
            ListLiteral::Float(f64::INFINITY),
            ListLiteral::String("up".into()),
            ListLiteral::String("".into()),
            ListLiteral::String("1".into()),
            ListLiteral::String("Account".into()),
            ListLiteral::String("a string longer than fifteen bytes".into()),
            ListLiteral::Symbol("up".into()),
            ListLiteral::Bool(true),
            ListLiteral::Bool(false),
            ListLiteral::Null,
        ]
    }

    /// Needles of every kind `includes?` can meet, scalar or not.
    fn needles() -> Vec<Value> {
        let class = |name: &str| {
            Value::Class(Rc::new(Class {
                name: name.to_string(),
                ..Default::default()
            }))
        };
        vec![
            Value::Int(1),
            Value::Int(0),
            Value::Int(-3),
            Value::Int(2),
            Value::Int(i64::MAX),
            Value::Float(1.0),
            Value::Float(0.0),
            Value::Float(2.5),
            Value::Float(-3.0),
            Value::Float(f64::NAN),
            Value::Float(f64::INFINITY),
            Value::Float(i64::MAX as f64),
            Value::String("up".into()),
            Value::String("".into()),
            Value::String("1".into()),
            Value::String("Up".into()),
            Value::String("a string longer than fifteen bytes".into()),
            Value::Symbol("up".into()),
            Value::Symbol("Account".into()),
            Value::Bool(true),
            Value::Bool(false),
            Value::Null,
            Value::Decimal(DecimalValue(rust_decimal::Decimal::ONE, 0)),
            Value::Array(Rc::new(RefCell::new(vec![Value::Int(1)]))),
            Value::Array(Rc::new(RefCell::new(Vec::new()))),
            class("Account"),
            class("User"),
            Value::DateTime(0, crate::interpreter::value::DtZone::UTC),
        ]
    }

    /// The literal answers each needle exactly as the array of its value
    /// does under `Value`'s `==`, the comparison `Array#includes?` makes.
    #[test]
    fn literal_list_matches_value_equality() {
        for literal in every_literal() {
            let single = list(vec![literal.clone()]);
            for needle in needles() {
                let expected = literal.to_value() == needle;
                assert_eq!(
                    single.contains(&needle),
                    expected,
                    "{literal:?} includes? {needle:?}"
                );
                if let Some(found) = single.contains_scalar(&needle) {
                    assert_eq!(found, expected, "scalar: {literal:?} includes? {needle:?}");
                }
            }
        }
    }

    /// Scalars are answered without the long way; everything else is not.
    #[test]
    fn only_scalar_needles_take_the_short_way() {
        let all = list(every_literal());
        for needle in needles() {
            let scalar = matches!(
                needle,
                Value::Int(_)
                    | Value::Float(_)
                    | Value::String(_)
                    | Value::Symbol(_)
                    | Value::Bool(_)
                    | Value::Null
            );
            assert_eq!(all.contains_scalar(&needle).is_some(), scalar, "{needle:?}");
        }
    }

    #[test]
    fn membership_over_a_whole_list() {
        let statuses = list(vec![
            ListLiteral::String("up".into()),
            ListLiteral::String("late".into()),
            ListLiteral::String("overdue".into()),
        ]);
        assert!(statuses.contains(&Value::String("overdue".into())));
        assert!(!statuses.contains(&Value::String("pending".into())));
        assert!(!statuses.contains(&Value::Symbol("up".into())));
        assert!(!statuses.contains(&Value::Null));
        assert!(!list(Vec::new()).contains(&Value::Null));
        let numbers = list(vec![ListLiteral::Int(1), ListLiteral::Float(2.5)]);
        assert!(numbers.contains(&Value::Float(1.0)));
        assert!(numbers.contains(&Value::Float(2.5)));
        assert!(!numbers.contains(&Value::Int(2)));
    }

    /// The built array holds what each literal evaluates to, and a new one
    /// each time.
    #[test]
    fn the_array_is_the_literals_values_and_fresh() {
        let mixed = list(every_literal());
        let first = mixed.to_array();
        let second = mixed.to_array();
        let (Value::Array(a), Value::Array(b)) = (&first, &second) else {
            panic!("an array");
        };
        assert!(!Rc::ptr_eq(a, b));
        let expected: Vec<Value> = every_literal().iter().map(ListLiteral::to_value).collect();
        assert_eq!(*a.borrow(), expected);
        a.borrow_mut().push(Value::Int(9));
        assert_eq!(b.borrow().len(), expected.len());
        assert!(matches!(
            ListLiteral::Symbol("up".into()).to_value(),
            Value::Symbol(s) if s.as_str() == "up"
        ));
        assert!(matches!(
            ListLiteral::String("up".into()).to_value(),
            Value::String(s) if s.as_str() == "up"
        ));
    }
}
