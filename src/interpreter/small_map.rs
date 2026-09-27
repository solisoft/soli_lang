//! `SmallMap`: the storage behind every Soli hash.
//!
//! Most hashes a web app builds are small — request params, a JSON row, an
//! options hash, `{"status": 422}` — and `IndexMap` made each of them pay for
//! a hash table: an index allocation on top of the entries, and a hash of
//! every key on every insert and lookup. Ruby keeps hashes of up to eight
//! entries as a plain array searched linearly (`ar_table`) for the same
//! reason. `SmallMap` does the same: up to [`SMALL_MAX`] entries live in a
//! `Vec` in insertion order and are found by comparing keys, with no hashing;
//! the ninth insert moves them into the `IndexMap` every hash used before.
//!
//! The API is the subset of `IndexMap`'s that the interpreter uses, with the
//! same meaning — insertion order, `shift_remove` / `swap_remove`, positions,
//! `Equivalent` lookups with borrowed keys (`StrKey`, `SymKey`), and equality
//! that ignores order — so the representation is invisible to callers.

use std::hash::Hash;

use indexmap::{Equivalent, IndexMap};

use super::value::{HashKey, Value};

type Hasher = ahash::RandomState;

/// Entries kept in the linear representation. Ruby's `ar_table` limit.
pub const SMALL_MAX: usize = 8;

#[derive(Clone)]
enum Repr {
    Small(Vec<(HashKey, Value)>),
    Large(IndexMap<HashKey, Value, Hasher>),
}

/// An insertion-ordered map from `HashKey` to `Value`.
#[derive(Clone)]
pub struct SmallMap {
    repr: Repr,
}

impl Default for SmallMap {
    #[inline]
    fn default() -> Self {
        SmallMap {
            repr: Repr::Small(Vec::new()),
        }
    }
}

impl std::fmt::Debug for SmallMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

/// Equality ignores order, as `IndexMap`'s does.
impl PartialEq for SmallMap {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len()
            && self
                .iter()
                .all(|(key, value)| other.get(key).is_some_and(|v| v == value))
    }
}

/// Position of `key` among the linear entries.
#[inline(always)]
fn small_position<Q>(entries: &[(HashKey, Value)], key: &Q) -> Option<usize>
where
    Q: ?Sized + Equivalent<HashKey>,
{
    entries.iter().position(|(k, _)| key.equivalent(k))
}

impl SmallMap {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        if capacity <= SMALL_MAX {
            SmallMap {
                repr: Repr::Small(Vec::with_capacity(capacity)),
            }
        } else {
            SmallMap {
                repr: Repr::Large(IndexMap::with_capacity_and_hasher(
                    capacity,
                    Hasher::default(),
                )),
            }
        }
    }

    /// `IndexMap`-compatible constructor; the hasher only matters once the
    /// map outgrows the linear representation, and every caller passes a
    /// default one.
    #[inline]
    pub fn with_capacity_and_hasher(capacity: usize, _hasher: Hasher) -> Self {
        Self::with_capacity(capacity)
    }

    #[inline]
    pub fn with_hasher(_hasher: Hasher) -> Self {
        Self::default()
    }

    /// Build a map from `keys` and their values, in order, where the caller
    /// guarantees the keys are distinct — a hash literal whose keys the
    /// compiler checked. Up to [`SMALL_MAX`] entries this is a single
    /// allocation and no key comparison at all.
    pub fn from_distinct(keys: &[HashKey], values: impl Iterator<Item = Value>) -> Self {
        debug_assert!(
            keys.iter().collect::<std::collections::HashSet<_>>().len() == keys.len(),
            "from_distinct called with duplicate keys"
        );
        if keys.len() <= SMALL_MAX {
            SmallMap {
                repr: Repr::Small(keys.iter().cloned().zip(values).collect()),
            }
        } else {
            let mut map = IndexMap::with_capacity_and_hasher(keys.len(), Hasher::default());
            for (key, value) in keys.iter().cloned().zip(values) {
                map.insert(key, value);
            }
            SmallMap {
                repr: Repr::Large(map),
            }
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        match &self.repr {
            Repr::Small(entries) => entries.len(),
            Repr::Large(map) => map.len(),
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn clear(&mut self) {
        self.repr = Repr::Small(Vec::new());
    }

    pub fn reserve(&mut self, additional: usize) {
        let wanted = self.len() + additional;
        match &mut self.repr {
            Repr::Small(entries) if wanted <= SMALL_MAX => entries.reserve(additional),
            Repr::Small(_) => self.promote(wanted),
            Repr::Large(map) => map.reserve(additional),
        }
    }

    /// Move the linear entries into an `IndexMap` sized for `capacity`.
    #[cold]
    fn promote(&mut self, capacity: usize) {
        if let Repr::Small(entries) = &mut self.repr {
            let mut map = IndexMap::with_capacity_and_hasher(capacity, Hasher::default());
            for (key, value) in entries.drain(..) {
                map.insert(key, value);
            }
            self.repr = Repr::Large(map);
        }
    }

    #[inline]
    pub fn get<Q>(&self, key: &Q) -> Option<&Value>
    where
        Q: ?Sized + Hash + Equivalent<HashKey>,
    {
        match &self.repr {
            Repr::Small(entries) => small_position(entries, key).map(|i| &entries[i].1),
            Repr::Large(map) => map.get(key),
        }
    }

    #[inline]
    pub fn get_mut<Q>(&mut self, key: &Q) -> Option<&mut Value>
    where
        Q: ?Sized + Hash + Equivalent<HashKey>,
    {
        match &mut self.repr {
            Repr::Small(entries) => small_position(entries, key).map(move |i| &mut entries[i].1),
            Repr::Large(map) => map.get_mut(key),
        }
    }

    #[inline]
    pub fn get_full<Q>(&self, key: &Q) -> Option<(usize, &HashKey, &Value)>
    where
        Q: ?Sized + Hash + Equivalent<HashKey>,
    {
        match &self.repr {
            Repr::Small(entries) => small_position(entries, key).map(|i| {
                let (k, v) = &entries[i];
                (i, k, v)
            }),
            Repr::Large(map) => map.get_full(key),
        }
    }

    #[inline]
    pub fn get_full_mut<Q>(&mut self, key: &Q) -> Option<(usize, &HashKey, &mut Value)>
    where
        Q: ?Sized + Hash + Equivalent<HashKey>,
    {
        match &mut self.repr {
            Repr::Small(entries) => small_position(entries, key).map(move |i| {
                let (k, v) = &mut entries[i];
                (i, &*k, v)
            }),
            Repr::Large(map) => map.get_full_mut(key),
        }
    }

    #[inline]
    pub fn get_index_of<Q>(&self, key: &Q) -> Option<usize>
    where
        Q: ?Sized + Hash + Equivalent<HashKey>,
    {
        match &self.repr {
            Repr::Small(entries) => small_position(entries, key),
            Repr::Large(map) => map.get_index_of(key),
        }
    }

    #[inline]
    pub fn contains_key<Q>(&self, key: &Q) -> bool
    where
        Q: ?Sized + Hash + Equivalent<HashKey>,
    {
        self.get_index_of(key).is_some()
    }

    #[inline]
    pub fn get_index(&self, index: usize) -> Option<(&HashKey, &Value)> {
        match &self.repr {
            Repr::Small(entries) => entries.get(index).map(|(k, v)| (k, v)),
            Repr::Large(map) => map.get_index(index),
        }
    }

    #[inline]
    pub fn get_index_mut(&mut self, index: usize) -> Option<(&HashKey, &mut Value)> {
        match &mut self.repr {
            Repr::Small(entries) => entries.get_mut(index).map(|(k, v)| (&*k, v)),
            Repr::Large(map) => map.get_index_mut(index),
        }
    }

    #[inline]
    pub fn first(&self) -> Option<(&HashKey, &Value)> {
        self.get_index(0)
    }

    #[inline]
    pub fn last(&self) -> Option<(&HashKey, &Value)> {
        self.len().checked_sub(1).and_then(|i| self.get_index(i))
    }

    /// Insert or overwrite; an overwrite keeps the key's position and returns
    /// the old value, as `IndexMap::insert` does.
    #[inline]
    pub fn insert(&mut self, key: HashKey, value: Value) -> Option<Value> {
        self.insert_full(key, value).1
    }

    pub fn insert_full(&mut self, key: HashKey, value: Value) -> (usize, Option<Value>) {
        match &mut self.repr {
            Repr::Small(entries) => {
                if let Some(i) = small_position(entries, &key) {
                    let old = std::mem::replace(&mut entries[i].1, value);
                    return (i, Some(old));
                }
                if entries.len() < SMALL_MAX {
                    entries.push((key, value));
                    return (entries.len() - 1, None);
                }
                let len = entries.len();
                self.promote(len + 1);
                self.insert_full(key, value)
            }
            Repr::Large(map) => map.insert_full(key, value),
        }
    }

    pub fn shift_remove<Q>(&mut self, key: &Q) -> Option<Value>
    where
        Q: ?Sized + Hash + Equivalent<HashKey>,
    {
        match &mut self.repr {
            Repr::Small(entries) => small_position(entries, key).map(|i| entries.remove(i).1),
            Repr::Large(map) => map.shift_remove(key),
        }
    }

    pub fn swap_remove<Q>(&mut self, key: &Q) -> Option<Value>
    where
        Q: ?Sized + Hash + Equivalent<HashKey>,
    {
        match &mut self.repr {
            Repr::Small(entries) => small_position(entries, key).map(|i| entries.swap_remove(i).1),
            Repr::Large(map) => map.swap_remove(key),
        }
    }

    pub fn shift_remove_entry<Q>(&mut self, key: &Q) -> Option<(HashKey, Value)>
    where
        Q: ?Sized + Hash + Equivalent<HashKey>,
    {
        match &mut self.repr {
            Repr::Small(entries) => small_position(entries, key).map(|i| entries.remove(i)),
            Repr::Large(map) => map.shift_remove_entry(key),
        }
    }

    pub fn shift_remove_index(&mut self, index: usize) -> Option<(HashKey, Value)> {
        match &mut self.repr {
            Repr::Small(entries) => (index < entries.len()).then(|| entries.remove(index)),
            Repr::Large(map) => map.shift_remove_index(index),
        }
    }

    pub fn swap_remove_index(&mut self, index: usize) -> Option<(HashKey, Value)> {
        match &mut self.repr {
            Repr::Small(entries) => (index < entries.len()).then(|| entries.swap_remove(index)),
            Repr::Large(map) => map.swap_remove_index(index),
        }
    }

    pub fn pop(&mut self) -> Option<(HashKey, Value)> {
        match &mut self.repr {
            Repr::Small(entries) => entries.pop(),
            Repr::Large(map) => map.pop(),
        }
    }

    pub fn retain<F>(&mut self, mut keep: F)
    where
        F: FnMut(&HashKey, &mut Value) -> bool,
    {
        match &mut self.repr {
            Repr::Small(entries) => entries.retain_mut(|(k, v)| keep(k, v)),
            Repr::Large(map) => map.retain(keep),
        }
    }

    pub fn sort_by<F>(&mut self, mut compare: F)
    where
        F: FnMut(&HashKey, &Value, &HashKey, &Value) -> std::cmp::Ordering,
    {
        match &mut self.repr {
            Repr::Small(entries) => entries.sort_by(|(k1, v1), (k2, v2)| compare(k1, v1, k2, v2)),
            Repr::Large(map) => map.sort_by(compare),
        }
    }

    pub fn reverse(&mut self) {
        match &mut self.repr {
            Repr::Small(entries) => entries.reverse(),
            Repr::Large(map) => map.reverse(),
        }
    }

    pub fn truncate(&mut self, len: usize) {
        match &mut self.repr {
            Repr::Small(entries) => entries.truncate(len),
            Repr::Large(map) => map.truncate(len),
        }
    }

    pub fn entry(&mut self, key: HashKey) -> Entry<'_> {
        match self.get_index_of(&key) {
            Some(index) => Entry::Occupied(OccupiedEntry { map: self, index }),
            None => Entry::Vacant(VacantEntry { map: self, key }),
        }
    }

    #[inline]
    pub fn iter(&self) -> Iter<'_> {
        match &self.repr {
            Repr::Small(entries) => Iter::Small(entries.iter()),
            Repr::Large(map) => Iter::Large(map.iter()),
        }
    }

    #[inline]
    pub fn iter_mut(&mut self) -> IterMut<'_> {
        match &mut self.repr {
            Repr::Small(entries) => IterMut::Small(entries.iter_mut()),
            Repr::Large(map) => IterMut::Large(map.iter_mut()),
        }
    }

    #[inline]
    pub fn keys(&self) -> impl DoubleEndedIterator<Item = &HashKey> + ExactSizeIterator {
        self.iter().map(|(k, _)| k)
    }

    #[inline]
    pub fn values(&self) -> impl DoubleEndedIterator<Item = &Value> + ExactSizeIterator {
        self.iter().map(|(_, v)| v)
    }

    #[inline]
    pub fn values_mut(
        &mut self,
    ) -> impl DoubleEndedIterator<Item = &mut Value> + ExactSizeIterator {
        self.iter_mut().map(|(_, v)| v)
    }

    #[inline]
    pub fn into_keys(self) -> impl DoubleEndedIterator<Item = HashKey> + ExactSizeIterator {
        self.into_iter().map(|(k, _)| k)
    }

    #[inline]
    pub fn into_values(self) -> impl DoubleEndedIterator<Item = Value> + ExactSizeIterator {
        self.into_iter().map(|(_, v)| v)
    }

    pub fn drain(&mut self) -> IntoIter {
        std::mem::take(self).into_iter()
    }
}

/// `map[&key]`, panicking on a missing key as `IndexMap`'s does.
impl<Q> std::ops::Index<&Q> for SmallMap
where
    Q: ?Sized + Hash + Equivalent<HashKey>,
{
    type Output = Value;
    fn index(&self, key: &Q) -> &Value {
        self.get(key).expect("SmallMap: key not found")
    }
}

impl Extend<(HashKey, Value)> for SmallMap {
    fn extend<I: IntoIterator<Item = (HashKey, Value)>>(&mut self, iter: I) {
        for (key, value) in iter {
            self.insert(key, value);
        }
    }
}

impl FromIterator<(HashKey, Value)> for SmallMap {
    fn from_iter<I: IntoIterator<Item = (HashKey, Value)>>(iter: I) -> Self {
        let iter = iter.into_iter();
        let mut map = SmallMap::with_capacity(iter.size_hint().0);
        map.extend(iter);
        map
    }
}

// ---------------------------------------------------------------------------
// Iterators: one enum per kind, over either representation.
// ---------------------------------------------------------------------------

pub enum Iter<'a> {
    Small(std::slice::Iter<'a, (HashKey, Value)>),
    Large(indexmap::map::Iter<'a, HashKey, Value>),
}

impl<'a> Iterator for Iter<'a> {
    type Item = (&'a HashKey, &'a Value);
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Iter::Small(it) => it.next().map(|(k, v)| (k, v)),
            Iter::Large(it) => it.next(),
        }
    }
    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Iter::Small(it) => it.size_hint(),
            Iter::Large(it) => it.size_hint(),
        }
    }
}

impl DoubleEndedIterator for Iter<'_> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        match self {
            Iter::Small(it) => it.next_back().map(|(k, v)| (k, v)),
            Iter::Large(it) => it.next_back(),
        }
    }
}

impl ExactSizeIterator for Iter<'_> {}

impl Clone for Iter<'_> {
    fn clone(&self) -> Self {
        match self {
            Iter::Small(it) => Iter::Small(it.clone()),
            Iter::Large(it) => Iter::Large(it.clone()),
        }
    }
}

pub enum IterMut<'a> {
    Small(std::slice::IterMut<'a, (HashKey, Value)>),
    Large(indexmap::map::IterMut<'a, HashKey, Value>),
}

impl<'a> Iterator for IterMut<'a> {
    type Item = (&'a HashKey, &'a mut Value);
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            IterMut::Small(it) => it.next().map(|(k, v)| (&*k, v)),
            IterMut::Large(it) => it.next(),
        }
    }
    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            IterMut::Small(it) => it.size_hint(),
            IterMut::Large(it) => it.size_hint(),
        }
    }
}

impl DoubleEndedIterator for IterMut<'_> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        match self {
            IterMut::Small(it) => it.next_back().map(|(k, v)| (&*k, v)),
            IterMut::Large(it) => it.next_back(),
        }
    }
}

impl ExactSizeIterator for IterMut<'_> {}

pub enum IntoIter {
    Small(std::vec::IntoIter<(HashKey, Value)>),
    Large(indexmap::map::IntoIter<HashKey, Value>),
}

impl Iterator for IntoIter {
    type Item = (HashKey, Value);
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            IntoIter::Small(it) => it.next(),
            IntoIter::Large(it) => it.next(),
        }
    }
    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            IntoIter::Small(it) => it.size_hint(),
            IntoIter::Large(it) => it.size_hint(),
        }
    }
}

impl DoubleEndedIterator for IntoIter {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        match self {
            IntoIter::Small(it) => it.next_back(),
            IntoIter::Large(it) => it.next_back(),
        }
    }
}

impl ExactSizeIterator for IntoIter {}

impl IntoIterator for SmallMap {
    type Item = (HashKey, Value);
    type IntoIter = IntoIter;
    fn into_iter(self) -> IntoIter {
        match self.repr {
            Repr::Small(entries) => IntoIter::Small(entries.into_iter()),
            Repr::Large(map) => IntoIter::Large(map.into_iter()),
        }
    }
}

impl<'a> IntoIterator for &'a SmallMap {
    type Item = (&'a HashKey, &'a Value);
    type IntoIter = Iter<'a>;
    fn into_iter(self) -> Iter<'a> {
        self.iter()
    }
}

impl<'a> IntoIterator for &'a mut SmallMap {
    type Item = (&'a HashKey, &'a mut Value);
    type IntoIter = IterMut<'a>;
    fn into_iter(self) -> IterMut<'a> {
        self.iter_mut()
    }
}

// ---------------------------------------------------------------------------
// Entry API (the part the interpreter uses).
// ---------------------------------------------------------------------------

pub enum Entry<'a> {
    Occupied(OccupiedEntry<'a>),
    Vacant(VacantEntry<'a>),
}

pub struct OccupiedEntry<'a> {
    map: &'a mut SmallMap,
    index: usize,
}

pub struct VacantEntry<'a> {
    map: &'a mut SmallMap,
    key: HashKey,
}

impl<'a> Entry<'a> {
    pub fn or_insert(self, default: Value) -> &'a mut Value {
        match self {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(default),
        }
    }

    pub fn or_insert_with<F: FnOnce() -> Value>(self, default: F) -> &'a mut Value {
        match self {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(default()),
        }
    }

    pub fn key(&self) -> &HashKey {
        match self {
            Entry::Occupied(entry) => entry.key(),
            Entry::Vacant(entry) => &entry.key,
        }
    }
}

impl<'a> OccupiedEntry<'a> {
    pub fn key(&self) -> &HashKey {
        self.map.get_index(self.index).unwrap().0
    }

    pub fn get(&self) -> &Value {
        self.map.get_index(self.index).unwrap().1
    }

    pub fn get_mut(&mut self) -> &mut Value {
        self.map.get_index_mut(self.index).unwrap().1
    }

    pub fn into_mut(self) -> &'a mut Value {
        self.map.get_index_mut(self.index).unwrap().1
    }

    pub fn insert(&mut self, value: Value) -> Value {
        std::mem::replace(self.get_mut(), value)
    }

    pub fn index(&self) -> usize {
        self.index
    }
}

impl<'a> VacantEntry<'a> {
    pub fn key(&self) -> &HashKey {
        &self.key
    }

    pub fn insert(self, value: Value) -> &'a mut Value {
        let (index, _) = self.map.insert_full(self.key, value);
        self.map.get_index_mut(index).unwrap().1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(s: &str) -> HashKey {
        HashKey::String(s.into())
    }

    fn filled(n: usize) -> SmallMap {
        let mut map = SmallMap::default();
        for i in 0..n {
            map.insert(key(&format!("k{i}")), Value::Int(i as i64));
        }
        map
    }

    /// Order, positions and removals behave the same on both sides of the
    /// promotion to `IndexMap`.
    #[test]
    fn order_and_removal_match_across_the_promotion() {
        for n in [3, SMALL_MAX, SMALL_MAX + 1, 20] {
            let mut map = filled(n);
            assert_eq!(map.len(), n);
            assert_eq!(map.get_index_of(&key("k1")), Some(1));
            assert_eq!(map.insert(key("k1"), Value::Int(99)), Some(Value::Int(1)));
            assert_eq!(
                map.get_index_of(&key("k1")),
                Some(1),
                "overwrite keeps position"
            );
            assert_eq!(map.shift_remove(&key("k0")), Some(Value::Int(0)));
            let keys: Vec<_> = map.keys().cloned().collect();
            let expected: Vec<_> = (1..n).map(|i| key(&format!("k{i}"))).collect();
            assert_eq!(keys, expected, "n = {n}");
            assert_eq!(map.get(&key("k1")), Some(&Value::Int(99)));
            assert!(map.get(&key("missing")).is_none());
        }
    }

    /// Equality ignores order, like `IndexMap`'s.
    #[test]
    fn equality_ignores_order() {
        let mut a = SmallMap::default();
        a.insert(key("x"), Value::Int(1));
        a.insert(key("y"), Value::Int(2));
        let mut b = SmallMap::default();
        b.insert(key("y"), Value::Int(2));
        b.insert(key("x"), Value::Int(1));
        assert_eq!(a, b);
        assert_eq!(filled(12), filled(12));
        assert_ne!(filled(3), filled(4));
    }

    /// Borrowed-key lookups (`StrKey`) work in the linear representation.
    #[test]
    fn borrowed_keys_find_linear_entries() {
        let map = filled(4);
        assert_eq!(
            map.get(&crate::interpreter::value::StrKey("k2")),
            Some(&Value::Int(2))
        );
    }
}
