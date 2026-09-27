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
//! The loop state sits on `iter_stack` for a reason: `Return` truncates that
//! stack to the returning frame's base and exception unwinding to the
//! handler's depth, so a loop abandoned by a `throw` disappears without any
//! code here. Only the common shape is taken — a compiled closure taking
//! exactly the arguments the method passes; everything else keeps the
//! native driver in `vm_hash_methods.rs` / `vm_array_methods.rs`, whose
//! semantics this mirrors element for element.

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
    /// `each` / `each_value` / `each_key`: result ignored, returns the receiver.
    HashEach,
    /// `map`: a `[key, value]` result becomes an entry of the new hash.
    HashMap,
    /// `filter` / `select` / `keep_if` (keep when truthy) and `reject` /
    /// `delete_if` (keep when falsy): a new hash of the kept entries.
    HashFilter { keep_when_truthy: bool },
    /// `any?` (stop at the first truthy) / `all?` (stop at the first falsy).
    HashAnyAll { want_any: bool },
    /// `transform_values`: same keys, callback results as values.
    HashTransformValues,
    /// `transform_keys`: callback results as keys, same values.
    HashTransformKeys,
    /// Array `each`: result ignored, returns the receiver.
    ArrayEach,
    /// Array `map`.
    ArrayMap,
    /// Array `filter` / `select`.
    ArrayFilter,
}

/// What the callback receives for each hash entry.
#[derive(Clone, Copy, PartialEq, Eq)]
enum HashArgs {
    /// `(key, value)` — a callback declaring two or more parameters.
    KeyValue,
    /// `[key, value]` — a callback declaring fewer.
    Pair,
    Key,
    Value,
}

#[derive(Clone)]
enum Source {
    Hash(Rc<RefCell<HashPairs>>),
    Array(Rc<RefCell<Vec<Value>>>),
    /// A spare state between loops: holds nothing alive.
    Empty,
}

/// An iterator method in progress; lives on `iter_stack`.
#[derive(Clone)]
pub struct CallbackLoop {
    kind: LoopKind,
    args: HashArgs,
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
    /// `any?` / `all?` answer so far.
    answer: bool,
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
    One(Value),
    Two(Value, Value),
}

impl CallbackArgs {
    /// Push the arguments; returns how many.
    #[inline]
    fn push_onto(self, stack: &mut Vec<Value>) -> usize {
        match self {
            CallbackArgs::One(a) => {
                stack.push(a);
                1
            }
            CallbackArgs::Two(a, b) => {
                stack.push(a);
                stack.push(b);
                2
            }
        }
    }
}

/// Declared parameter count and arity of a compiled closure, or `None` for
/// any other callee — which keeps the native driver.
fn closure_params(callee: &Value) -> Option<(usize, usize)> {
    match callee {
        Value::VmClosure(closure) => Some((
            closure.proto.param_names.len(),
            closure.proto.arity as usize,
        )),
        _ => None,
    }
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
        LoopKind::HashEach | LoopKind::ArrayEach => {
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
        LoopKind::HashAnyAll { want_any } => {
            if truthy_fast(&result) == want_any {
                state.answer = want_any;
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
        LoopKind::ArrayFilter => {
            if truthy_fast(&result) {
                let (_, item) = state.current.take().expect("filter keeps its item");
                state.out_array.push(item);
            }
            super::vm::discard(result);
            None
        }
    }
}

/// Whether the running callback's entry must be kept for its result:
/// `filter` keeps it if the callback says so, `transform_keys` keeps the
/// value. `transform_values` keeps it only once it has left the mirrored
/// mode (see `CallbackLoop::mirror`), decided per element.
#[inline(always)]
fn keeps_entry(state: &CallbackLoop) -> bool {
    matches!(
        state.kind,
        LoopKind::HashFilter { .. } | LoopKind::HashTransformKeys | LoopKind::ArrayFilter
    )
}

/// Write the next element's arguments straight into the kept frame's
/// slots from `first_slot`, advancing the loop; `false` when it is done.
/// The same element reading as `next_callback_args`, without building the
/// arguments first and moving them into place.
#[inline(always)]
fn refill(state: &mut CallbackLoop, stack: &mut [Value], first_slot: usize) -> bool {
    if state.index >= state.len {
        return false;
    }
    state.touched = true;
    let keeps_entry = keeps_entry(state);
    match &state.source {
        Source::Array(array) => {
            let array = array.borrow();
            let Some(item) = array.get(state.index) else {
                return false;
            };
            if keeps_entry {
                state.current = Some((HashKey::Null, item.clone()));
            }
            replace_slot(stack, first_slot, clone_scalar_fast(item));
        }
        Source::Empty => unreachable!("a running loop has a source"),
        Source::Hash(hash) => {
            let hash = hash.borrow();
            let Some((key, value)) = hash.get_index(state.index) else {
                if state.mirror {
                    // The receiver shrank: only the visited entries count.
                    state.out_hash.truncate(state.index);
                }
                return false;
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
                HashArgs::KeyValue => {
                    replace_slot(stack, first_slot, key_value_fast(key));
                    replace_slot(stack, first_slot + 1, clone_scalar_fast(value));
                }
                HashArgs::Pair => {
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
                HashArgs::Key => replace_slot(stack, first_slot, key_value_fast(key)),
                HashArgs::Value => replace_slot(stack, first_slot, clone_scalar_fast(value)),
            }
        }
    }
    state.index += 1;
    true
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
    /// `GET_UPVALUE u; CONSTANT c | GET_LOCAL p; ADD; SET_UPVALUE u` —
    /// `fn(k, v) { total = total + v }`, `{ count += 1 }`: the accumulator a
    /// loop body most often is. Its one effect, the write to the captured
    /// variable, happens only once the sum is known to be exactly what
    /// `ADD` would produce.
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
            [Op::GetUpvalue(u), Op::Constant(c), Op::Add, Op::SetUpvalue(w), Op::Return, ..]
                if u == w =>
            {
                Some(Kernel::Accumulate(*u, Operand::Const(constant(*c)?)))
            }
            [Op::GetUpvalue(u), Op::GetLocal(p), Op::Add, Op::SetUpvalue(w), Op::Return, ..]
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
/// the new value as `SET_UPVALUE` leaves it. `None`, having written nothing,
/// unless both operands are ints (overflow checked) or both floats — the
/// cases `ADD` answers inline; the frame path does the rest.
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
                match kind {
                    LoopKind::ArrayEach => super::vm::discard(out),
                    LoopKind::ArrayMap => out_array.push(out),
                    LoopKind::ArrayFilter => {
                        if truthy_fast(&out) {
                            out_array.push(clone_scalar_fast(item));
                        }
                        super::vm::discard(out);
                    }
                    _ => unreachable!("an array loop"),
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
                    (HashArgs::KeyValue, 1) | (HashArgs::Key, _) => ArgRef::Key(key),
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
                    LoopKind::HashEach => super::vm::discard(out),
                    LoopKind::HashFilter { keep_when_truthy } => {
                        if truthy_fast(&out) == keep_when_truthy {
                            out_hash.insert(clone_key_fast(key), clone_scalar_fast(value));
                        }
                        super::vm::discard(out);
                    }
                    LoopKind::HashAnyAll { want_any } => {
                        if truthy_fast(&out) == want_any {
                            *answer = want_any;
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

impl Vm {
    /// Start `receiver.name(callback)` as an in-VM loop when it has the
    /// common shape. The receiver is at `receiver_idx` with `argc` arguments
    /// above it. Returns `false`, having changed nothing, when the call
    /// should take the native driver.
    pub(crate) fn try_start_callback_loop(
        &mut self,
        receiver_idx: usize,
        argc: usize,
        name: &str,
    ) -> Result<bool, RuntimeError> {
        if argc != 1 || self.frames.len() >= MAX_CALL_DEPTH {
            return Ok(false);
        }
        let Some((params, arity)) = closure_params(&self.stack[receiver_idx + 1]) else {
            return Ok(false);
        };
        // Mirrors `callback_wants_two_args` for a compiled closure.
        let wants_two = arity >= 2;
        let (kind, args, is_hash) = match &self.stack[receiver_idx] {
            Value::Hash(_) => {
                let kv = if wants_two {
                    HashArgs::KeyValue
                } else {
                    HashArgs::Pair
                };
                let (kind, args) = match name {
                    "each" => (LoopKind::HashEach, kv),
                    "each_value" => (LoopKind::HashEach, HashArgs::Value),
                    "each_key" => (LoopKind::HashEach, HashArgs::Key),
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
                    "any?" => (LoopKind::HashAnyAll { want_any: true }, kv),
                    "all?" => (LoopKind::HashAnyAll { want_any: false }, kv),
                    "transform_values" => (LoopKind::HashTransformValues, HashArgs::Value),
                    "transform_keys" => (LoopKind::HashTransformKeys, HashArgs::Key),
                    _ => return Ok(false),
                };
                (kind, args, true)
            }
            Value::Array(_) => {
                let kind = match name {
                    "each" => LoopKind::ArrayEach,
                    "map" => LoopKind::ArrayMap,
                    "filter" | "select" => LoopKind::ArrayFilter,
                    _ => return Ok(false),
                };
                (kind, HashArgs::Value, false)
            }
            _ => return Ok(false),
        };
        let passed = if args == HashArgs::KeyValue { 2 } else { 1 };
        // Exactly the declared parameters: no default to fill, no arity error
        // to raise — those stay with the native driver.
        if params != passed {
            return Ok(false);
        }
        // Checks done: move the callback and the receiver off the stack into
        // the loop — no reference-count round trip for either. The method's
        // result is pushed where the receiver was when the loop ends.
        let Some(Value::VmClosure(closure)) = self.stack.pop() else {
            unreachable!("checked as a compiled closure above");
        };
        let receiver = self
            .stack
            .pop()
            .expect("the receiver is below its argument");
        debug_assert_eq!(self.stack.len(), receiver_idx);
        let (source, len) = match receiver {
            Value::Hash(hash) if is_hash => {
                let len = hash.borrow().len();
                (Source::Hash(hash), len)
            }
            Value::Array(array) => {
                let len = array.borrow().len();
                (Source::Array(array), len)
            }
            _ => unreachable!("checked as a hash or an array above"),
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
        let out_array = if matches!(kind, LoopKind::ArrayMap | LoopKind::ArrayFilter) {
            Vec::with_capacity(len)
        } else {
            Vec::new()
        };
        let answer = matches!(kind, LoopKind::HashAnyAll { want_any: false });
        // A `[key, value]` pair argument is an array the body may keep or
        // change, so only the per-part shapes get a template.
        let kernel = if args == HashArgs::Pair {
            None
        } else {
            Kernel::of(&closure.proto, params)
        };
        // Refill the spare state when there is one: no allocation, no move of
        // the whole struct into a fresh box.
        let state = match self.spare_callback_loop.take() {
            Some(mut spare) => {
                spare.kind = kind;
                spare.args = args;
                spare.source = source;
                spare.closure = Some(closure);
                spare.index = 0;
                spare.len = len;
                spare.current = None;
                spare.out_hash = out_hash;
                spare.out_array = out_array;
                spare.answer = answer;
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
                index: 0,
                len,
                current: None,
                out_hash,
                out_array,
                answer,
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
    fn next_callback_args(&mut self) -> Option<CallbackArgs> {
        let Some(IterState::Callback(state)) = self.iter_stack.last_mut() else {
            unreachable!("no loop on the iterator stack");
        };
        if state.index >= state.len {
            return None;
        }
        let keeps_entry = keeps_entry(state);
        let args = match &state.source {
            Source::Array(array) => {
                let item = array.borrow().get(state.index)?.clone();
                if keeps_entry {
                    state.current = Some((HashKey::Null, item.clone()));
                }
                CallbackArgs::One(item)
            }
            Source::Empty => unreachable!("a running loop has a source"),
            Source::Hash(hash) => {
                let hash = hash.borrow();
                let Some((key, value)) = hash.get_index(state.index) else {
                    if state.mirror {
                        state.out_hash.truncate(state.index);
                    }
                    return None;
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
                    HashArgs::KeyValue => CallbackArgs::Two(key_value_fast(key), value.clone()),
                    HashArgs::Pair => CallbackArgs::One(Value::Array(Rc::new(RefCell::new(vec![
                        key_value_fast(key),
                        value.clone(),
                    ])))),
                    HashArgs::Key => CallbackArgs::One(key_value_fast(key)),
                    HashArgs::Value => CallbackArgs::One(value.clone()),
                }
            }
        };
        state.index += 1;
        Some(args)
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
        let Some(args) = self.next_callback_args() else {
            return self.finish_callback_loop();
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
            self.iter_stack.pop();
            return Err(RuntimeError::type_error(message, self.current_span()));
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
                Err(message) => {
                    self.iter_stack.pop();
                    return Err(RuntimeError::type_error(message, self.current_span()));
                }
            }
        }
        if refill(state, &mut self.stack, stack_base + 1) {
            return Ok(true);
        }
        self.frames.pop();
        self.stack.truncate(stack_base);
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
            LoopKind::HashEach | LoopKind::ArrayEach => match source {
                Source::Hash(hash) => Value::Hash(hash),
                Source::Array(array) => Value::Array(array),
                Source::Empty => unreachable!("a running loop has a source"),
            },
            LoopKind::HashAnyAll { .. } => Value::Bool(state.answer),
            LoopKind::HashMap
            | LoopKind::HashFilter { .. }
            | LoopKind::HashTransformValues
            | LoopKind::HashTransformKeys => {
                Value::Hash(Rc::new(RefCell::new(std::mem::take(&mut state.out_hash))))
            }
            LoopKind::ArrayMap | LoopKind::ArrayFilter => {
                Value::Array(Rc::new(RefCell::new(std::mem::take(&mut state.out_array))))
            }
        };
        // Keep the state for the next loop, holding nothing alive.
        state.closure = None;
        state.current = None;
        self.spare_callback_loop = Some(state);
        self.stack.push(result);
        Ok(())
    }
}
