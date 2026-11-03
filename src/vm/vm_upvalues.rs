//! Closures and their upvalue cells: making the closure a `fn` expression
//! evaluates to, capturing a local into a cell, closing cells when their
//! variables go out of scope, and reusing what an evaluation left behind.
//!
//! For `xs.map { |x| x * k }` in a method, the life of the block is:
//!
//! 1. `Op::Closure` makes it (`Vm::make_closure`). A closure that captures
//!    nothing is one shared instance per `fn` expression (`closure_cache`).
//!    One that captures takes the spare its expression left last time
//!    (`closure_spares`), or a new one, and a cell for each captured local:
//!    the cell already open on that slot, else one from `upvalue_pool`, else
//!    a new one.
//! 2. When the iterator loop is done with the block and holds the last
//!    reference to it, `Vm::recycle_closure` empties its upvalues and keeps
//!    it as that expression's spare.
//! 3. When the method returns, `Vm::close_upvalues` closes the cells of its
//!    slots. A cell some closure still holds keeps the variable's value; a
//!    cell nothing else holds needs none, and goes back to the pool.
//!
//! So once warm, evaluating a capturing block allocates nothing. Reusing a
//! closure or a cell is only ever done with the sole reference to it, so no
//! live closure can tell: closures are never `==`, and a cell's identity is
//! what links the closures that share a variable, which only live ones can.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use super::chunk::FunctionProto;
use super::upvalue::{Upvalue, VmClosure};
use super::vm::{Vm, CLOSURE_CACHE_SLOTS};

/// Most cells `Vm::upvalue_pool` keeps.
pub(crate) const UPVALUE_POOL_MAX: usize = 32;

/// `CLOSURE_CACHE_SLOTS` is `1 << SLOT_BITS`.
const SLOT_BITS: u32 = 6;
const _: () = assert!(CLOSURE_CACHE_SLOTS == 1 << SLOT_BITS);

/// The `closure_cache` / `closure_spares` slot of a `fn` expression, by the
/// address of its prototype.
#[inline]
pub(crate) fn closure_slot(proto: &Arc<FunctionProto>) -> usize {
    slot_of_address(Arc::as_ptr(proto) as usize)
}

/// Every bit of the address, mixed. The slot used to be `(address >> 4) % 64`:
/// prototypes all come from one allocator size class (320 bytes), so their
/// addresses step by multiples of 20 units of 16 bytes and reached only 16 of
/// the 64 slots — a 17th hot lambda already missed, and two lambdas 16 slots
/// apart evicted each other on every evaluation. A multiply–xorshift–multiply
/// (splitmix64's finaliser) spreads any stride over all of them.
#[inline]
fn slot_of_address(address: usize) -> usize {
    // Allocations are 16-byte aligned: the low four bits carry nothing.
    let mut x = (address >> 4) as u64;
    x = x.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 29;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    (x >> (64 - SLOT_BITS)) as usize
}

impl Vm {
    /// The closure `Op::Closure` pushes for the `fn` expression `proto`,
    /// evaluated in the current frame.
    ///
    /// Out of line: `run_fast` calls it, and its body would only crowd that
    /// loop.
    #[inline(never)]
    pub(crate) fn make_closure(&mut self, proto: &Arc<FunctionProto>) -> Rc<VmClosure> {
        let slot = closure_slot(proto);
        // A closure that captures nothing is immutable, and a closure's
        // identity is never observable (two closures are never `==`), so one
        // instance per `fn` expression serves every evaluation: a callback
        // literal inside a loop — `h.each(fn(k, v) …)` — does not allocate on
        // each pass.
        if proto.upvalue_descriptors.is_empty() {
            return match &self.closure_cache[slot] {
                Some(cached) if Arc::ptr_eq(&cached.proto, proto) => cached.clone(),
                _ => {
                    let fresh = Rc::new(VmClosure::new(proto.clone(), Vec::new()));
                    self.closure_cache[slot] = Some(fresh.clone());
                    fresh
                }
            };
        }
        // One that captures: the spare this expression's last closure left
        // once nothing else held it (`recycle_closure`), or a new one.
        let mut closure = match self.closure_spares[slot].take() {
            Some(spare) if Arc::ptr_eq(&spare.proto, proto) => spare,
            other => {
                self.closure_spares[slot] = other;
                Rc::new(VmClosure::new(
                    proto.clone(),
                    Vec::with_capacity(proto.upvalue_descriptors.len()),
                ))
            }
        };
        let upvalues = &mut Rc::get_mut(&mut closure)
            .expect("a closure being made has no other owner")
            .upvalues;
        let base = self
            .frames
            .last()
            .expect("a closure is made in a frame")
            .stack_base;
        for desc in &proto.upvalue_descriptors {
            let cell = if desc.is_local {
                self.capture_upvalue(base + desc.index as usize)
            } else {
                self.frames.last().unwrap().closure.upvalues[desc.index as usize].clone()
            };
            upvalues.push(cell);
        }
        closure
    }

    /// The open cell for stack slot `slot`, shared by every closure that
    /// captures that variable while it is live: the cell already open on it,
    /// else a pooled one, else a new one.
    fn capture_upvalue(&mut self, slot: usize) -> Rc<RefCell<Upvalue>> {
        // `open_upvalues` is sorted by slot, and the current frame's cells
        // are on top: walk down from there to the cell or to where it goes.
        let mut at = self.open_upvalues.len();
        while at > 0 {
            let (open_slot, cell) = &self.open_upvalues[at - 1];
            if *open_slot == slot {
                return cell.clone();
            }
            if *open_slot < slot {
                break;
            }
            at -= 1;
        }
        let cell = match self.upvalue_pool.pop() {
            Some(cell) => {
                debug_assert!(
                    Rc::strong_count(&cell) == 1 && Rc::weak_count(&cell) == 0,
                    "a pooled cell has no other owner"
                );
                *cell.borrow_mut() = Upvalue::Open(slot);
                cell
            }
            None => Rc::new(RefCell::new(Upvalue::Open(slot))),
        };
        self.open_upvalues.insert(at, (slot, cell.clone()));
        self.open_upvalue_top = self.open_upvalue_top.max(slot + 1);
        cell
    }

    /// Whether any upvalue is still open on a stack slot at or above
    /// `from_slot` — what `close_upvalues(from_slot)` would close.
    #[inline]
    pub(crate) fn has_open_upvalues_from(&self, from_slot: usize) -> bool {
        self.open_upvalue_top > from_slot
    }

    /// Close every cell still open on a stack slot at or above `from_slot`:
    /// the variables going out of scope — a returning frame's, a block's, an
    /// unwound `try`'s. Call it while those slots still hold their values.
    ///
    /// A cell some closure still holds takes the variable's value, so that
    /// closure keeps reading it. A cell nothing else holds can never be read
    /// again, so it is not given a value (nor kept alive by one): it goes
    /// back to `upvalue_pool`. That is the usual case for a block passed to
    /// an iterator, which the loop let go of (`recycle_closure`) before the
    /// method returned.
    pub fn close_upvalues(&mut self, from_slot: usize) {
        if !self.has_open_upvalues_from(from_slot) {
            return;
        }
        // Sorted by slot: the cells to close are the tail.
        while let Some(&(slot, _)) = self.open_upvalues.last() {
            if slot < from_slot {
                break;
            }
            let (slot, cell) = self.open_upvalues.pop().expect("checked by `last`");
            if Rc::strong_count(&cell) == 1 && Rc::weak_count(&cell) == 0 {
                if self.upvalue_pool.len() < UPVALUE_POOL_MAX {
                    self.upvalue_pool.push(cell);
                }
                continue;
            }
            let value = self.stack[slot].clone();
            *cell.borrow_mut() = Upvalue::Closed(value);
        }
        // Everything still open points below `from_slot`.
        self.open_upvalue_top = self.open_upvalues.last().map_or(0, |&(slot, _)| slot + 1);
    }

    /// Keep `closure`, a block an iterator loop has just finished with, as
    /// the spare for the next evaluation of its `fn` expression — when the
    /// loop held the last reference to it. Its cells are let go now, so that
    /// the method's return finds them held by nothing and pools them.
    /// Anything else is an ordinary drop.
    pub(crate) fn recycle_closure(&mut self, mut closure: Rc<VmClosure>) {
        // `get_mut`: no other `Rc` and no `Weak` — nothing can observe it.
        let Some(only) = Rc::get_mut(&mut closure) else {
            return;
        };
        // One that captures nothing is shared through `closure_cache`.
        if only.proto.upvalue_descriptors.is_empty() {
            return;
        }
        only.upvalues.clear();
        let slot = closure_slot(&only.proto);
        self.closure_spares[slot] = Some(closure);
    }
}

#[cfg(test)]
mod tests {
    use super::slot_of_address;
    use crate::interpreter::value::Value;
    use crate::lexer::Scanner;
    use crate::parser::Parser;
    use crate::span::Span;
    use crate::vm::compiler::Compiler;
    use crate::vm::upvalue::{Upvalue, VmClosure};
    use crate::vm::vm::Vm;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Run `source` on a fresh VM and hand it back, with the global `x` as
    /// text.
    fn run(source: &str) -> (Vm, String) {
        let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        let module = Compiler::compile(&program).expect("compile error");
        let mut vm = Vm::new();
        vm.execute(&module.main)
            .unwrap_or_else(|e| panic!("{source}\n=> {e}"));
        let x = match vm.globals.get("x") {
            Some(Value::String(s)) => s.to_string(),
            other => format!("{other:?}"),
        };
        (vm, x)
    }

    fn ints(values: &[i64]) -> Value {
        Value::Array(Rc::new(RefCell::new(
            values.iter().map(|n| Value::Int(*n)).collect(),
        )))
    }

    fn spares(vm: &Vm) -> Vec<*const VmClosure> {
        vm.closure_spares.iter().flatten().map(Rc::as_ptr).collect()
    }

    /// The steady state of a method whose block captures one of its
    /// parameters: each call takes the closure the previous call's loop let
    /// go of and the cell its return pooled, and leaves them behind again.
    #[test]
    fn a_capturing_block_reuses_its_closure_and_its_cell() {
        let (mut vm, _) = run("def scale(xs, k)\n  xs.map { |v| v * k }\nend");
        let scale = vm.globals.get("scale").cloned().expect("scale is defined");
        let mut kept = Vec::new();
        for k in [3, 4, 5] {
            let result = vm
                .call_value_direct(
                    scale.clone(),
                    &[ints(&[1, 2]), Value::Int(k)],
                    Span::default(),
                )
                .expect("scale runs");
            assert_eq!(result, ints(&[k, 2 * k]));
            let spare = spares(&vm);
            assert_eq!(spare.len(), 1, "the block is kept for the next call");
            kept.push(spare[0]);
            assert!(vm.open_upvalues.is_empty(), "every cell closed");
            assert_eq!(vm.upvalue_pool.len(), 1, "k's cell went back to the pool");
            // The pooled cell holds no value, and the spare no cell: nothing
            // of the call stays alive.
            assert!(matches!(*vm.upvalue_pool[0].borrow(), Upvalue::Open(_)));
            assert!(vm
                .closure_spares
                .iter()
                .flatten()
                .all(|c| c.upvalues.is_empty()));
        }
        assert!(
            kept.windows(2).all(|pair| pair[0] == pair[1]),
            "each call reused the same closure: {kept:?}"
        );
    }

    /// What escapes is never reused: a closure kept past the loop keeps its
    /// cells, which keep their values, and the next evaluation of the same
    /// expressions builds its own.
    #[test]
    fn what_escapes_keeps_its_own_closure_and_cells() {
        let (vm, x) = run(r##"
kept = []
def collect(xs, k, kept)
  xs.map do |v|
    kept.push(fn() { v * k })
    v
  end
end
collect([1, 2], 10, kept)
collect([3], 100, kept)
x = "#{kept.map { |f| f() }}"
"##);
        assert_eq!(x, "[10, 20, 300]");
        assert!(vm.open_upvalues.is_empty());
    }

    /// A loop's block whose own parameter is captured closes that cell at
    /// each return and runs again in the same frame. Seen from outside: the
    /// loop ends from that frame (`rerun_loop_callback`), where nothing else
    /// holds the block, so it is kept as a spare — after the general
    /// `Return`, which still held the popped frame, it would not be.
    #[test]
    fn a_block_capturing_its_parameter_reruns_its_frame() {
        let (vm, x) = run(r##"
def run(xs)
  fns = []
  doubled = xs.map do |v|
    fns.push(fn() { v })
    v * 2
  end
  "#{doubled} #{fns.map { |f| f() }}"
end
x = run([1, 2, 3])
"##);
        assert_eq!(x, "[2, 4, 6] [1, 2, 3]");
        assert!(
            vm.closure_spares
                .iter()
                .flatten()
                .any(|spare| spare.proto.upvalue_descriptors.len() == 1),
            "the map block (it captures `fns`) was not kept"
        );
        assert!(vm.open_upvalues.is_empty());
    }

    /// `open_upvalues` stays sorted when a closure captures a high slot
    /// before a low one, and a scope's `CloseUpvalue` closes only the cells
    /// above it: the function-level `a` stays open and shared, the
    /// per-iteration `b` gets a cell per pass.
    #[test]
    fn cells_stay_sorted_and_close_by_scope() {
        let (vm, x) = run(r##"
def mixed
  fns = []
  a = 1
  i = 0
  while i < 2
    let b = i * 10
    fns.push(fn() { b + a })
    i += 1
  end
  a = 5
  fns.map { |f| f() }
end
x = "#{mixed()}"
"##);
        assert_eq!(x, "[5, 15]");
        assert!(vm.open_upvalues.is_empty());
        assert_eq!(vm.open_upvalue_top, 0);
    }

    /// The slot hash reaches far more than the 16 slots `(address >> 4) % 64`
    /// did for prototypes allocated a size class apart (320 bytes; 384 if the
    /// prototype grows), so 32 hot lambdas mostly stop evicting each other.
    #[test]
    fn the_closure_slot_spreads_prototypes_over_the_cache() {
        for stride in [320usize, 384] {
            for k in 0..8usize {
                let base = 0x7f00_0000_0000 + 0x10000 * k + 0x40 * k;
                let slots: std::collections::HashSet<usize> = (0..32)
                    .map(|i| slot_of_address(base + i * stride))
                    .collect();
                assert!(
                    slots.len() >= 20,
                    "stride {stride}, base {base:#x}: only {} slots",
                    slots.len()
                );
                assert!(slots.iter().all(|slot| *slot < super::CLOSURE_CACHE_SLOTS));
            }
        }
    }
}
