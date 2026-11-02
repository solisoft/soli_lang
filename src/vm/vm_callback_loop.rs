//! Iterator methods whose callback runs inside the dispatch loop.
//!
//! `h.each(fn(k, v) …)`, `a.map(fn(x) …)` and the other closure-taking
//! iterators used to loop in Rust and re-enter the interpreter once per
//! element: push the callee and its arguments, open a frame, call `run()`,
//! return a `Result` through half a dozen Rust frames, drop it, repeat. That
//! round trip cost several hundred instructions per element, far more than a
//! typical callback body.
//!
//! Here the iteration lives in the VM instead. Starting the method leaves a
//! [`CallbackLoop`] on the iterator stack and opens a frame for the first
//! element's callback, flagged as driving the loop, then returns to the
//! dispatch loop. When that frame returns, `Op::Return` hands the result to
//! [`Vm::callback_returned`], which records it and opens the next element's
//! frame — or, after the last, pops the loop and pushes the method's result
//! where the call left its receiver. No Rust recursion per element.
//!
//! The methods: on a hash `each` / `each_key` / `each_value`, `map`,
//! `filter` / `select` / `keep_if`, `reject` / `delete_if`, `any?` / `all?`,
//! `transform_values` / `transform_keys`; on an array `each`,
//! `each_with_index`, `map`, `filter` / `select`, `reject`, `find`, `any?` /
//! `all?` / `none?` / `one?`, `count`, `reduce` / `fold` and `sort_by`; on an
//! integer `times`, `upto` and `downto`.
//!
//! The loop state sits on `iter_stack` for a reason: `Return` truncates that
//! stack to the returning frame's base and exception unwinding to the
//! handler's depth, so a loop abandoned by a `throw` disappears without any
//! code here. Only the common shape is taken: a compiled closure that takes
//! exactly the arguments the method passes, the first of two
//! (`each_with_index { |x| … }`), none at all (a block that ignores its
//! element, `xs.each { print("tick") }`), or one parameter per part of an
//! array element (`pairs.map { |a, b| … }`). Everything else keeps the native
//! driver in `vm_hash_methods.rs` / `vm_array_methods.rs` /
//! `vm_int_methods.rs`, whose semantics this mirrors element for element: a
//! native function or a bound method, a parameter the call would leave to
//! its default, an arity the call would refuse, a function compiled to
//! machine code, and `sort` with a comparator.

use std::cell::RefCell;
use std::rc::Rc;

use crate::error::RuntimeError;
use crate::interpreter::executor::MAX_CALL_DEPTH;
use crate::interpreter::value::{HashKey, HashPairs, Value};

use super::upvalue::VmClosure;
use super::vm::{clone_scalar_fast, positional_supplied_mask, CallFrame, IterState, Vm};

/// What the method does with each callback result.
#[derive(Clone, Copy)]
pub(crate) enum LoopKind {
    /// `each` on a hash or an array, `each_value` / `each_key`,
    /// `each_with_index`, and `Int#times` / `upto` / `downto`: the result is
    /// ignored and the method returns its receiver.
    Each,
    /// `map`: a `[key, value]` result becomes an entry of the new hash.
    HashMap,
    /// `filter` / `select` / `keep_if` (keep when truthy) and `reject` /
    /// `delete_if` (keep when falsy): a new hash of the kept entries.
    HashFilter { keep_when_truthy: bool },
    /// `any?`, `all?` and (arrays only) `none?`: the walk stops at the first
    /// result whose truthiness is `stop_on`, and the method answers
    /// `answer_on_stop`; without a stop it answers the opposite. `any?` stops
    /// on a truthy result with `true`, `all?` on a falsy one with `false`,
    /// `none?` on a truthy one with `false`.
    AnyAll { stop_on: bool, answer_on_stop: bool },
    /// `transform_values`: same keys, callback results as values.
    HashTransformValues,
    /// `transform_keys`: callback results as keys, same values.
    HashTransformKeys,
    /// Array `map`.
    ArrayMap,
    /// Array `filter` / `select` (keep when truthy) and `reject` (keep when
    /// falsy).
    ArrayFilter { keep_when_truthy: bool },
    /// `find`: the first element whose result is truthy, or nil. Stops there.
    ArrayFind,
    /// `one?`: whether exactly one result is truthy. Stops at a second one.
    ArrayOne,
    /// `count` with a block: how many results are truthy.
    ArrayCount,
    /// `reduce` / `fold`: each result becomes the accumulator, which the
    /// method returns.
    ArrayReduce,
    /// `sort_by` with a block: the results are the elements' sort keys.
    ArraySortBy,
}

/// What the callback receives for each element.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ArgMode {
    /// `(key, value)` — a hash callback declaring two or more parameters.
    KeyValue,
    /// `[key, value]` — a hash callback declaring fewer.
    Pair,
    /// The entry's key: `each_key`, `transform_keys`.
    Key,
    /// One value: an entry's value, an array element, an integer.
    Value,
    /// Nothing: a callback that declares no parameter. Its frame has no
    /// argument slot, as the tree-walker binds such a block nothing.
    Nothing,
    /// `(item, index)`: `each_with_index` with two parameters.
    ItemIndex,
    /// `(acc, item)`: `reduce` with two parameters.
    AccItem,
    /// `[acc, item]`: `reduce` with one parameter, the pair the tree-walker
    /// binds it.
    AccPair,
    /// One argument per part of an array element: a callback with `n >= 2`
    /// required parameters destructures it, as a Ruby block does
    /// (`pairs.map { |a, b| … }`). A missing part reads as nil. An element
    /// that is not an array fails as the native driver's call does.
    Parts(usize),
}

#[derive(Clone)]
enum Source {
    Hash(Rc<RefCell<HashPairs>>),
    Array(Rc<RefCell<Vec<Value>>>),
    /// `Int#times` / `upto` / `downto`: element `i` is `start + i * step`
    /// (see [`int_at`]), and the method returns `receiver`.
    Ints {
        start: i64,
        step: i64,
        receiver: i64,
    },
    /// A spare state between loops: holds nothing alive.
    Empty,
}

/// An iterator method in progress; lives on `iter_stack`.
#[derive(Clone)]
pub struct CallbackLoop {
    kind: LoopKind,
    args: ArgMode,
    source: Source,
    /// `None` only in a spare state between loops.
    closure: Option<Rc<VmClosure>>,
    /// Element whose callback is running.
    index: usize,
    /// Length when the method started: as in the native drivers, elements
    /// appended by a callback are not visited, and the walk stops early if
    /// the collection shrinks.
    len: usize,
    /// The entry passed to the running callback, for the kinds that keep it.
    current: Option<(HashKey, Value)>,
    out_hash: HashPairs,
    out_array: Vec<Value>,
    /// The `any?` / `all?` / `none?` answer so far, or for `one?` whether
    /// exactly one result has been truthy.
    answer: bool,
    /// The one value the method returns, for the kinds that return one:
    /// `reduce`'s accumulator (moved into the callback's argument slot while
    /// the callback runs, and replaced by its result) and `find`'s element.
    value: Value,
    /// `count`'s tally.
    count: i64,
    /// `sort_by`'s `(element, key)` pairs, sorted when the walk ends.
    keyed: Vec<(Value, Value)>,
    /// `transform_values` builds its result on a copy of the receiver and
    /// overwrites each value in place — no hashing, no key clone per element
    /// — while every visited key is still at its position in the receiver.
    /// A callback that adds or removes keys turns this off: the copy is cut
    /// back to the visited entries and the rest is inserted, as the native
    /// driver does.
    mirror: bool,
    /// The callback's template, when its body is one (see [`Kernel`]).
    kernel: Option<Kernel>,
    /// Some element went through a real callback frame, which may have
    /// changed the receiver; until then a template run can trust it.
    touched: bool,
}

impl std::fmt::Debug for CallbackLoop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallbackLoop")
            .field("index", &self.index)
            .field("len", &self.len)
            .finish_non_exhaustive()
    }
}

/// Overwrite a stack slot, dropping the old value inline when it is a
/// scalar (see `vm::discard`). A string replacing a string — the key slot of
/// a hash iteration — is assigned in place, so the old one is released by
/// `EcoString`'s own drop rather than `Value`'s out-of-line drop glue.
#[inline(always)]
fn replace_slot(stack: &mut [Value], slot: usize, value: Value) {
    let target = &mut stack[slot];
    match (target, value) {
        (Value::String(old), Value::String(new)) => *old = new,
        (target, value) => super::vm::discard(std::mem::replace(target, value)),
    }
}

/// Most emptied arrays `Vm::array_pool` keeps.
const ARRAY_POOL_MAX: usize = 16;

/// The arguments of one callback call.
enum CallbackArgs {
    /// A callback with no parameter ([`ArgMode::Nothing`]).
    Zero,
    One(Value),
    Two(Value, Value),
    /// The first `count` parts of an array element ([`ArgMode::Parts`]).
    Parts(Rc<RefCell<Vec<Value>>>, usize),
}

impl CallbackArgs {
    /// Push the arguments; returns how many.
    #[inline]
    fn push_onto(self, stack: &mut Vec<Value>) -> usize {
        match self {
            CallbackArgs::Zero => 0,
            CallbackArgs::One(a) => {
                stack.push(a);
                1
            }
            CallbackArgs::Two(a, b) => {
                stack.push(a);
                stack.push(b);
                2
            }
            CallbackArgs::Parts(parts, count) => {
                let parts = parts.borrow();
                for offset in 0..count {
                    stack.push(parts.get(offset).cloned().unwrap_or(Value::Null));
                }
                count
            }
        }
    }
}

/// An [`ArgMode::Parts`] callback met an element that is not an array.
struct NotAnArray;

/// How [`refill`] left the loop.
enum Refill {
    /// The next element's arguments are in its slots: run the frame again.
    Ready,
    /// No element is left.
    Done,
    /// See [`NotAnArray`].
    NotAnArray,
}

/// Element `index` of a [`Source::Ints`] walk. Exact for every index the
/// walk reaches, since the true value lies between the receiver and the
/// limit and so fits an `i64`.
#[inline(always)]
fn int_at(start: i64, step: i64, index: usize) -> i64 {
    start.wrapping_add((index as i64).wrapping_mul(step))
}

/// How many integers `low..=high` holds, saturated to `usize`.
fn inclusive_count(low: i64, high: i64) -> usize {
    if high < low {
        return 0;
    }
    usize::try_from(i128::from(high) - i128::from(low) + 1).unwrap_or(usize::MAX)
}

/// `HashKey::to_value` with the string case inline (it is an out-of-line
/// call otherwise), for the key handed to each callback.
#[inline(always)]
fn key_value_fast(key: &HashKey) -> Value {
    match key {
        HashKey::String(s) => Value::String(s.clone()),
        other => other.to_value(),
    }
}

/// `HashKey::clone` with the string case inline.
#[inline(always)]
fn clone_key_fast(key: &HashKey) -> HashKey {
    match key {
        HashKey::String(s) => HashKey::String(s.clone()),
        other => other.clone(),
    }
}

/// `Value::is_truthy` with a boolean — what a predicate callback returns —
/// answered inline.
#[inline(always)]
fn truthy_fast(value: &Value) -> bool {
    match value {
        Value::Bool(b) => *b,
        other => other.is_truthy(),
    }
}

/// Fold one callback result into the loop; the message of the error that
/// fails the method (a `map` pair or transformed key that cannot be a hash
/// key, as in the native driver), if any. Scalar results are dropped inline.
#[inline(always)]
fn record(state: &mut CallbackLoop, result: Value) -> Option<&'static str> {
    // Only the kinds that keep the running entry take it: dropping the
    // (usually empty) `Option` is an out-of-line call on every element.
    match state.kind {
        LoopKind::Each => {
            super::vm::discard(result);
            None
        }
        LoopKind::HashMap => match &result {
            Value::Array(pair) if pair.borrow().len() == 2 => {
                let pair = pair.borrow();
                match pair[0].to_hash_key() {
                    Some(key) => {
                        state.out_hash.insert(key, pair[1].clone());
                        None
                    }
                    None => Some("hash key must be hashable"),
                }
            }
            _ => None,
        },
        LoopKind::HashFilter { keep_when_truthy } => {
            if truthy_fast(&result) == keep_when_truthy {
                let (key, value) = state.current.take().expect("filter keeps its entry");
                state.out_hash.insert(key, value);
            }
            super::vm::discard(result);
            None
        }
        LoopKind::AnyAll {
            stop_on,
            answer_on_stop,
        } => {
            if truthy_fast(&result) == stop_on {
                state.answer = answer_on_stop;
                state.index = state.len; // decided: stop here
            }
            super::vm::discard(result);
            None
        }
        LoopKind::HashTransformValues => {
            if state.mirror {
                // `index` has already moved past the element whose callback ran.
                if let Some((_, slot)) = state.out_hash.get_index_mut(state.index - 1) {
                    *slot = result;
                }
            } else {
                let (key, _) = state
                    .current
                    .take()
                    .expect("transform_values keeps its key");
                state.out_hash.insert(key, result);
            }
            None
        }
        LoopKind::HashTransformKeys => match result.to_hash_key() {
            Some(key) => {
                let (_, value) = state
                    .current
                    .take()
                    .expect("transform_keys keeps its value");
                state.out_hash.insert(key, value);
                None
            }
            None => Some("transformed key must be hashable"),
        },
        LoopKind::ArrayMap => {
            state.out_array.push(result);
            None
        }
        LoopKind::ArrayFilter { keep_when_truthy } => {
            if truthy_fast(&result) == keep_when_truthy {
                let (_, item) = state.current.take().expect("filter keeps its item");
                state.out_array.push(item);
            }
            super::vm::discard(result);
            None
        }
        LoopKind::ArrayFind => {
            if truthy_fast(&result) {
                let (_, item) = state.current.take().expect("find keeps its item");
                state.value = item;
                state.index = state.len; // found: stop here
            }
            super::vm::discard(result);
            None
        }
        LoopKind::ArrayOne => {
            if truthy_fast(&result) {
                if state.answer {
                    // A second one: the answer is no, and final.
                    state.answer = false;
                    state.index = state.len;
                } else {
                    state.answer = true;
                }
            }
            super::vm::discard(result);
            None
        }
        LoopKind::ArrayCount => {
            if truthy_fast(&result) {
                state.count += 1;
            }
            super::vm::discard(result);
            None
        }
        LoopKind::ArrayReduce => {
            super::vm::discard(std::mem::replace(&mut state.value, result));
            None
        }
        LoopKind::ArraySortBy => {
            let (_, item) = state.current.take().expect("sort_by keeps its item");
            state.keyed.push((item, result));
            None
        }
    }
}

/// Whether the running callback's entry must be kept for its result:
/// `filter` / `reject` keep it if the callback says so, `find` keeps the
/// element it finds, `sort_by` pairs every element with its key, and
/// `transform_keys` keeps the value. `transform_values` keeps it only once it
/// has left the mirrored mode (see `CallbackLoop::mirror`), decided per
/// element.
#[inline(always)]
fn keeps_entry(state: &CallbackLoop) -> bool {
    matches!(
        state.kind,
        LoopKind::HashFilter { .. }
            | LoopKind::HashTransformKeys
            | LoopKind::ArrayFilter { .. }
            | LoopKind::ArrayFind
            | LoopKind::ArraySortBy
    )
}

/// Write the next element's arguments straight into the kept frame's
/// slots from `first_slot`, advancing the loop. The same element reading as
/// `next_callback_args`, without building the arguments first and moving
/// them into place. A callback with no parameter ([`ArgMode::Nothing`]) has
/// no slot to write: the element is still read, kept and counted.
#[inline(always)]
fn refill(state: &mut CallbackLoop, stack: &mut [Value], first_slot: usize) -> Refill {
    if state.index >= state.len {
        return Refill::Done;
    }
    state.touched = true;
    let keeps_entry = keeps_entry(state);
    match &state.source {
        Source::Array(array) => {
            let array = array.borrow();
            let Some(item) = array.get(state.index) else {
                return Refill::Done;
            };
            if keeps_entry {
                state.current = Some((HashKey::Null, item.clone()));
            }
            match state.args {
                ArgMode::Value => replace_slot(stack, first_slot, clone_scalar_fast(item)),
                ArgMode::Nothing => {}
                ArgMode::ItemIndex => {
                    replace_slot(stack, first_slot, clone_scalar_fast(item));
                    replace_slot(stack, first_slot + 1, Value::Int(state.index as i64));
                }
                ArgMode::AccItem => {
                    let acc = std::mem::replace(&mut state.value, Value::Null);
                    replace_slot(stack, first_slot, acc);
                    replace_slot(stack, first_slot + 1, clone_scalar_fast(item));
                }
                ArgMode::AccPair => {
                    let acc = std::mem::replace(&mut state.value, Value::Null);
                    let pair = vec![acc, clone_scalar_fast(item)];
                    replace_slot(stack, first_slot, Value::Array(Rc::new(RefCell::new(pair))));
                }
                ArgMode::Parts(count) => {
                    let Value::Array(parts) = item else {
                        return Refill::NotAnArray;
                    };
                    let parts = parts.borrow();
                    for offset in 0..count {
                        let part = parts.get(offset).map_or(Value::Null, clone_scalar_fast);
                        replace_slot(stack, first_slot + offset, part);
                    }
                }
                ArgMode::KeyValue | ArgMode::Pair | ArgMode::Key => {
                    unreachable!("a hash argument mode on an array loop")
                }
            }
        }
        Source::Ints { start, step, .. } => {
            if state.args != ArgMode::Nothing {
                let n = int_at(*start, *step, state.index);
                replace_slot(stack, first_slot, Value::Int(n));
            }
        }
        Source::Empty => unreachable!("a running loop has a source"),
        Source::Hash(hash) => {
            let hash = hash.borrow();
            let Some((key, value)) = hash.get_index(state.index) else {
                if state.mirror {
                    // The receiver shrank: only the visited entries count.
                    state.out_hash.truncate(state.index);
                }
                return Refill::Done;
            };
            if state.mirror && state.out_hash.get_index(state.index).map(|(k, _)| k) != Some(key) {
                // A callback added or removed keys: keep what was visited and
                // insert from here on.
                state.out_hash.truncate(state.index);
                state.mirror = false;
            }
            if keeps_entry || (matches!(state.kind, LoopKind::HashTransformValues) && !state.mirror)
            {
                state.current = Some((clone_key_fast(key), clone_scalar_fast(value)));
            }
            match state.args {
                ArgMode::KeyValue => {
                    replace_slot(stack, first_slot, key_value_fast(key));
                    replace_slot(stack, first_slot + 1, clone_scalar_fast(value));
                }
                ArgMode::Pair => {
                    // The previous element's `[key, value]` pair, if the
                    // callback kept no reference to it, is refilled in place:
                    // nothing else can see it, and the element saves two
                    // allocations and two frees.
                    let reused = match &stack[first_slot] {
                        Value::Array(pair)
                            if Rc::strong_count(pair) == 1 && Rc::weak_count(pair) == 0 =>
                        {
                            let mut cells = pair.borrow_mut();
                            if cells.len() == 2 {
                                cells[0] = key_value_fast(key);
                                cells[1] = clone_scalar_fast(value);
                                true
                            } else {
                                false
                            }
                        }
                        _ => false,
                    };
                    if !reused {
                        replace_slot(
                            stack,
                            first_slot,
                            Value::Array(Rc::new(RefCell::new(vec![
                                key_value_fast(key),
                                value.clone(),
                            ]))),
                        );
                    }
                }
                ArgMode::Key => replace_slot(stack, first_slot, key_value_fast(key)),
                ArgMode::Value => replace_slot(stack, first_slot, clone_scalar_fast(value)),
                ArgMode::Nothing => {}
                ArgMode::ItemIndex | ArgMode::AccItem | ArgMode::AccPair | ArgMode::Parts(_) => {
                    unreachable!("an array argument mode on a hash loop")
                }
            }
        }
    }
    state.index += 1;
    Refill::Ready
}

// ---------------------------------------------------------------------------
// Templates: pure callback bodies evaluated without a frame.
// ---------------------------------------------------------------------------

/// A constant operand of a template.
#[derive(Clone, Copy)]
enum KConst {
    Int(i64),
    Float(f64),
}

#[derive(Clone, Copy)]
enum ArithOp {
    Add,
    Sub,
    Mul,
}

#[derive(Clone, Copy)]
enum CmpOp {
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
}

/// A callback body the loop evaluates itself, element after element, with no
/// frame and no dispatch: the whole body is one of these shapes followed by
/// `Return`, reading only parameters (slots `1..=params`). Such a body has no
/// side effects, so skipping the frame is unobservable — and every template
/// computes inline only the cases its opcode's general arm answers inline
/// (int with overflow checked, float with float); anything else hands that
/// element to the frame path, which runs the real opcodes and so produces the
/// identical result or error.
#[derive(Clone, Copy)]
enum Kernel {
    /// `GET_LOCAL s` — `fn(x) x`
    Param(u16),
    /// `ADD_LC` / `SUB_LC` / `MUL_LC s, c` — `fn(x) x * 2`
    Arith(ArithOp, u16, KConst),
    /// `<CMP>_LC s, c` — `fn(x) x > 10`
    Cmp(CmpOp, u16, KConst),
    /// `GET_LOCAL a; <ARITH>_LC b, c; ARRAY 2` — `fn(k, v) [k, v + 1]`
    PairParamArith(u16, ArithOp, u16, KConst),
    /// `GET_LOCAL a; GET_LOCAL b; ARRAY 2` — `fn(k, v) [v, k]`
    PairParams(u16, u16),
    /// `GET_UPVALUE u; CONSTANT c | GET_LOCAL p; ADD_ASSIGN_U u` —
    /// `fn(k, v) { total = total + v }`, `{ count += 1 }`: the accumulator a
    /// loop body most often is. Its one effect, the write to the captured
    /// variable, happens only once the sum is known to be exactly what
    /// `ADD_ASSIGN_U` would produce.
    Accumulate(u16, Operand),
}

/// The second operand of an `Accumulate` template.
#[derive(Clone, Copy)]
enum Operand {
    Const(KConst),
    Param(u16),
}

/// What a template produced. `Pair` is `[a, b]` never materialised: `map`
/// inserts it directly, and anything else builds the array.
enum KernelOut {
    Value(Value),
    Pair(Value, Value),
}

impl KernelOut {
    #[inline(always)]
    fn into_value(self) -> Value {
        match self {
            KernelOut::Value(value) => value,
            KernelOut::Pair(a, b) => Value::Array(Rc::new(RefCell::new(vec![a, b]))),
        }
    }
}

/// A parameter as the template reads it: the element's key or a value.
#[derive(Clone, Copy)]
enum ArgRef<'a> {
    Key(&'a HashKey),
    Val(&'a Value),
}

impl ArgRef<'_> {
    #[inline(always)]
    fn to_value(self) -> Value {
        match self {
            ArgRef::Key(key) => key_value_fast(key),
            ArgRef::Val(value) => clone_scalar_fast(value),
        }
    }

    /// The operand as the fast arms see it: an int or a float, or `None`.
    #[inline(always)]
    fn number(self) -> Option<KConst> {
        match self {
            ArgRef::Val(Value::Int(n)) | ArgRef::Key(HashKey::Int(n)) => Some(KConst::Int(*n)),
            ArgRef::Val(Value::Float(f)) => Some(KConst::Float(*f)),
            _ => None,
        }
    }
}

#[inline(always)]
fn arith(op: ArithOp, a: KConst, b: KConst) -> Option<Value> {
    match (a, b) {
        (KConst::Int(x), KConst::Int(y)) => match op {
            ArithOp::Add => x.checked_add(y),
            ArithOp::Sub => x.checked_sub(y),
            ArithOp::Mul => x.checked_mul(y),
        }
        .map(Value::Int),
        (KConst::Float(x), KConst::Float(y)) => Some(Value::Float(match op {
            ArithOp::Add => x + y,
            ArithOp::Sub => x - y,
            ArithOp::Mul => x * y,
        })),
        _ => None,
    }
}

#[inline(always)]
fn compare(op: CmpOp, a: KConst, b: KConst) -> Option<bool> {
    let ordering = match (a, b) {
        (KConst::Int(x), KConst::Int(y)) => x.cmp(&y),
        (KConst::Float(x), KConst::Float(y)) => x.partial_cmp(&y)?,
        _ => return None,
    };
    Some(match op {
        CmpOp::Less => ordering.is_lt(),
        CmpOp::LessEqual => ordering.is_le(),
        CmpOp::Greater => ordering.is_gt(),
        CmpOp::GreaterEqual => ordering.is_ge(),
    })
}

impl Kernel {
    /// Recognise a template in a closure body taking `params` parameters.
    fn of(proto: &super::chunk::FunctionProto, params: usize) -> Option<Kernel> {
        use super::chunk::Constant;
        use super::opcode::Op;
        let param = |slot: u16| (1..=params).contains(&(slot as usize)).then_some(slot);
        let constant = |idx: u16| match proto.chunk.constants.get(idx as usize)? {
            Constant::Int(n) => Some(KConst::Int(*n)),
            Constant::Float(f) => Some(KConst::Float(*f)),
            _ => None,
        };
        let arith_of = |op: Op| match op {
            Op::AddLocalConst(s, c) => Some((ArithOp::Add, s, c)),
            Op::SubLocalConst(s, c) => Some((ArithOp::Sub, s, c)),
            Op::MulLocalConst(s, c) => Some((ArithOp::Mul, s, c)),
            _ => None,
        };
        let code = &proto.chunk.code;
        match code.as_slice() {
            [Op::GetLocal(s), Op::Return, ..] => Some(Kernel::Param(param(*s)?)),
            [op, Op::Return, ..] if arith_of(*op).is_some() => {
                let (a, s, c) = arith_of(*op)?;
                Some(Kernel::Arith(a, param(s)?, constant(c)?))
            }
            [Op::LessLocalConst(s, c), Op::Return, ..] => {
                Some(Kernel::Cmp(CmpOp::Less, param(*s)?, constant(*c)?))
            }
            [Op::LessEqualLocalConst(s, c), Op::Return, ..] => {
                Some(Kernel::Cmp(CmpOp::LessEqual, param(*s)?, constant(*c)?))
            }
            [Op::GreaterLocalConst(s, c), Op::Return, ..] => {
                Some(Kernel::Cmp(CmpOp::Greater, param(*s)?, constant(*c)?))
            }
            [Op::GreaterEqualLocalConst(s, c), Op::Return, ..] => {
                Some(Kernel::Cmp(CmpOp::GreaterEqual, param(*s)?, constant(*c)?))
            }
            [Op::GetLocal(a), op, Op::Array(2), Op::Return, ..] if arith_of(*op).is_some() => {
                let (ar, s, c) = arith_of(*op)?;
                Some(Kernel::PairParamArith(
                    param(*a)?,
                    ar,
                    param(s)?,
                    constant(c)?,
                ))
            }
            [Op::GetLocal(a), Op::GetLocal(b), Op::Array(2), Op::Return, ..] => {
                Some(Kernel::PairParams(param(*a)?, param(*b)?))
            }
            [Op::GetUpvalue(u), Op::Constant(c), Op::AddAssignUpvalue(w), Op::Return, ..]
                if u == w =>
            {
                Some(Kernel::Accumulate(*u, Operand::Const(constant(*c)?)))
            }
            [Op::GetUpvalue(u), Op::GetLocal(p), Op::AddAssignUpvalue(w), Op::Return, ..]
                if u == w =>
            {
                Some(Kernel::Accumulate(*u, Operand::Param(param(*p)?)))
            }
            _ => None,
        }
    }

    /// `eval` for the templates that produce one value — all but the pairs —
    /// without the `KernelOut` wrapper on the per-element path.
    #[inline(always)]
    fn eval_value<'a>(self, arg: impl Fn(u16) -> ArgRef<'a> + Copy) -> Option<Value> {
        match self {
            Kernel::Param(s) => Some(arg(s).to_value()),
            Kernel::Arith(op, s, c) => arith(op, arg(s).number()?, c),
            Kernel::Cmp(op, s, c) => Some(Value::Bool(compare(op, arg(s).number()?, c)?)),
            Kernel::PairParamArith(..) | Kernel::PairParams(..) => {
                Some(self.eval(arg)?.into_value())
            }
            Kernel::Accumulate(..) => None,
        }
    }

    /// Evaluate for one element; `None` hands the element to the frame path.
    #[inline(always)]
    fn eval<'a>(self, arg: impl Fn(u16) -> ArgRef<'a> + Copy) -> Option<KernelOut> {
        Some(match self {
            Kernel::Param(s) => KernelOut::Value(arg(s).to_value()),
            Kernel::Arith(op, s, c) => KernelOut::Value(arith(op, arg(s).number()?, c)?),
            Kernel::Cmp(op, s, c) => {
                KernelOut::Value(Value::Bool(compare(op, arg(s).number()?, c)?))
            }
            Kernel::PairParamArith(a, op, s, c) => {
                let second = arith(op, arg(s).number()?, c)?;
                KernelOut::Pair(arg(a).to_value(), second)
            }
            Kernel::PairParams(a, b) => KernelOut::Pair(arg(a).to_value(), arg(b).to_value()),
            // Needs the closure and the stack: see `accumulate`.
            Kernel::Accumulate(..) => return None,
        })
    }
}

/// Run an `Accumulate` template: `upvalue = upvalue + operand`, returning
/// the new value as `ADD_ASSIGN_U` leaves it. `None`, having written
/// nothing, unless both operands are ints (overflow checked) or both floats;
/// the frame path runs the opcode for the rest (a string, a mixed pair, an
/// overflow).
#[inline(always)]
fn accumulate<'a>(
    closure: &VmClosure,
    stack: &mut [Value],
    upvalue: u16,
    operand: Operand,
    arg: impl Fn(u16) -> ArgRef<'a>,
) -> Option<Value> {
    let right = match operand {
        Operand::Const(c) => c,
        Operand::Param(slot) => arg(slot).number()?,
    };
    let cell = closure.upvalues.get(upvalue as usize)?;
    let mut cell = cell.borrow_mut();
    let target = match &mut *cell {
        super::upvalue::Upvalue::Open(slot) => stack.get_mut(*slot)?,
        super::upvalue::Upvalue::Closed(value) => value,
    };
    let left = match target {
        Value::Int(n) => KConst::Int(*n),
        Value::Float(f) => KConst::Float(*f),
        _ => return None,
    };
    let sum = arith(ArithOp::Add, left, right)?;
    *target = clone_scalar_fast(&sum);
    Some(sum)
}

/// Where a template run stopped.
enum KernelRun {
    /// Every element is done.
    Done,
    /// The element at `index` needs the frame path (not consumed).
    Fallback,
}

/// Run the loop's template over its elements from `index`, recording each
/// result exactly as `record` would. Stops at the first element the template
/// hands off. `Err` carries the message of the error that fails the method
/// (an unhashable `map` / `transform_keys` key), as `record` reports it.
fn run_kernel(
    state: &mut CallbackLoop,
    kernel: Kernel,
    stack: &mut [Value],
) -> Result<KernelRun, &'static str> {
    let CallbackLoop {
        kind,
        args,
        source,
        closure,
        index,
        len,
        out_hash,
        out_array,
        answer,
        value: found,
        count,
        keyed,
        mirror,
        touched,
        ..
    } = state;
    let closure = closure.as_ref().expect("a running loop has its callback");
    // Untouched, the receiver is exactly what the mirror copied: no key moved.
    let check_mirror = *touched;
    // Evaluate the template for one element's parameters.
    macro_rules! evaluate {
        ($arg:expr) => {
            match kernel {
                Kernel::Accumulate(upvalue, operand) => {
                    accumulate(closure, stack, upvalue, operand, $arg).map(KernelOut::Value)
                }
                _ => kernel.eval($arg),
            }
        };
    }
    macro_rules! evaluate_value {
        ($arg:expr) => {
            match kernel {
                Kernel::Accumulate(upvalue, operand) => {
                    accumulate(closure, stack, upvalue, operand, $arg)
                }
                _ => kernel.eval_value($arg),
            }
        };
    }
    match source {
        Source::Empty => unreachable!("a running loop has a source"),
        Source::Array(array) => {
            // No side effect can run while the template does: hold the borrow.
            let array = array.borrow();
            while *index < *len {
                let Some(item) = array.get(*index) else {
                    return Ok(KernelRun::Done);
                };
                let Some(out) = evaluate_value!(|_| ArgRef::Val(item)) else {
                    return Ok(KernelRun::Fallback);
                };
                *index += 1;
                // As `record` folds a frame's result, with the element read
                // in place: nothing can have changed it.
                match *kind {
                    LoopKind::Each => super::vm::discard(out),
                    LoopKind::ArrayMap => out_array.push(out),
                    LoopKind::ArrayFilter { keep_when_truthy } => {
                        if truthy_fast(&out) == keep_when_truthy {
                            out_array.push(clone_scalar_fast(item));
                        }
                        super::vm::discard(out);
                    }
                    LoopKind::ArrayFind => {
                        if truthy_fast(&out) {
                            *found = clone_scalar_fast(item);
                            *index = *len;
                        }
                        super::vm::discard(out);
                    }
                    LoopKind::AnyAll {
                        stop_on,
                        answer_on_stop,
                    } => {
                        if truthy_fast(&out) == stop_on {
                            *answer = answer_on_stop;
                            *index = *len;
                        }
                        super::vm::discard(out);
                    }
                    LoopKind::ArrayOne => {
                        if truthy_fast(&out) {
                            if *answer {
                                *answer = false;
                                *index = *len;
                            } else {
                                *answer = true;
                            }
                        }
                        super::vm::discard(out);
                    }
                    LoopKind::ArrayCount => {
                        if truthy_fast(&out) {
                            *count += 1;
                        }
                        super::vm::discard(out);
                    }
                    LoopKind::ArraySortBy => keyed.push((clone_scalar_fast(item), out)),
                    _ => unreachable!("an array loop with a template"),
                }
            }
            Ok(KernelRun::Done)
        }
        Source::Ints { start, step, .. } => {
            let (start, step) = (*start, *step);
            while *index < *len {
                let n = Value::Int(int_at(start, step, *index));
                let Some(out) = evaluate_value!(|_| ArgRef::Val(&n)) else {
                    return Ok(KernelRun::Fallback);
                };
                *index += 1;
                match *kind {
                    LoopKind::Each => super::vm::discard(out),
                    _ => unreachable!("an integer loop is an `each`"),
                }
            }
            Ok(KernelRun::Done)
        }
        Source::Hash(hash) => {
            let hash = hash.borrow();
            while *index < *len {
                let Some((key, value)) = hash.get_index(*index) else {
                    if *mirror {
                        out_hash.truncate(*index);
                    }
                    return Ok(KernelRun::Done);
                };
                if *mirror
                    && check_mirror
                    && out_hash.get_index(*index).map(|(k, _)| k) != Some(key)
                {
                    out_hash.truncate(*index);
                    *mirror = false;
                }
                let arg = |slot: u16| match (*args, slot) {
                    (ArgMode::KeyValue, 1) | (ArgMode::Key, _) => ArgRef::Key(key),
                    _ => ArgRef::Val(value),
                };
                if let LoopKind::HashMap = *kind {
                    let Some(out) = evaluate!(arg) else {
                        return Ok(KernelRun::Fallback);
                    };
                    *index += 1;
                    match out {
                        KernelOut::Pair(k, v) => match HashKey::from_value_owned(k) {
                            Some(k) => {
                                out_hash.insert(k, v);
                            }
                            None => return Err("hash key must be hashable"),
                        },
                        KernelOut::Value(Value::Array(pair)) => {
                            let pair = pair.borrow();
                            if pair.len() == 2 {
                                match pair[0].to_hash_key() {
                                    Some(k) => {
                                        out_hash.insert(k, pair[1].clone());
                                    }
                                    None => return Err("hash key must be hashable"),
                                }
                            }
                        }
                        KernelOut::Value(_) => {}
                    }
                    continue;
                }
                let Some(out) = evaluate_value!(arg) else {
                    return Ok(KernelRun::Fallback);
                };
                *index += 1;
                match *kind {
                    LoopKind::Each => super::vm::discard(out),
                    LoopKind::HashFilter { keep_when_truthy } => {
                        if truthy_fast(&out) == keep_when_truthy {
                            out_hash.insert(clone_key_fast(key), clone_scalar_fast(value));
                        }
                        super::vm::discard(out);
                    }
                    LoopKind::AnyAll {
                        stop_on,
                        answer_on_stop,
                    } => {
                        if truthy_fast(&out) == stop_on {
                            *answer = answer_on_stop;
                            *index = *len;
                        }
                        super::vm::discard(out);
                    }
                    LoopKind::HashTransformValues => {
                        if *mirror {
                            if let Some((_, slot)) = out_hash.get_index_mut(*index - 1) {
                                super::vm::discard(std::mem::replace(slot, out));
                            }
                        } else {
                            out_hash.insert(clone_key_fast(key), out);
                        }
                    }
                    LoopKind::HashTransformKeys => match out.to_hash_key() {
                        Some(k) => {
                            out_hash.insert(k, clone_scalar_fast(value));
                        }
                        None => return Err("transformed key must be hashable"),
                    },
                    _ => unreachable!("a hash loop"),
                }
            }
            Ok(KernelRun::Done)
        }
    }
}

/// The loop that runs `receiver.name(…)`, from the receiver and its
/// arguments (`operands`) and the callback's declared (`params`) and
/// required (`arity`) parameter counts; `None` keeps the native driver.
///
/// The argument rules are the native drivers': a block takes as many of the
/// arguments as it declares (`vm_calls::block_argc`), and a block of two or
/// more required parameters destructures an array element
/// (`Vm::invoke_in_batch_one`). Whatever the native call would fill with a
/// default, or refuse with an arity error, stays with the native driver,
/// which owns those rules and their messages.
fn loop_shape(
    operands: &[Value],
    name: &str,
    params: usize,
    arity: usize,
) -> Option<(LoopKind, ArgMode)> {
    let argc = operands.len() - 1;
    match &operands[0] {
        Value::Hash(_) => {
            if argc != 1 {
                return None;
            }
            // Mirrors `callback_wants_two_args` for a compiled closure.
            let kv = if arity >= 2 {
                ArgMode::KeyValue
            } else {
                ArgMode::Pair
            };
            let (kind, args) = match name {
                "each" => (LoopKind::Each, kv),
                "each_value" => (LoopKind::Each, ArgMode::Value),
                "each_key" => (LoopKind::Each, ArgMode::Key),
                "map" => (LoopKind::HashMap, kv),
                "filter" | "select" | "keep_if" => (
                    LoopKind::HashFilter {
                        keep_when_truthy: true,
                    },
                    kv,
                ),
                "reject" | "delete_if" => (
                    LoopKind::HashFilter {
                        keep_when_truthy: false,
                    },
                    kv,
                ),
                "any?" => (
                    LoopKind::AnyAll {
                        stop_on: true,
                        answer_on_stop: true,
                    },
                    kv,
                ),
                "all?" => (
                    LoopKind::AnyAll {
                        stop_on: false,
                        answer_on_stop: false,
                    },
                    kv,
                ),
                "transform_values" => (LoopKind::HashTransformValues, ArgMode::Value),
                "transform_keys" => (LoopKind::HashTransformKeys, ArgMode::Key),
                _ => return None,
            };
            let passed = if args == ArgMode::KeyValue { 2 } else { 1 };
            // Exactly the declared parameters, or none at all (`h.each { … }`
            // gets no argument slot, as the tree-walker binds it nothing).
            match params {
                0 => Some((kind, ArgMode::Nothing)),
                _ if params == passed => Some((kind, args)),
                _ => None,
            }
        }
        Value::Array(array) => {
            let kind = match name {
                "each" | "each_with_index" => LoopKind::Each,
                "map" => LoopKind::ArrayMap,
                "filter" | "select" => LoopKind::ArrayFilter {
                    keep_when_truthy: true,
                },
                "reject" => LoopKind::ArrayFilter {
                    keep_when_truthy: false,
                },
                "find" => LoopKind::ArrayFind,
                "any?" => LoopKind::AnyAll {
                    stop_on: true,
                    answer_on_stop: true,
                },
                "all?" => LoopKind::AnyAll {
                    stop_on: false,
                    answer_on_stop: false,
                },
                "none?" => LoopKind::AnyAll {
                    stop_on: true,
                    answer_on_stop: false,
                },
                "one?" => LoopKind::ArrayOne,
                "count" => LoopKind::ArrayCount,
                "reduce" | "fold" => LoopKind::ArrayReduce,
                "sort_by" => LoopKind::ArraySortBy,
                _ => return None,
            };
            let args = match kind {
                LoopKind::ArrayReduce => {
                    // A seed may follow the callback. Without one the first
                    // element seeds, and an empty array raises (natively).
                    if argc > 2 || (argc == 1 && array.borrow().is_empty()) {
                        return None;
                    }
                    match params {
                        0 => ArgMode::Nothing,
                        1 => ArgMode::AccPair,
                        2 => ArgMode::AccItem,
                        _ => return None,
                    }
                }
                _ if argc != 1 => return None,
                // Two arguments: the element, then its index.
                _ if name == "each_with_index" => match params {
                    0 => ArgMode::Nothing,
                    1 => ArgMode::Value,
                    2 => ArgMode::ItemIndex,
                    _ => return None,
                },
                _ => match params {
                    0 => ArgMode::Nothing,
                    1 => ArgMode::Value,
                    _ if arity == params => ArgMode::Parts(params),
                    _ => return None,
                },
            };
            Some((kind, args))
        }
        Value::Int(_) => {
            match (name, argc) {
                ("times", 1) => {}
                ("upto" | "downto", 2) if matches!(operands[1], Value::Int(_)) => {}
                _ => return None,
            }
            let args = match params {
                0 => ArgMode::Nothing,
                1 => ArgMode::Value,
                _ => return None,
            };
            Some((LoopKind::Each, args))
        }
        _ => None,
    }
}

impl Vm {
    /// Start `receiver.name(…)` as an in-VM loop when it has the common
    /// shape. The receiver is at `receiver_idx` with `argc` arguments above
    /// it: the callback, except that `reduce`'s seed may follow it and
    /// `upto` / `downto` take their limit before it. Returns `false`, having
    /// changed nothing, when the call should take the native driver.
    pub(crate) fn try_start_callback_loop(
        &mut self,
        receiver_idx: usize,
        argc: usize,
        name: &str,
    ) -> Result<bool, RuntimeError> {
        if self.frames.len() >= MAX_CALL_DEPTH {
            return Ok(false);
        }
        let callback_idx = match (&self.stack[receiver_idx], argc) {
            (Value::Int(_), 2) => receiver_idx + 2,
            (_, 1 | 2) => receiver_idx + 1,
            _ => return Ok(false),
        };
        // Only a compiled closure: any other callee keeps the native driver.
        let Value::VmClosure(callback) = &self.stack[callback_idx] else {
            return Ok(false);
        };
        let params = callback.proto.param_names.len();
        let arity = callback.proto.arity as usize;
        let Some((kind, args)) = loop_shape(&self.stack[receiver_idx..], name, params, arity)
        else {
            return Ok(false);
        };
        // A template reads each parameter as one value of the element. A
        // `[key, value]` pair is an array the body may keep or change, and
        // the index, the accumulator and an element's parts are not the
        // element: those shapes run every element on the frame path.
        let kernel = match args {
            ArgMode::KeyValue | ArgMode::Key | ArgMode::Value | ArgMode::Nothing
                if !matches!(kind, LoopKind::ArrayReduce) =>
            {
                Kernel::of(&callback.proto, params)
            }
            _ => None,
        };
        // A function compiled to machine code (`crate::native`) keeps the
        // native driver, which calls that code for each element with no
        // frame, where the loop's frame would run its bytecode instead
        // (`xs.map(score)` for a typed `def score(n: Int) -> Int` with a loop
        // in it: about 20 times slower). Unless its body is a template, which
        // costs less than either.
        if callback.proto.kernel.is_some() && kernel.is_none() {
            return Ok(false);
        }
        // Checks done: move the callback, the receiver and the method's other
        // argument off the stack into the loop — no reference-count round
        // trip for any of them. The method's result is pushed where the
        // receiver was when the loop ends.
        let mut operands = self.stack.drain(receiver_idx..);
        let receiver = operands.next().expect("the receiver");
        let first = operands.next().expect("the callback or the limit");
        let second = operands.next();
        drop(operands);
        let (callback, other) = if callback_idx == receiver_idx + 1 {
            (first, second)
        } else {
            (second.expect("the callback after the limit"), Some(first))
        };
        let Value::VmClosure(closure) = callback else {
            unreachable!("checked as a compiled closure above");
        };
        // Where the walk starts, and `reduce`'s accumulator: the seed, or
        // the first element (already checked to exist) with the walk from
        // the second.
        let (source, len, index, value) = match receiver {
            Value::Hash(hash) => {
                let len = hash.borrow().len();
                (Source::Hash(hash), len, 0, Value::Null)
            }
            Value::Array(array) => {
                let len = array.borrow().len();
                let (index, value) = match (kind, other) {
                    (LoopKind::ArrayReduce, Some(seed)) => (0, seed),
                    (LoopKind::ArrayReduce, None) => (1, array.borrow()[0].clone()),
                    _ => (0, Value::Null),
                };
                (Source::Array(array), len, index, value)
            }
            Value::Int(n) => {
                let limit = match other {
                    Some(Value::Int(limit)) => limit,
                    _ => 0,
                };
                // `times` counts 0..n, `upto` n..=limit, `downto` n down to
                // limit: as many elements as the native driver's loop.
                let (start, step, len) = match name {
                    "times" => (0, 1, usize::try_from(n.max(0)).unwrap_or(usize::MAX)),
                    "upto" => (n, 1, inclusive_count(n, limit)),
                    _ => (n, -1, inclusive_count(limit, n)),
                };
                let source = Source::Ints {
                    start,
                    step,
                    receiver: n,
                };
                (source, len, 0, Value::Null)
            }
            _ => unreachable!("checked as a hash, an array or an int above"),
        };
        let presized = matches!(
            kind,
            LoopKind::HashMap
                | LoopKind::HashFilter { .. }
                | LoopKind::HashTransformValues
                | LoopKind::HashTransformKeys
        );
        let mirror = matches!(kind, LoopKind::HashTransformValues);
        let out_hash = match &source {
            // Copying the receiver copies its index table as-is: nothing is
            // rehashed, and each value is then overwritten in place.
            Source::Hash(hash) if mirror => hash.borrow().clone(),
            _ if presized => HashPairs::with_capacity(len),
            _ => HashPairs::default(),
        };
        let out_array = if matches!(kind, LoopKind::ArrayMap | LoopKind::ArrayFilter { .. }) {
            Vec::with_capacity(len)
        } else {
            Vec::new()
        };
        let keyed = if matches!(kind, LoopKind::ArraySortBy) {
            Vec::with_capacity(len)
        } else {
            Vec::new()
        };
        let answer = match kind {
            LoopKind::AnyAll { answer_on_stop, .. } => !answer_on_stop,
            _ => false,
        };
        // Refill the spare state when there is one: no allocation, no move of
        // the whole struct into a fresh box.
        let state = match self.spare_callback_loop.take() {
            Some(mut spare) => {
                spare.kind = kind;
                spare.args = args;
                spare.source = source;
                spare.closure = Some(closure);
                spare.index = index;
                spare.len = len;
                spare.current = None;
                spare.out_hash = out_hash;
                spare.out_array = out_array;
                spare.answer = answer;
                spare.value = value;
                spare.count = 0;
                spare.keyed = keyed;
                spare.mirror = mirror;
                spare.kernel = kernel;
                spare.touched = false;
                spare
            }
            None => Box::new(CallbackLoop {
                kind,
                args,
                source,
                closure: Some(closure),
                index,
                len,
                current: None,
                out_hash,
                out_array,
                answer,
                value,
                count: 0,
                keyed,
                mirror,
                kernel,
                touched: false,
            }),
        };
        self.iter_stack.push(IterState::Callback(state));
        self.next_callback()?;
        Ok(true)
    }

    /// A frame that drives a loop returned `result`: record it, then open
    /// the next element's frame or finish.
    pub(crate) fn callback_returned(&mut self, result: Value) -> Result<(), RuntimeError> {
        self.record_callback_result(result)?;
        self.next_callback()
    }

    /// Fold one callback result into the loop. An error (a `map` pair whose
    /// key cannot be a hash key) fails the method at its call site, as the
    /// native driver does, and takes the loop with it.
    fn record_callback_result(&mut self, result: Value) -> Result<(), RuntimeError> {
        let Some(IterState::Callback(state)) = self.iter_stack.last_mut() else {
            unreachable!("a loop-driving frame returned with no loop on the iterator stack");
        };
        if let Some(message) = record(state, result) {
            self.iter_stack.pop();
            return Err(RuntimeError::type_error(message, self.current_span()));
        }
        Ok(())
    }

    /// The next element's callback arguments, advancing the loop, or `None`
    /// when it is done. Read afresh each time: a callback may have changed
    /// the collection, exactly as the native drivers allow.
    fn next_callback_args(&mut self) -> Result<Option<CallbackArgs>, NotAnArray> {
        let Some(IterState::Callback(state)) = self.iter_stack.last_mut() else {
            unreachable!("no loop on the iterator stack");
        };
        if state.index >= state.len {
            return Ok(None);
        }
        let keeps_entry = keeps_entry(state);
        let args = match &state.source {
            Source::Array(array) => {
                let Some(item) = array.borrow().get(state.index).cloned() else {
                    return Ok(None);
                };
                if keeps_entry {
                    state.current = Some((HashKey::Null, item.clone()));
                }
                match state.args {
                    ArgMode::Value => CallbackArgs::One(item),
                    ArgMode::Nothing => CallbackArgs::Zero,
                    ArgMode::ItemIndex => CallbackArgs::Two(item, Value::Int(state.index as i64)),
                    ArgMode::AccItem => {
                        CallbackArgs::Two(std::mem::replace(&mut state.value, Value::Null), item)
                    }
                    ArgMode::AccPair => {
                        let acc = std::mem::replace(&mut state.value, Value::Null);
                        CallbackArgs::One(Value::Array(Rc::new(RefCell::new(vec![acc, item]))))
                    }
                    ArgMode::Parts(count) => match item {
                        Value::Array(parts) => CallbackArgs::Parts(parts, count),
                        _ => return Err(NotAnArray),
                    },
                    ArgMode::KeyValue | ArgMode::Pair | ArgMode::Key => {
                        unreachable!("a hash argument mode on an array loop")
                    }
                }
            }
            Source::Ints { start, step, .. } => match state.args {
                ArgMode::Nothing => CallbackArgs::Zero,
                _ => CallbackArgs::One(Value::Int(int_at(*start, *step, state.index))),
            },
            Source::Empty => unreachable!("a running loop has a source"),
            Source::Hash(hash) => {
                let hash = hash.borrow();
                let Some((key, value)) = hash.get_index(state.index) else {
                    if state.mirror {
                        state.out_hash.truncate(state.index);
                    }
                    return Ok(None);
                };
                if state.mirror
                    && state.out_hash.get_index(state.index).map(|(k, _)| k) != Some(key)
                {
                    state.out_hash.truncate(state.index);
                    state.mirror = false;
                }
                if keeps_entry
                    || (matches!(state.kind, LoopKind::HashTransformValues) && !state.mirror)
                {
                    state.current = Some((clone_key_fast(key), clone_scalar_fast(value)));
                }
                match state.args {
                    ArgMode::KeyValue => CallbackArgs::Two(key_value_fast(key), value.clone()),
                    ArgMode::Pair => CallbackArgs::One(Value::Array(Rc::new(RefCell::new(vec![
                        key_value_fast(key),
                        value.clone(),
                    ])))),
                    ArgMode::Key => CallbackArgs::One(key_value_fast(key)),
                    ArgMode::Value => CallbackArgs::One(value.clone()),
                    ArgMode::Nothing => CallbackArgs::Zero,
                    ArgMode::ItemIndex
                    | ArgMode::AccItem
                    | ArgMode::AccPair
                    | ArgMode::Parts(_) => {
                        unreachable!("an array argument mode on a hash loop")
                    }
                }
            }
        };
        state.index += 1;
        Ok(Some(args))
    }

    /// Open the frame for the next element, or end the loop and push the
    /// method's result.
    fn next_callback(&mut self) -> Result<(), RuntimeError> {
        // A template runs the elements it can right here, with no frame.
        let Some(IterState::Callback(state)) = self.iter_stack.last_mut() else {
            unreachable!("no loop on the iterator stack");
        };
        if let Some(kernel) = state.kernel {
            match run_kernel(state, kernel, &mut self.stack) {
                Ok(KernelRun::Done) => return self.finish_callback_loop(),
                Ok(KernelRun::Fallback) => {}
                Err(message) => {
                    self.iter_stack.pop();
                    return Err(RuntimeError::type_error(message, self.current_span()));
                }
            }
        }
        let args = match self.next_callback_args() {
            Ok(Some(args)) => args,
            Ok(None) => return self.finish_callback_loop(),
            Err(NotAnArray) => return Err(self.parts_of_a_non_array()),
        };
        if let Some(IterState::Callback(state)) = self.iter_stack.last_mut() {
            state.touched = true;
        }
        let Some(IterState::Callback(state)) = self.iter_stack.last() else {
            unreachable!("no loop on the iterator stack");
        };
        let closure = state
            .closure
            .clone()
            .expect("a running loop has its callback");
        self.stack.push(Value::VmClosure(closure.clone()));
        let argc = args.push_onto(&mut self.stack);
        let stack_base = self.stack.len() - argc - 1;
        let mut frame = CallFrame::new(
            closure,
            stack_base,
            self.iter_stack.len(),
            None,
            positional_supplied_mask(argc),
        );
        frame.drives_loop = true;
        self.frames.push(frame);
        Ok(())
    }

    /// End a loop whose [`ArgMode::Parts`] callback met an element that is
    /// not an array, with the native driver's error. It calls such a
    /// callback with the element alone, and the call fails its arity check
    /// at the method's call site. The callback's frame must already be gone,
    /// so that the error points at the call, as natively.
    #[cold]
    fn parts_of_a_non_array(&mut self) -> RuntimeError {
        let Some(IterState::Callback(state)) = self.iter_stack.pop() else {
            unreachable!("no loop on the iterator stack");
        };
        let ArgMode::Parts(declared) = state.args else {
            unreachable!("only a destructuring callback reads an element's parts");
        };
        RuntimeError::wrong_arity(declared, 1, self.current_span())
    }

    /// End a loop whose callback, still the top frame, returned a result
    /// that fails the method (`message`: a `map` pair's key or a transformed
    /// key that cannot be a hash key). The frame goes first, as on the
    /// general path, whose `Return` pops it before `callback_returned`: the
    /// error then points at the method's call site, as natively and on the
    /// tree-walker, and not at whatever op the callback last wrote its
    /// instruction pointer back for. Its cells, iterators and handlers are
    /// already gone (`run_fast`'s `Return`).
    #[cold]
    fn rerun_loop_failed(&mut self, message: &'static str, stack_base: usize) -> RuntimeError {
        self.frames.pop();
        self.stack.truncate(stack_base);
        self.iter_stack.pop();
        RuntimeError::type_error(message, self.current_span())
    }

    /// The loop callback on top of the frame stack returned `result`, and
    /// its locals above the arguments are already gone: record the result
    /// and run the same frame again for the next element — its argument
    /// slots overwritten in place, no frame popped or pushed. Returns `true`
    /// when the frame is ready to restart at ip 0; `false` when the loop has
    /// ended, the frame is gone and the method's result has been pushed.
    #[inline]
    pub(crate) fn rerun_loop_callback(
        &mut self,
        result: Value,
        stack_base: usize,
    ) -> Result<bool, RuntimeError> {
        // One lookup of the loop for the whole step — record the result, then
        // write the next element into the argument slots — with `iter_stack`
        // and `stack` borrowed separately.
        let Some(IterState::Callback(state)) = self.iter_stack.last_mut() else {
            unreachable!("a loop-driving frame returned with no loop on the iterator stack");
        };
        let mut pooled = None;
        let recorded = match (state.kind, result) {
            // A `map` callback's fresh `[key, value]` pair that nothing else
            // references: move the entry out instead of cloning it, and keep
            // the emptied array for the next `Op::Array`.
            (LoopKind::HashMap, Value::Array(pair))
                if Rc::strong_count(&pair) == 1
                    && Rc::weak_count(&pair) == 0
                    && pair.borrow().len() == 2 =>
            {
                let (key, value) = {
                    let mut cells = pair.borrow_mut();
                    let value = cells.pop().expect("length checked");
                    let key = cells.pop().expect("length checked");
                    (key, value)
                };
                match HashKey::from_value_owned(key) {
                    Some(key) => {
                        state.out_hash.insert(key, value);
                        pooled = Some(pair);
                        None
                    }
                    None => Some("hash key must be hashable"),
                }
            }
            (_, result) => record(state, result),
        };
        if let Some(message) = recorded {
            return Err(self.rerun_loop_failed(message, stack_base));
        }
        if let Some(pair) = pooled {
            if self.array_pool.len() < ARRAY_POOL_MAX {
                self.array_pool.push(pair);
            }
        }
        if let Some(kernel) = state.kernel {
            match run_kernel(state, kernel, &mut self.stack) {
                Ok(KernelRun::Done) => {
                    self.frames.pop();
                    self.stack.truncate(stack_base);
                    self.finish_callback_loop()?;
                    return Ok(false);
                }
                Ok(KernelRun::Fallback) => {}
                Err(message) => return Err(self.rerun_loop_failed(message, stack_base)),
            }
        }
        let refilled = refill(state, &mut self.stack, stack_base + 1);
        if let Refill::Ready = refilled {
            return Ok(true);
        }
        self.frames.pop();
        self.stack.truncate(stack_base);
        if let Refill::NotAnArray = refilled {
            return Err(self.parts_of_a_non_array());
        }
        self.finish_callback_loop()?;
        Ok(false)
    }

    /// Pop the finished loop and push the method's result.
    fn finish_callback_loop(&mut self) -> Result<(), RuntimeError> {
        let Some(IterState::Callback(mut state)) = self.iter_stack.pop() else {
            unreachable!("finish_callback_loop with no loop on the iterator stack");
        };
        let source = std::mem::replace(&mut state.source, Source::Empty);
        let result = match state.kind {
            LoopKind::Each => match source {
                Source::Hash(hash) => Value::Hash(hash),
                Source::Array(array) => Value::Array(array),
                Source::Ints { receiver, .. } => Value::Int(receiver),
                Source::Empty => unreachable!("a running loop has a source"),
            },
            LoopKind::AnyAll { .. } | LoopKind::ArrayOne => Value::Bool(state.answer),
            LoopKind::HashMap
            | LoopKind::HashFilter { .. }
            | LoopKind::HashTransformValues
            | LoopKind::HashTransformKeys => {
                Value::Hash(Rc::new(RefCell::new(std::mem::take(&mut state.out_hash))))
            }
            LoopKind::ArrayMap | LoopKind::ArrayFilter { .. } => {
                Value::Array(Rc::new(RefCell::new(std::mem::take(&mut state.out_array))))
            }
            LoopKind::ArrayFind | LoopKind::ArrayReduce => {
                std::mem::replace(&mut state.value, Value::Null)
            }
            LoopKind::ArrayCount => Value::Int(state.count),
            LoopKind::ArraySortBy => {
                // The native driver's sort, on the same pairs: the same order.
                let mut keyed = std::mem::take(&mut state.keyed);
                keyed.sort_by(|a, b| {
                    crate::interpreter::executor::calls::array_ops::compare_sort_values(&a.1, &b.1)
                });
                let sorted: Vec<Value> = keyed.into_iter().map(|(item, _)| item).collect();
                Value::Array(Rc::new(RefCell::new(sorted)))
            }
        };
        // Keep the state for the next loop, holding nothing alive. The block
        // goes back to its `fn` expression when this was its last reference
        // (`Vm::recycle_closure`).
        if let Some(closure) = state.closure.take() {
            self.recycle_closure(closure);
        }
        state.current = None;
        self.spare_callback_loop = Some(state);
        self.stack.push(result);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::interpreter::value::Value;
    use crate::lexer::Scanner;
    use crate::parser::Parser;
    use crate::vm::compiler::Compiler;
    use crate::vm::vm::Vm;

    /// Run `source` on a fresh VM. Returns the global `x` as text, and
    /// whether an in-VM loop ran to its end: a finished loop leaves its
    /// state behind as the spare, and nothing else does.
    fn run(source: &str) -> Result<(String, bool), String> {
        let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        let module = Compiler::compile(&program).expect("compile error");
        let mut vm = Vm::new();
        vm.execute(&module.main).map_err(|e| e.to_string())?;
        let x = match vm.globals.get("x") {
            Some(Value::String(s)) => s.to_string(),
            other => format!("{other:?}"),
        };
        Ok((x, vm.spare_callback_loop.is_some()))
    }

    /// Each source holds one iterator call, which must run on the in-VM loop
    /// and leave `x` as given. The expected values are what the native
    /// drivers return for the same calls.
    fn assert_looped(cases: &[(&str, &str)]) {
        for (source, expected) in cases {
            let (x, looped) = run(source).unwrap_or_else(|e| panic!("{source}\n=> {e}"));
            assert_eq!(x, *expected, "{source}");
            assert!(looped, "did not take the in-VM loop: {source}");
        }
    }

    /// The array block methods that used to re-enter `run()` for every
    /// element now run in the dispatch loop: on the frame path, and through
    /// a template when the body is one (`v > 2`).
    #[test]
    fn array_block_methods_take_the_in_vm_loop() {
        assert_looped(&[
            // `each_with_index` passes (item, index), the item alone, or
            // nothing, and returns its receiver.
            (
                "let seen = []\nlet back = [5, 6].each_with_index { |v, i| seen.push(v * 10 + i) }\nlet x = \"#{seen} #{back}\"",
                "[50, 61] [5, 6]",
            ),
            ("let seen = []\n[5, 6].each_with_index { |v| seen.push(v) }\nlet x = \"#{seen}\"", "[5, 6]"),
            ("let n = 0\n[5, 6].each_with_index { n = n + 1 }\nlet x = \"#{n}\"", "2"),
            ("let x = \"#{[1, 2, 3, 4].reject { |v| v % 2 == 0 }}\"", "[1, 3]"),
            ("let x = \"#{[1, 2, 3, 4].reject { |v| v > 2 }}\"", "[1, 2]"),
            ("let x = \"#{[1, 2, 3, 4].find { |v| v > 2 }}\"", "3"),
            ("let x = \"#{[1, 2, 3, 4].find { |v| v % 3 == 0 }}\"", "3"),
            ("let x = \"#{[1, 2].find { |v| v > 5 }}\"", "null"),
            ("let x = \"#{[1, 2].any? { |v| v > 1 }}\"", "true"),
            ("let x = \"#{[1, 2].any? { |v| v > 5 }}\"", "false"),
            ("let x = \"#{[1, 2].all? { |v| v > 0 }}\"", "true"),
            ("let x = \"#{[1, 2].all? { |v| v > 1 }}\"", "false"),
            ("let x = \"#{[1, 2].none? { |v| v > 5 }}\"", "true"),
            ("let x = \"#{[1, 2].none? { |v| v > 1 }}\"", "false"),
            ("let x = \"#{[1, 2, 3].one? { |v| v > 2 }}\"", "true"),
            ("let x = \"#{[1, 2, 3].one? { |v| v > 1 }}\"", "false"),
            ("let x = \"#{[1, 2, 3].one? { |v| v > 5 }}\"", "false"),
            // `count` has a method id: `CallMethodById` takes the loop too.
            ("let x = \"#{[1, 2, 3].count { |v| v > 1 }}\"", "2"),
            ("let x = \"#{[1, 2, 3, 4].count { |v| v % 2 == 0 }}\"", "2"),
            ("let x = \"#{[3, 1, 2].sort_by { |v| v }}\"", "[1, 2, 3]"),
            ("let x = \"#{[3, 1, 2].sort_by { |v| v * -1 }}\"", "[3, 2, 1]"),
            ("let x = \"#{[\"bb\", \"a\", \"ccc\"].sort_by { |s| s.length }}\"", "[a, bb, ccc]"),
            // Stable, as the native sort is: ties keep their order.
            (
                "let x = \"#{[[2, \"b\"], [1, \"a\"], [2, \"a\"], [1, \"b\"]].sort_by { |p| p[0] }}\"",
                "[[1, a], [1, b], [2, b], [2, a]]",
            ),
            // A template hands an element it cannot compute (a float against
            // an int constant) to the frame path, which runs the real opcode.
            ("let x = \"#{[1, 2.5, 3].count { |v| v > 1 }}\"", "2"),
            ("let x = \"#{[1, 2.5, 3].find { |v| v > 2 }}\"", "2.5"),
            ("let x = \"#{[0.5, 1, 2].any? { |v| v > 1 }}\"", "true"),
            ("let x = \"#{[3, 1.5, 2].sort_by { |v| v * 2 }}\"", "[1.5, 2, 3]"),
        ]);
    }

    /// `reduce` / `fold` keep the accumulator in the loop: seeded or not,
    /// with `(acc, item)`, the `[acc, item]` pair, or nothing.
    #[test]
    fn reduce_takes_the_in_vm_loop() {
        assert_looped(&[
            (
                "let x = \"#{[1, 2, 3].reduce(fn(acc, v) { acc + v }, 10)}\"",
                "16",
            ),
            (
                "let x = \"#{[1, 2, 3].reduce(fn(acc, v) { acc * v })}\"",
                "6",
            ),
            ("let x = \"#{[7].reduce(fn(acc, v) { acc + v })}\"", "7"),
            ("let x = \"#{[].reduce(fn(acc, v) { acc + v }, 5)}\"", "5"),
            (
                "let x = \"#{[1, 2].fold(fn(acc, v) { acc - v }, 0)}\"",
                "-3",
            ),
            (
                "let x = \"#{[\"a\", \"b\"].reduce(fn(acc, s) { acc + s }, \"\")}\"",
                "ab",
            ),
            (
                "let x = \"#{[1, 2, 3].reduce(fn(pair) { pair[0] + pair[1] }, 0)}\"",
                "6",
            ),
            (
                "let x = \"#{[1, 2, 3].reduce(fn(pair) { pair[0] * pair[1] })}\"",
                "6",
            ),
            ("let x = \"#{[1, 2, 3].reduce(fn() { 7 }, 0)}\"", "7"),
        ]);
    }

    /// `Int#times` / `upto` / `downto` walk their integers in the loop and
    /// return the receiver; an empty range runs nothing.
    #[test]
    fn int_iterators_take_the_in_vm_loop() {
        assert_looped(&[
            ("let seen = []\nlet r = 3.times { |i| seen.push(i) }\nlet x = \"#{seen} #{r}\"", "[0, 1, 2] 3"),
            ("let seen = []\nlet r = 2.upto(4) { |i| seen.push(i) }\nlet x = \"#{seen} #{r}\"", "[2, 3, 4] 2"),
            ("let seen = []\nlet r = 4.downto(2) { |i| seen.push(i) }\nlet x = \"#{seen} #{r}\"", "[4, 3, 2] 4"),
            ("let seen = []\nlet r = 0.times { |i| seen.push(i) }\nlet x = \"#{seen} #{r}\"", "[] 0"),
            ("let seen = []\nlet r = (0 - 3).times { |i| seen.push(i) }\nlet x = \"#{seen} #{r}\"", "[] -3"),
            ("let seen = []\nlet r = 5.upto(3) { |i| seen.push(i) }\nlet x = \"#{seen} #{r}\"", "[] 5"),
            ("let seen = []\nlet r = 1.downto(3) { |i| seen.push(i) }\nlet x = \"#{seen} #{r}\"", "[] 1"),
            ("let n = 0\n3.times { n = n + 1 }\nlet x = \"#{n}\"", "3"),
            // `total += i` is the accumulator template: no frame at all.
            (
                "def tally(n)\n  let total = 0\n  n.times { |i| total += i }\n  total\nend\nlet x = \"#{tally(5)}\"",
                "10",
            ),
            (
                "let seen = []\n9223372036854775807.upto(9223372036854775807) { |i| seen.push(i) }\nlet x = \"#{seen}\"",
                "[9223372036854775807]",
            ),
            (
                "let seen = []\n(0 - 9223372036854775807).downto(0 - 9223372036854775807) { |i| seen.push(i) }\nlet x = \"#{seen}\"",
                "[-9223372036854775807]",
            ),
        ]);
    }

    /// A block of two or more parameters over an array of arrays binds one
    /// part to each, in the loop: missing parts read as nil, extra ones are
    /// dropped, as `invoke_in_batch_one` destructures them.
    #[test]
    fn destructuring_blocks_take_the_in_vm_loop() {
        assert_looped(&[
            ("let x = \"#{[[1, 2], [3, 4]].map { |a, b| a + b }}\"", "[3, 7]"),
            ("let x = \"#{[[1, 2], [3]].map { |a, b| b }}\"", "[2, null]"),
            ("let x = \"#{[[1, 2, 3]].map { |a, b| a + b }}\"", "[3]"),
            ("let x = \"#{[[1, 2, 3], [4, 5, 6]].map { |a, b, c| a + b + c }}\"", "[6, 15]"),
            ("let x = \"#{[[1, 2], [3, 4]].filter { |a, b| b > 2 }}\"", "[[3, 4]]"),
            ("let x = \"#{[[1, 2], [3, 4]].reject { |a, b| a == 1 }}\"", "[[3, 4]]"),
            ("let x = \"#{[[1, 2], [3, 4]].find { |a, b| a > 2 }}\"", "[3, 4]"),
            ("let x = \"#{[[1, 2], [3, 4]].any? { |a, b| b == 4 }}\"", "true"),
            ("let x = \"#{[[1, 2], [3, 4]].count { |a, b| a + b > 3 }}\"", "1"),
            ("let x = \"#{[[\"b\", 2], [\"a\", 1]].sort_by { |k, v| v }}\"", "[[a, 1], [b, 2]]"),
            ("let total = 0\n[[1, 2], [3, 4]].each { |a, b| total = total + a * b }\nlet x = \"#{total}\"", "14"),
            // The block-less `each_with_index` builds the pairs natively; the
            // `map` over them destructures in the loop.
            ("let x = \"#{[5, 6].each_with_index.map { |v, i| v * i }}\"", "[0, 6]"),
        ]);
    }

    /// The early-exit methods stop where the native drivers stop: the block
    /// runs exactly as many times.
    #[test]
    fn early_exits_stop_at_the_same_element() {
        assert_looped(&[
            ("let n = 0\nlet r = [1, 2, 3, 4].any? do |v|\n  n = n + 1\n  v > 1\nend\nlet x = \"#{r} #{n}\"", "true 2"),
            ("let n = 0\nlet r = [1, 2, 3, 4].all? do |v|\n  n = n + 1\n  v < 2\nend\nlet x = \"#{r} #{n}\"", "false 2"),
            ("let n = 0\nlet r = [1, 2, 3, 4].none? do |v|\n  n = n + 1\n  v > 2\nend\nlet x = \"#{r} #{n}\"", "false 3"),
            ("let n = 0\nlet r = [1, 2, 3, 4].one? do |v|\n  n = n + 1\n  v > 1\nend\nlet x = \"#{r} #{n}\"", "false 3"),
            ("let n = 0\nlet r = [1, 2, 3, 4].find do |v|\n  n = n + 1\n  v > 2\nend\nlet x = \"#{r} #{n}\"", "3 3"),
            ("let n = 0\nlet r = [1, 2, 3, 4].count do |v|\n  n = n + 1\n  v > 2\nend\nlet x = \"#{r} #{n}\"", "2 4"),
        ]);
    }

    /// A block that changes the array it walks sees what the native drivers
    /// see: the live array, never past the length it had at the start, and
    /// no further than it has shrunk to.
    #[test]
    fn changes_to_the_receiver_are_seen_as_natively() {
        assert_looped(&[
            (
                "let ys = [1, 2, 3, 4]\nlet seen = []\nys.each_with_index do |y, i|\n  seen.push(y)\n  ys.pop() if i == 0\nend\nlet x = \"#{seen} #{ys}\"",
                "[1, 2, 3] [1, 2, 3]",
            ),
            (
                "let zs = [1, 2, 3]\nlet r = zs.find do |z|\n  zs.push(z + 10)\n  z > 5\nend\nlet x = \"#{r} #{zs}\"",
                "null [1, 2, 3, 11, 12, 13]",
            ),
            ("let ws = [1, 2, 3, 4]\nlet c = ws.count do |w|\n  ws.pop()\n  true\nend\nlet x = \"#{c} #{ws}\"", "2 [1, 2]"),
            (
                "let rs = [1, 2, 3, 4]\nlet r = rs.reject do |v|\n  rs.pop()\n  false\nend\nlet x = \"#{r} #{rs}\"",
                "[1, 2] [1, 2]",
            ),
            (
                "let ss = [3, 1, 2, 4]\nlet r = ss.sort_by do |s|\n  ss.pop()\n  s\nend\nlet x = \"#{r} #{ss}\"",
                "[1, 3] [3, 1]",
            ),
            (
                "let vs = [1, 2, 3]\nlet r = vs.reduce(fn(acc, v) {\n  vs.push(v) if acc < 3\n  acc + v\n}, 0)\nlet x = \"#{r} #{vs}\"",
                "6 [1, 2, 3, 1, 2]",
            ),
            (
                "let ps = [[1, 2], [3, 4], [5, 6]]\nlet r = ps.map do |a, b|\n  ps.pop()\n  a + b\nend\nlet x = \"#{r} #{ps.length}\"",
                "[3, 7] 1",
            ),
        ]);
    }

    /// A block that captures its own parameters closes their cells when it
    /// returns and runs again in the same frame, its argument slots
    /// overwritten: each capture keeps the value of its own pass.
    #[test]
    fn blocks_that_capture_their_parameters_keep_each_pass_values() {
        assert_looped(&[
            (
                "let fns = []\nlet r = [1, 2, 3].find do |v|\n  fns.push(fn() { v })\n  v > 1\nend\nlet x = \"#{r} #{fns.length} #{fns[0]()}\"",
                "2 2 1",
            ),
            (
                "let fns = []\nlet r = [1, 2, 3].reduce(fn(acc, v) {\n  fns.push(fn() { acc })\n  acc + v\n}, 0)\nlet x = \"#{r} #{fns[2]()}\"",
                "6 3",
            ),
            (
                "let fns = []\nlet r = [1, 2, 3].reduce(fn(pair) {\n  fns.push(fn() { pair })\n  pair[0] + pair[1]\n}, 0)\nlet x = \"#{r} #{fns[2]()}\"",
                "6 [3, 3]",
            ),
            (
                "let fns = []\n[5, 6].each_with_index do |v, i|\n  fns.push(fn() { v * 10 + i })\nend\nlet x = \"#{fns[0]()} #{fns[1]()}\"",
                "50 61",
            ),
            (
                "let fns = []\nlet r = [[1, 2], [3, 4]].map do |a, b|\n  fns.push(fn() { a + b })\n  a\nend\nlet x = \"#{r} #{fns[1]()}\"",
                "[1, 3] 7",
            ),
            ("let fns = []\n3.times do |i|\n  fns.push(fn() { i })\nend\nlet x = \"#{fns[2]()}\"", "2"),
            (
                "let fns = []\nlet r = [3, 1, 2].sort_by do |v|\n  fns.push(fn() { v })\n  v\nend\nlet x = \"#{r} #{fns[0]()}\"",
                "[1, 2, 3] 3",
            ),
        ]);
    }

    /// The first function compiled inside the first function of `source`.
    fn inner_proto(source: &str) -> std::sync::Arc<crate::vm::chunk::FunctionProto> {
        use crate::vm::chunk::Constant;
        let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        let module = Compiler::compile(&program).expect("compile error");
        let first = |constants: &[Constant]| {
            constants.iter().find_map(|constant| match constant {
                Constant::Function(proto) => Some(proto.clone()),
                _ => None,
            })
        };
        let outer = first(module.main.chunk.constants.as_slice()).expect("a function");
        first(outer.chunk.constants.as_slice()).expect("a function inside it")
    }

    /// `total += x`, `total = total + x` and `count += 1` in a block compile
    /// to `AddAssignUpvalue`, and the accumulator template is read from that
    /// op: such a body still runs with no frame. What the template does not
    /// compute (a mixed pair, strings) runs the opcode on the frame path.
    #[test]
    fn captured_accumulators_are_templates() {
        use super::{Kernel, Operand};
        for (body, params, param) in [
            ("n.times { |i| total += i }", 1, Some(1)),
            ("n.times { |i| total = total + i }", 1, Some(1)),
            ("n.times { total += 1 }", 0, None),
            ("n.times { |i| total += 2.5 }", 1, None),
            (
                "h = {\"a\": n}\n  h.each { |k, v| total = total + v }",
                2,
                Some(2),
            ),
        ] {
            let block = inner_proto(&format!(
                "def tally(n)\n  total = 0\n  {body}\n  total\nend"
            ));
            let recognised = match (Kernel::of(&block, params), param) {
                (Some(Kernel::Accumulate(_, Operand::Param(slot))), Some(expected)) => {
                    slot == expected
                }
                (Some(Kernel::Accumulate(_, Operand::Const(_))), None) => true,
                _ => false,
            };
            assert!(recognised, "{body}: {:?}", block.chunk.code);
        }
        assert_looped(&[
            (
                "def tally(n)\n  total = 0\n  n.times { total += 1 }\n  total\nend\nlet x = \"#{tally(4)}\"",
                "4",
            ),
            (
                "def tally(n)\n  total = 0.5\n  n.times { |i| total += 2.5 }\n  total\nend\nlet x = \"#{tally(2)}\"",
                "5.5",
            ),
            (
                "def tally(n)\n  total = 1\n  n.times { |i| total += 0.5 }\n  total\nend\nlet x = \"#{tally(3)}\"",
                "2.5",
            ),
            (
                "def glue(xs)\n  out = \"\"\n  xs.each { |s| out = out + s }\n  out\nend\nlet x = glue([\"a\", \"b\", \"c\"])",
                "abc",
            ),
        ]);
    }

    /// A loop inside another loop's block keeps its own state on top of the
    /// iterator stack, and hands the outer block its result.
    #[test]
    fn nested_loops_keep_their_own_state() {
        assert_looped(&[
            ("let x = \"#{[1, 2, 3].count { |v| [2, 3, 4].any? { |w| w == v } }}\"", "2"),
            (
                "let seen = []\n3.times { |i| 2.times { |j| seen.push(i * 10 + j) } }\nlet x = \"#{seen}\"",
                "[0, 1, 10, 11, 20, 21]",
            ),
            ("let x = \"#{[[1, 2], [3, 4]].reduce(fn(acc, row) { acc + row.find { |v| v > 1 } }, 0)}\"", "5"),
        ]);
    }

    /// A `throw` out of a block abandons its loop, the enclosing `catch`
    /// gets the thrown value, and the next loop starts clean.
    #[test]
    fn a_throw_abandons_the_loop() {
        for (source, expected) in [
            ("[1, 2].find { |v| throw \"f#{v}\" }", "f1"),
            ("3.times { |i| throw \"t#{i}\" if i == 1 }", "t1"),
            ("[1, 2].reduce(fn(a, v) { throw \"r#{v}\" }, 0)", "r1"),
            ("[1, 2].sort_by { |v| throw \"s#{v}\" }", "s1"),
            ("[[1, 2]].map { |a, b| throw \"p#{a}#{b}\" }", "p12"),
            (
                "[7, 8].each_with_index { |v, i| throw \"e#{v}#{i}\" if i == 1 }",
                "e81",
            ),
            ("[1, 2].count { |v| throw \"c#{v}\" }", "c1"),
        ] {
            let program = format!(
                "let x = \"\"\ntry\n  {source}\ncatch e\n  x = e\nend\nx = \"#{{x}} #{{[1, 2].map {{ |v| v * 2 }}}}\""
            );
            let (x, looped) = run(&program).unwrap_or_else(|e| panic!("{program}\n=> {e}"));
            assert_eq!(x, format!("{expected} [2, 4]"), "{program}");
            assert!(looped, "{program}");
        }
    }

    /// A destructuring block meets an element that is not an array: the
    /// native driver calls it with that element alone, which fails its arity
    /// check. The loop raises the same error at the same place, the method's
    /// call site, whether the element comes first, later, or after a frame
    /// that captured its parameters.
    #[test]
    fn a_destructuring_block_over_a_non_array_fails_as_natively() {
        for (source, expected) in [
            (
                "let x = \"\"\ntry\n  [3, [1, 2]].map { |a, b| a }\ncatch e\n  x = \"#{e}\"\nend",
                "Wrong number of arguments: expected 2, got 1 at 3:3",
            ),
            (
                "let x = \"\"\ntry\n  [[1, 2], 3].map { |a, b| a }\ncatch e\n  x = \"#{e}\"\nend",
                "Wrong number of arguments: expected 2, got 1 at 3:3",
            ),
            (
                "let fns = []\nlet x = \"\"\ntry\n  [[1, 2], 3].map do |a, b|\n    fns.push(fn() { a })\n    a\n  end\ncatch e\n  x = \"#{e}\"\nend",
                "Wrong number of arguments: expected 2, got 1 at 4:3",
            ),
            (
                "def pairs_sum(rows)\n  rows.map { |a, b| a + b }\nend\nlet x = \"\"\ntry\n  pairs_sum([[1, 2], 3])\ncatch e\n  x = \"#{e}\"\nend",
                "Wrong number of arguments: expected 2, got 1 at 2:3",
            ),
            (
                "def pairs_sum(rows)\n  rows.map { |a, b| a + b }\nend\nlet x = \"\"\ntry\n  pairs_sum([3])\ncatch e\n  x = \"#{e}\"\nend",
                "Wrong number of arguments: expected 2, got 1 at 2:3",
            ),
            // The VM is left in order: the next loop runs.
            (
                "let x = \"\"\ntry\n  [[1, 2], 3].count { |a, b| a }\ncatch e\n  x = \"#{e}\"\nend\nx = \"#{x} | #{[1, 2].map { |v| v + 1 }}\"",
                "Wrong number of arguments: expected 2, got 1 at 3:3 | [2, 3]",
            ),
        ] {
            let (x, _) = run(source).unwrap_or_else(|e| panic!("{source}\n=> {e}"));
            assert_eq!(x, expected, "{source}");
        }
    }

    /// What the loop does not take keeps the native driver and its errors:
    /// a non-closure argument, a default the call would fill, an arity the
    /// call refuses, a missing seed on an empty array, a bad limit.
    #[test]
    fn other_shapes_keep_the_native_driver() {
        for (source, expected) in [
            ("let x = \"#{[1, 2, 2].count(2)}\"", "2"),
            ("let x = \"#{[1, 2, 3].count()}\"", "3"),
            ("def f(a, b = 7)\n  a + b\nend\nlet x = \"#{[1, 2].map(f)}\"", "[8, 9]"),
            (
                "let x = \"\"\ntry\n  1.upto(\"x\") { |i| i }\ncatch e\n  x = \"#{e}\"\nend",
                "Type error: upto expects an integer limit at 3:3",
            ),
            (
                "let x = \"\"\ntry\n  3.times { |a, b| a }\ncatch e\n  x = \"#{e}\"\nend",
                "Wrong number of arguments: expected 2, got 1 at 3:3",
            ),
            (
                "let x = \"\"\ntry\n  [].reduce(fn(a, v) { a + v })\ncatch e\n  x = \"#{e}\"\nend",
                "Type error: reduce on empty array requires initial value at 3:3",
            ),
            (
                "let x = \"\"\ntry\n  [1, 2].each_with_index { |a, b, c| a }\ncatch e\n  x = \"#{e}\"\nend",
                "Wrong number of arguments: expected 3, got 2 at 3:3",
            ),
            (
                "let x = \"\"\ntry\n  [1, 2].reduce(fn(a, b, c) { a }, 0)\ncatch e\n  x = \"#{e}\"\nend",
                "Wrong number of arguments: expected 3, got 2 at 3:3",
            ),
            (
                "let x = \"\"\ntry\n  [1, 2].map(fn(v) { v }, 3)\ncatch e\n  x = \"#{e}\"\nend",
                "Wrong number of arguments: expected 1, got 2 at 3:3",
            ),
            (
                "let x = \"\"\ntry\n  [1, 2].reduce(fn(a, v) { a }, 0, 1)\ncatch e\n  x = \"#{e}\"\nend",
                "Wrong number of arguments: expected 2, got 3 at 3:3",
            ),
            (
                "let x = \"\"\ntry\n  3.times(fn(i) { i }, 1)\ncatch e\n  x = \"#{e}\"\nend",
                "Wrong number of arguments: expected 1, got 2 at 3:3",
            ),
        ] {
            let (x, looped) = run(source).unwrap_or_else(|e| panic!("{source}\n=> {e}"));
            assert_eq!(x, expected, "{source}");
            assert!(!looped, "took the in-VM loop: {source}");
        }
    }

    /// A function compiled to machine code keeps the native driver, which
    /// runs that code per element; a loop frame would run its bytecode. Its
    /// result is the same either way.
    #[cfg(feature = "native")]
    #[test]
    fn a_function_compiled_to_machine_code_keeps_the_native_driver() {
        let source = "def score(n: Int) -> Int\n  total = 0\n  i = 0\n  while i < 5\n    total = total + (n * i) % 7\n    i = i + 1\n  end\n  total\nend\nlet x = \"#{[3, 9, 4].map(score)}\"";
        let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        let kernels = crate::native::analyze_and_compile(&program, &|_| false)
            .expect("kernels compile (unless SOLI_NATIVE=0)");
        let module = Compiler::compile_with_kernels(&program, Some(kernels)).expect("compile");
        let mut vm = Vm::new();
        vm.execute(&module.main).expect("vm error");
        assert_eq!(
            vm.globals.get("x"),
            Some(&Value::String("[16, 13, 12]".into()))
        );
        assert!(
            vm.spare_callback_loop.is_none(),
            "a kernel's callee ran on the loop's frame"
        );
    }
}
