//! Pure, engine-agnostic array transforms shared by the tree-walking
//! interpreter and the bytecode VM.
//!
//! The VM used to reimplement these inline, and the copies had drifted:
//! `flatten` in the VM flattened only a single level (and rejected a depth
//! argument) while the interpreter — the reference engine — flattens
//! recursively with an optional depth. Sharing one implementation keeps the
//! engines in lockstep so a fix lands in exactly one place.

use crate::interpreter::value::{HashPairs, SoliStr, Value};
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

/// A hashable stand-in for the `Value`s that can be hashed *without* changing
/// `Value`'s `PartialEq` semantics. `None` from [`fast_key`] means "this value's
/// equality can't be reproduced by hashing" and sends it to a linear fallback.
///
/// Getting this wrong is silent — `uniq` would keep or drop the wrong elements —
/// so each variant below records why it is shaped the way it is.
#[derive(PartialEq, Eq, Hash)]
enum FastKey {
    Null,
    Bool(bool),
    /// Integral numbers. `Int(1)` and `Float(1.0)` **must** collide: `Value`'s
    /// `PartialEq` has cross-type numeric arms, so a set that separated them
    /// would make `[1, 1.0].uniq()` return two elements instead of one.
    Num(i64),
    /// Finite non-integral floats, keyed by bit pattern. `±0.0` never reaches
    /// here — it is integral, so it normalises to `Num(0)` — which matters
    /// because `-0.0 == 0.0` while their bit patterns differ.
    FloatBits(u64),
    Str(SoliStr),
    Sym(SoliStr),
}

fn fast_key(v: &Value) -> Option<FastKey> {
    Some(match v {
        Value::Null => FastKey::Null,
        Value::Bool(b) => FastKey::Bool(*b),
        Value::Int(n) => FastKey::Num(*n),
        Value::Float(f) => {
            if f.is_nan() {
                // `NaN != NaN`, so every NaN must survive a dedup. Only the
                // linear fallback, which uses `PartialEq`, reproduces that.
                return None;
            }
            if f.fract() == 0.0 && *f >= i64::MIN as f64 && *f <= i64::MAX as f64 {
                FastKey::Num(*f as i64)
            } else {
                FastKey::FloatBits(f.to_bits())
            }
        }
        Value::String(s) => FastKey::Str(s.clone()),
        Value::Symbol(s) => FastKey::Sym(s.clone()),
        // Everything else falls back:
        //   * `Decimal` compares only against `Decimal` and its equality is not
        //     obviously bit-for-bit (`1.10` vs `1.1`),
        //   * `Array`/`Hash` compare structurally,
        //   * `Instance` compares by timestamp (DateTime) or pointer identity,
        //   * `Deferred` compares as its resolved value — callers resolve those
        //     up front via [`resolve_deferred`] so they never arrive here.
        _ => return None,
    })
}

/// Membership test for `Value`s that is O(1) for the common scalar cases while
/// staying exactly faithful to `Value`'s `PartialEq`.
///
/// The two buckets never need to be consulted together: `PartialEq`'s catch-all
/// arm is `false`, so no hashable value can ever equal an unhashable one.
#[derive(Default)]
struct ValueSet {
    hashed: HashSet<FastKey, ahash::RandomState>,
    /// Values whose equality cannot be hashed. Rare in practice, so the linear
    /// scan here keeps the common path exact without costing anything real.
    other: Vec<Value>,
}

impl ValueSet {
    fn with_capacity(n: usize) -> Self {
        Self {
            hashed: HashSet::with_capacity_and_hasher(n, ahash::RandomState::new()),
            other: Vec::new(),
        }
    }

    /// Insert, returning `true` when the value was not already present.
    fn insert(&mut self, v: &Value) -> bool {
        match fast_key(v) {
            Some(k) => self.hashed.insert(k),
            None => {
                if self.other.iter().any(|o| o == v) {
                    false
                } else {
                    self.other.push(v.clone());
                    true
                }
            }
        }
    }

    fn contains(&self, v: &Value) -> bool {
        match fast_key(v) {
            Some(k) => self.hashed.contains(&k),
            None => self.other.iter().any(|o| o == v),
        }
    }

    fn from_slice(items: &[Value]) -> Self {
        let mut set = Self::with_capacity(items.len());
        for item in items {
            set.insert(item);
        }
        set
    }
}

/// Resolve `grouped {}` deferreds up front so they can be keyed by the value
/// they stand for. Returns `None` — and does no work — when there are none,
/// which is the overwhelmingly common case.
fn resolve_deferred(items: &[Value]) -> Option<Vec<Value>> {
    if items.iter().any(Value::is_deferred) {
        Some(items.iter().map(Value::force_deferred).collect())
    } else {
        None
    }
}

/// Flatten `items` up to `max_depth` levels deep (`None` = fully recursive).
pub(crate) fn flatten_values(items: &[Value], max_depth: Option<usize>) -> Vec<Value> {
    fn recur(arr: &[Value], depth: usize, max: Option<usize>) -> Vec<Value> {
        if let Some(max) = max {
            if depth >= max {
                return arr.to_vec();
            }
        }
        let mut result = Vec::new();
        for item in arr {
            if let Value::Array(inner) = item {
                result.extend(recur(&inner.borrow(), depth + 1, max));
            } else {
                result.push(item.clone());
            }
        }
        result
    }
    recur(items, 0, max_depth)
}

/// Deduplicate `items`, preserving first-occurrence order (value equality).
///
/// Uses a hash set rather than scanning the output. The scan made this O(n·k)
/// in the number of unique elements — measurably quadratic in practice: on an
/// all-unique array `uniq()` cost 1 ms at n=1000 but **295 ms at n=16000**,
/// quadrupling on every doubling.
pub(crate) fn uniq_values(items: &[Value]) -> Vec<Value> {
    let resolved = resolve_deferred(items);
    let items = resolved.as_deref().unwrap_or(items);

    let mut seen = ValueSet::with_capacity(items.len());
    let mut result: Vec<Value> = Vec::with_capacity(items.len());
    for item in items {
        if seen.insert(item) {
            result.push(item.clone());
        }
    }
    result
}

/// `a | b` — every distinct value of `a`, then those of `b` not already present,
/// in first-occurrence order.
pub(crate) fn union_values(a: &[Value], b: &[Value]) -> Vec<Value> {
    let ra = resolve_deferred(a);
    let a = ra.as_deref().unwrap_or(a);
    let rb = resolve_deferred(b);
    let b = rb.as_deref().unwrap_or(b);

    let mut seen = ValueSet::with_capacity(a.len() + b.len());
    let mut result: Vec<Value> = Vec::with_capacity(a.len() + b.len());
    for item in a.iter().chain(b.iter()) {
        if seen.insert(item) {
            result.push(item.clone());
        }
    }
    result
}

/// `a & b` — the distinct values of `a` that also appear in `b`, in `a`'s order.
pub(crate) fn intersection_values(a: &[Value], b: &[Value]) -> Vec<Value> {
    let ra = resolve_deferred(a);
    let a = ra.as_deref().unwrap_or(a);
    let rb = resolve_deferred(b);
    let b = rb.as_deref().unwrap_or(b);

    let other = ValueSet::from_slice(b);
    let mut seen = ValueSet::with_capacity(a.len());
    let mut result: Vec<Value> = Vec::new();
    for item in a {
        if other.contains(item) && seen.insert(item) {
            result.push(item.clone());
        }
    }
    result
}

/// `a - b` — the distinct values of `a` absent from `b`, in `a`'s order.
pub(crate) fn difference_values(a: &[Value], b: &[Value]) -> Vec<Value> {
    let ra = resolve_deferred(a);
    let a = ra.as_deref().unwrap_or(a);
    let rb = resolve_deferred(b);
    let b = rb.as_deref().unwrap_or(b);

    let other = ValueSet::from_slice(b);
    let mut seen = ValueSet::with_capacity(a.len());
    let mut result: Vec<Value> = Vec::new();
    for item in a {
        if !other.contains(item) && seen.insert(item) {
            result.push(item.clone());
        }
    }
    result
}

/// Drop `null` elements from `items`.
pub(crate) fn compact_values(items: &[Value]) -> Vec<Value> {
    items
        .iter()
        .filter(|v| !matches!(v, Value::Null))
        .cloned()
        .collect()
}

// ---------------------------------------------------------------------------
// Field-keyed aggregates
//
// These exist to keep whole-collection work inside Rust. The alternative
// spelling — `orders.reduce(fn(a, o) { return a + o["amount"] }, 0)` — runs a
// Soli closure per element, and a closure call is ~45-235x the cost of the same
// operation done natively (measured: `sum` 0.005 ms native vs 1.128 ms via
// `reduce` over 20k rows). Every method here takes a *field name* rather than a
// callback precisely so the loop never re-enters the interpreter.
// ---------------------------------------------------------------------------

/// Read `field` from one record: a hash key, or an index into an array row.
/// Mirrors `pluck`'s accessor so the whole family behaves identically.
pub(crate) fn field_of(value: &Value, key: &Value) -> Value {
    use crate::interpreter::value::HashKey;
    match (value, key) {
        (Value::Hash(h), Value::String(s)) => {
            let hk = HashKey::String(s.clone());
            h.borrow().get(&hk).cloned().unwrap_or(Value::Null)
        }
        (Value::Array(a), Value::Int(i)) => {
            let arr = a.borrow();
            let idx = if *i < 0 {
                (arr.len() as i64 + *i) as usize
            } else {
                *i as usize
            };
            arr.get(idx).cloned().unwrap_or(Value::Null)
        }
        _ => Value::Null,
    }
}

/// A `Value` usable as a hash key. `None` for values a hash cannot key on
/// (arrays, hashes, instances) — callers group those under `Null` rather than
/// failing, matching how `field_of` reports a missing field.
fn as_hash_key(v: &Value) -> Option<crate::interpreter::value::HashKey> {
    use crate::interpreter::value::HashKey;
    Some(match v {
        Value::String(s) => HashKey::String(s.clone()),
        Value::Symbol(s) => HashKey::Symbol(s.clone()),
        Value::Int(n) => HashKey::Int(*n),
        Value::Bool(b) => HashKey::Bool(*b),
        Value::Decimal(d) => HashKey::Decimal(d.clone()),
        Value::Null => HashKey::Null,
        // Floats are deliberately keyed through their integer value when exact,
        // so `1.0` and `1` group together the way `Value`'s equality says they
        // should; anything else is not a sensible grouping key.
        Value::Float(f) if f.fract() == 0.0 && *f >= i64::MIN as f64 && *f <= i64::MAX as f64 => {
            HashKey::Int(*f as i64)
        }
        _ => return None,
    })
}

fn key_or_null(v: &Value) -> crate::interpreter::value::HashKey {
    as_hash_key(v).unwrap_or(crate::interpreter::value::HashKey::Null)
}

/// `sum(field)` — total a numeric field across records.
///
/// Ints stay integral (so money-as-cents does not silently become a float);
/// the result promotes to `Float` only once a float is seen. Non-numeric and
/// missing fields are skipped rather than erroring, matching `pluck`.
pub(crate) fn sum_by(items: &[Value], field: &Value) -> Value {
    let mut int_total: i64 = 0;
    let mut float_total = 0.0f64;
    let mut saw_float = false;
    for item in items {
        match field_of(item, field) {
            Value::Int(n) => int_total = int_total.wrapping_add(n),
            Value::Float(f) => {
                saw_float = true;
                float_total += f;
            }
            _ => {}
        }
    }
    if saw_float {
        Value::Float(float_total + int_total as f64)
    } else {
        Value::Int(int_total)
    }
}

/// `group_by(field)` — field value → array of the records carrying it,
/// preserving first-seen key order and within-group order.
pub(crate) fn group_by_field(items: &[Value], field: &Value) -> Value {
    let mut out = HashPairs::default();
    for item in items {
        let key = key_or_null(&field_of(item, field));
        match out.entry(key) {
            indexmap::map::Entry::Occupied(mut e) => {
                if let Value::Array(a) = e.get_mut() {
                    a.borrow_mut().push(item.clone());
                }
            }
            indexmap::map::Entry::Vacant(e) => {
                e.insert(Value::Array(Rc::new(RefCell::new(vec![item.clone()]))));
            }
        }
    }
    Value::Hash(Rc::new(RefCell::new(out)))
}

/// `index_by(field)` — field value → the record, for building lookup maps.
/// Last write wins on a duplicate key, as in Rails.
pub(crate) fn index_by(items: &[Value], field: &Value) -> Value {
    let mut out = HashPairs::default();
    for item in items {
        out.insert(key_or_null(&field_of(item, field)), item.clone());
    }
    Value::Hash(Rc::new(RefCell::new(out)))
}

/// `count_by(field)` — field value → how many records carry it.
pub(crate) fn count_by(items: &[Value], field: &Value) -> Value {
    let mut out = HashPairs::default();
    for item in items {
        let key = key_or_null(&field_of(item, field));
        match out.entry(key) {
            indexmap::map::Entry::Occupied(mut e) => {
                if let Value::Int(n) = e.get_mut() {
                    *n += 1;
                }
            }
            indexmap::map::Entry::Vacant(e) => {
                e.insert(Value::Int(1));
            }
        }
    }
    Value::Hash(Rc::new(RefCell::new(out)))
}

/// `tally()` — value → occurrence count, for a flat array.
pub(crate) fn tally(items: &[Value]) -> Value {
    let mut out = HashPairs::default();
    for item in items {
        let key = key_or_null(item);
        match out.entry(key) {
            indexmap::map::Entry::Occupied(mut e) => {
                if let Value::Int(n) = e.get_mut() {
                    *n += 1;
                }
            }
            indexmap::map::Entry::Vacant(e) => {
                e.insert(Value::Int(1));
            }
        }
    }
    Value::Hash(Rc::new(RefCell::new(out)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn arr(v: Vec<Value>) -> Value {
        Value::Array(Rc::new(RefCell::new(v)))
    }

    #[test]
    fn flatten_is_recursive_by_default() {
        // [[1, [2]], 3] -> [1, 2, 3] (this is where the VM previously diverged,
        // producing the shallow [1, [2], 3]).
        let input = vec![
            arr(vec![Value::Int(1), arr(vec![Value::Int(2)])]),
            Value::Int(3),
        ];
        assert_eq!(
            flatten_values(&input, None),
            vec![Value::Int(1), Value::Int(2), Value::Int(3)]
        );
    }

    #[test]
    fn flatten_respects_max_depth() {
        // depth 1: [[1, [2]]] -> [1, [2]]
        let input = vec![arr(vec![Value::Int(1), arr(vec![Value::Int(2)])])];
        assert_eq!(
            flatten_values(&input, Some(1)),
            vec![Value::Int(1), arr(vec![Value::Int(2)])]
        );
    }

    #[test]
    fn uniq_preserves_first_occurrence() {
        let input = vec![
            Value::Int(1),
            Value::Int(2),
            Value::Int(1),
            Value::Int(3),
            Value::Int(2),
        ];
        assert_eq!(
            uniq_values(&input),
            vec![Value::Int(1), Value::Int(2), Value::Int(3)]
        );
    }

    // --- field-keyed aggregates ------------------------------------------

    fn rec(pairs: &[(&str, Value)]) -> Value {
        let mut h = crate::interpreter::value::HashPairs::default();
        for (k, v) in pairs {
            h.insert(
                crate::interpreter::value::HashKey::String((*k).into()),
                v.clone(),
            );
        }
        Value::Hash(Rc::new(RefCell::new(h)))
    }
    fn key(s: &str) -> Value {
        Value::String(s.into())
    }

    /// Integers must stay integral — money is routinely held as cents, and a
    /// silent promotion to `Float` would introduce rounding into totals.
    #[test]
    fn sum_by_keeps_integers_integral() {
        let rows = vec![
            rec(&[("n", Value::Int(1250))]),
            rec(&[("n", Value::Int(900))]),
        ];
        assert_eq!(sum_by(&rows, &key("n")), Value::Int(2150));
    }

    #[test]
    fn sum_by_promotes_once_a_float_appears() {
        let rows = vec![
            rec(&[("n", Value::Float(1.5))]),
            rec(&[("n", Value::Int(2))]),
        ];
        assert_eq!(sum_by(&rows, &key("n")), Value::Float(3.5));
    }

    /// A missing or non-numeric field is skipped, matching `pluck`'s tolerance,
    /// rather than raising — totalling a sparse column is a normal thing to do.
    #[test]
    fn sum_by_skips_missing_and_non_numeric() {
        let rows = vec![
            rec(&[("n", Value::Int(5))]),
            rec(&[("other", Value::Int(99))]),
            rec(&[("n", Value::String("x".into()))]),
        ];
        assert_eq!(sum_by(&rows, &key("n")), Value::Int(5));
        assert_eq!(sum_by(&[], &key("n")), Value::Int(0));
    }

    #[test]
    fn group_by_preserves_key_and_member_order() {
        let rows = vec![
            rec(&[("t", key("b")), ("i", Value::Int(1))]),
            rec(&[("t", key("a")), ("i", Value::Int(2))]),
            rec(&[("t", key("b")), ("i", Value::Int(3))]),
        ];
        let Value::Hash(h) = group_by_field(&rows, &key("t")) else {
            panic!("expected a hash")
        };
        let h = h.borrow();
        let keys: Vec<_> = h.keys().cloned().collect();
        assert_eq!(
            keys,
            vec![
                crate::interpreter::value::HashKey::String("b".into()),
                crate::interpreter::value::HashKey::String("a".into())
            ],
            "keys follow first appearance"
        );
        let Some(Value::Array(bs)) = h.get(&crate::interpreter::value::HashKey::String("b".into()))
        else {
            panic!("missing group")
        };
        assert_eq!(bs.borrow().len(), 2);
    }

    /// Rails semantics: a duplicate key keeps the *last* record.
    #[test]
    fn index_by_last_write_wins() {
        let rows = vec![
            rec(&[("id", Value::Int(1)), ("v", key("first"))]),
            rec(&[("id", Value::Int(1)), ("v", key("second"))]),
        ];
        let Value::Hash(h) = index_by(&rows, &key("id")) else {
            panic!("expected a hash")
        };
        let h = h.borrow();
        assert_eq!(h.len(), 1);
        let Some(Value::Hash(row)) = h.get(&crate::interpreter::value::HashKey::Int(1)) else {
            panic!("missing row")
        };
        assert_eq!(
            row.borrow()
                .get(&crate::interpreter::value::HashKey::String("v".into())),
            Some(&key("second"))
        );
    }

    #[test]
    fn count_by_and_tally_agree() {
        let rows = vec![
            rec(&[("s", key("open"))]),
            rec(&[("s", key("shut"))]),
            rec(&[("s", key("open"))]),
        ];
        let Value::Hash(c) = count_by(&rows, &key("s")) else {
            panic!()
        };
        assert_eq!(
            c.borrow()
                .get(&crate::interpreter::value::HashKey::String("open".into())),
            Some(&Value::Int(2))
        );
        let Value::Hash(t) = tally(&[Value::Int(1), Value::Int(2), Value::Int(2)]) else {
            panic!()
        };
        assert_eq!(
            t.borrow().get(&crate::interpreter::value::HashKey::Int(2)),
            Some(&Value::Int(2))
        );
    }

    /// A record missing the field groups under `null` rather than vanishing, so
    /// counts still add up to the input length.
    #[test]
    fn missing_field_groups_under_null() {
        let rows = vec![rec(&[("t", key("a"))]), rec(&[("other", key("z"))])];
        let Value::Hash(h) = count_by(&rows, &key("t")) else {
            panic!()
        };
        let h = h.borrow();
        assert_eq!(
            h.get(&crate::interpreter::value::HashKey::Null),
            Some(&Value::Int(1))
        );
        let total: i64 = h
            .values()
            .map(|v| if let Value::Int(n) = v { *n } else { 0 })
            .sum();
        assert_eq!(total, 2, "every record is counted somewhere");
    }

    #[test]
    fn compact_drops_nulls() {
        let input = vec![Value::Int(1), Value::Null, Value::Int(2), Value::Null];
        assert_eq!(compact_values(&input), vec![Value::Int(1), Value::Int(2)]);
    }

    // --- equality edge cases the hash-set dedup must reproduce exactly -------
    //
    // These are the cases where swapping a linear `contains` scan for a hash
    // set silently changes behaviour. Each one failed at least one candidate
    // key design while this was written.

    /// `Value`'s `PartialEq` has cross-type numeric arms: `1 == 1.0`. So the
    /// two must share a key, or `uniq` keeps both.
    #[test]
    fn uniq_treats_int_and_equal_float_as_one() {
        let input = vec![Value::Int(1), Value::Float(1.0), Value::Int(1)];
        assert_eq!(uniq_values(&input), vec![Value::Int(1)]);
        // ...and the reverse order keeps the float, since first occurrence wins.
        let input = vec![Value::Float(2.0), Value::Int(2)];
        assert_eq!(uniq_values(&input), vec![Value::Float(2.0)]);
    }

    /// `-0.0 == 0.0` is true, but their bit patterns differ — so a bitwise key
    /// would wrongly keep both.
    #[test]
    fn uniq_collapses_negative_zero() {
        let input = vec![Value::Float(0.0), Value::Float(-0.0), Value::Int(0)];
        assert_eq!(uniq_values(&input), vec![Value::Float(0.0)]);
    }

    /// `NaN != NaN`, so no NaN is ever a duplicate — not even of itself.
    #[test]
    fn uniq_keeps_every_nan() {
        let input = vec![Value::Float(f64::NAN), Value::Float(f64::NAN)];
        assert_eq!(uniq_values(&input).len(), 2);
    }

    /// Non-integral floats still dedup normally.
    #[test]
    fn uniq_dedups_fractional_floats() {
        let input = vec![Value::Float(1.5), Value::Float(1.5), Value::Float(2.5)];
        assert_eq!(
            uniq_values(&input),
            vec![Value::Float(1.5), Value::Float(2.5)]
        );
    }

    /// Arrays compare structurally and cannot be hashed — they must still
    /// dedup, via the linear fallback.
    #[test]
    fn uniq_dedups_structurally_equal_arrays() {
        let input = vec![
            arr(vec![Value::Int(1), Value::Int(2)]),
            arr(vec![Value::Int(1), Value::Int(2)]),
            arr(vec![Value::Int(3)]),
        ];
        let out = uniq_values(&input);
        assert_eq!(out.len(), 2, "structurally equal arrays are duplicates");
    }

    /// A hashable value must never be confused with an unhashable one — the
    /// two live in different buckets, and `PartialEq` says they are unequal.
    #[test]
    fn uniq_keeps_scalar_and_array_apart() {
        let input = vec![Value::Int(1), arr(vec![Value::Int(1)])];
        assert_eq!(uniq_values(&input).len(), 2);
    }

    #[test]
    fn union_is_ordered_and_deduped() {
        let a = vec![Value::Int(1), Value::Int(2), Value::Int(2)];
        let b = vec![Value::Int(2), Value::Int(3)];
        assert_eq!(
            union_values(&a, &b),
            vec![Value::Int(1), Value::Int(2), Value::Int(3)]
        );
    }

    #[test]
    fn intersection_keeps_left_order_and_dedups() {
        let a = vec![Value::Int(3), Value::Int(1), Value::Int(3), Value::Int(9)];
        let b = vec![Value::Int(1), Value::Int(3)];
        assert_eq!(
            intersection_values(&a, &b),
            vec![Value::Int(3), Value::Int(1)]
        );
    }

    #[test]
    fn difference_removes_all_of_b_and_dedups() {
        let a = vec![Value::Int(1), Value::Int(2), Value::Int(2), Value::Int(3)];
        let b = vec![Value::Int(2)];
        assert_eq!(
            difference_values(&a, &b),
            vec![Value::Int(1), Value::Int(3)]
        );
    }

    /// Set ops honour the same cross-type numeric equality as `uniq`.
    #[test]
    fn set_ops_match_int_against_float() {
        let a = vec![Value::Int(1), Value::Int(2)];
        let b = vec![Value::Float(1.0)];
        assert_eq!(intersection_values(&a, &b), vec![Value::Int(1)]);
        assert_eq!(difference_values(&a, &b), vec![Value::Int(2)]);
    }
}
