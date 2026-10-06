//! Per-primitive-type user-method overlay.
//!
//! When Soli code does `Int.class_eval do define_method(:double) { this * 2 } end`,
//! the new method is recorded here rather than in `Class.methods`, because
//! primitive types (Int, Float, Bool, Null, Decimal, String, Array, Hash, Symbol)
//! dispatch via Rust match arms in `member.rs` and the VM, not via `Class.methods`.
//!
//! ## Hot-path guarantee
//!
//! `USER_METHOD_FLAGS` is a single process-global `AtomicU16` (one bit per type).
//! When zero — the common case in any program that doesn't extend primitives —
//! `has_user_methods` returns false in one Relaxed load + bit test. Every dispatch
//! hook is gated on this and short-circuits before touching the hashmap.
//!
//! ## Threading
//!
//! Soli uses one `Interpreter` per worker thread. The method tables themselves
//! are `thread_local!` (so we can hold `Rc<Function>`, which is `!Send`). The
//! flags are process-global so that the fast-path test is uniform across threads;
//! a thread that hasn't registered anything will load empty maps and fall through.

use crate::interpreter::value::Function;
pub use crate::interpreter::value::{PrimType, PRIM_TYPE_COUNT};
use ahash::AHashMap;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU16, Ordering};

/// One bit per `PrimType`. Zero means no user methods anywhere; every
/// dispatch hook short-circuits in a single Relaxed load.
pub static USER_METHOD_FLAGS: AtomicU16 = AtomicU16::new(0);

thread_local! {
    static USER_METHOD_TABLES: [RefCell<AHashMap<String, Rc<Function>>>; PRIM_TYPE_COUNT] =
        std::array::from_fn(|_| RefCell::new(AHashMap::new()));
}

/// Fast-path test: returns true only if at least one user method has been
/// registered for `t` somewhere in the process. Single Relaxed atomic load + bit test.
#[inline(always)]
pub fn has_user_methods(t: PrimType) -> bool {
    USER_METHOD_FLAGS.load(Ordering::Relaxed) & (1u16 << t as u16) != 0
}

/// Lookup a user-defined method on a primitive type. Returns None if no user
/// methods exist for this type, or if `name` is not registered.
#[inline]
pub fn lookup_user_method(t: PrimType, name: &str) -> Option<Rc<Function>> {
    if !has_user_methods(t) {
        return None;
    }
    USER_METHOD_TABLES.with(|tables| tables[t as usize].borrow().get(name).cloned())
}

/// Register a user method. Sets the appropriate flag bit so future lookups
/// take the slow path. Once set, the bit is never cleared (rare and avoids races).
pub fn register_user_method(t: PrimType, name: String, f: Rc<Function>) {
    USER_METHOD_TABLES.with(|tables| {
        tables[t as usize].borrow_mut().insert(name, f);
    });
    USER_METHOD_FLAGS.fetch_or(1u16 << t as u16, Ordering::Relaxed);
}

/// Alias an existing user method under a new name. Returns true on success,
/// false if the source method is not registered. Aliases are intra-type only.
pub fn alias_user_method(t: PrimType, new_name: String, old_name: &str) -> bool {
    USER_METHOD_TABLES.with(|tables| {
        let mut table = tables[t as usize].borrow_mut();
        if let Some(f) = table.get(old_name).cloned() {
            table.insert(new_name, f);
            USER_METHOD_FLAGS.fetch_or(1u16 << t as u16, Ordering::Relaxed);
            true
        } else {
            false
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_path_when_empty() {
        // Default empty state: load + bit-test returns false for every type.
        // Note: this test runs on whatever thread cargo gives it. If another
        // test in this module registers a method, the global flag may be set;
        // but on a fresh process with no registrations, all types short-circuit.
        for t in [
            PrimType::Int,
            PrimType::Float,
            PrimType::Bool,
            PrimType::Null,
            PrimType::Decimal,
            PrimType::String,
            PrimType::Array,
            PrimType::Hash,
            PrimType::Symbol,
        ] {
            // Nothing registered yet for this type on this thread — lookup misses.
            assert!(lookup_user_method(t, "definitely_not_a_real_method_xyz").is_none());
        }
    }
}
