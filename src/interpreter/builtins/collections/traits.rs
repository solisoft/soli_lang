//! Shared collection traits.

/// Trait for collection-like types that can be iterated.
pub trait Collection {
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool;
}

/// Trait for types that can be indexed.
pub trait Indexable {
    type Item;
    fn get(&self, index: i64) -> Option<Self::Item>;
}

/// Trait for iterable types.
pub trait Iterable {
    type Item;
    fn iter(&self) -> Box<dyn Iterator<Item = Self::Item>>;
}
