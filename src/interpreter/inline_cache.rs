//! Inline Cache (IC) system for optimizing property access and method calls.
//!
//! Implements Polymorphic Inline Caching (PIC) similar to V8's IC.
//! Each cache entry stores:
//! - SymbolId of the accessed property/method
//! - Hidden class ID for type feedback
//! - Cached result (property offset or method pointer)
//!
//! Cache structure:
//! - Monomorphic: 1 entry for single type seen
//! - Megamorphic: 2+ entries, falls back to HashMap lookup

use crate::interpreter::SymbolId;
use lazy_static::lazy_static;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::RwLock;

lazy_static! {
    pub static ref INLINE_CACHE: InlineCacheRegistry = InlineCacheRegistry::new();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HiddenClassId(pub u32);

#[derive(Debug, Clone)]
pub struct MethodCacheEntry {
    pub symbol_id: SymbolId,
    pub hidden_class_id: HiddenClassId,
    pub method_class: String,
    pub method_offset: usize,
}

#[derive(Debug, Clone)]
pub struct PropertyInlineCache {
    entries: HashMap<(SymbolId, HiddenClassId), usize>,
    max_entries: usize,
}

impl Default for PropertyInlineCache {
    fn default() -> Self {
        Self::new()
    }
}

impl PropertyInlineCache {
    pub fn new() -> Self {
        Self {
            entries: HashMap::with_capacity(4),
            max_entries: 4,
        }
    }

    pub fn lookup(&self, symbol_id: SymbolId, hidden_class_id: HiddenClassId) -> Option<usize> {
        self.entries.get(&(symbol_id, hidden_class_id)).copied()
    }

    pub fn insert(
        &mut self,
        symbol_id: SymbolId,
        hidden_class_id: HiddenClassId,
        offset: usize,
    ) -> bool {
        let key = (symbol_id, hidden_class_id);
        if self.entries.len() >= self.max_entries && !self.entries.contains_key(&key) {
            return false;
        }
        self.entries.insert(key, offset);
        true
    }

    pub fn is_monomorphic(&self) -> bool {
        self.entries.len() == 1
    }

    pub fn is_megamorphic(&self) -> bool {
        self.entries.len() >= self.max_entries
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[derive(Debug, Clone)]
pub struct MethodInlineCache {
    entries: Vec<MethodCacheEntry>,
    max_entries: usize,
}

impl Default for MethodInlineCache {
    fn default() -> Self {
        Self::new()
    }
}

impl MethodInlineCache {
    pub fn new() -> Self {
        Self {
            entries: Vec::with_capacity(4),
            max_entries: 4,
        }
    }

    pub fn lookup(
        &self,
        symbol_id: SymbolId,
        hidden_class_id: HiddenClassId,
    ) -> Option<&MethodCacheEntry> {
        self.entries
            .iter()
            .find(|entry| entry.symbol_id == symbol_id && entry.hidden_class_id == hidden_class_id)
    }

    pub fn insert(
        &mut self,
        symbol_id: SymbolId,
        hidden_class_id: HiddenClassId,
        method_class: String,
        method_offset: usize,
    ) -> bool {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|e| e.symbol_id == symbol_id && e.hidden_class_id == hidden_class_id)
        {
            entry.method_class = method_class;
            entry.method_offset = method_offset;
            return true;
        }
        if self.entries.len() < self.max_entries {
            self.entries.push(MethodCacheEntry {
                symbol_id,
                hidden_class_id,
                method_class,
                method_offset,
            });
            true
        } else {
            false
        }
    }

    pub fn is_megamorphic(&self) -> bool {
        self.entries.len() >= self.max_entries
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[derive(Debug)]
pub struct InlineCacheRegistry {
    property_caches: RwLock<HashMap<usize, PropertyInlineCache>>,
    method_caches: RwLock<HashMap<usize, MethodInlineCache>>,
    next_hidden_class_id: AtomicU32,
}

impl InlineCacheRegistry {
    fn new() -> Self {
        Self {
            property_caches: RwLock::new(HashMap::new()),
            method_caches: RwLock::new(HashMap::new()),
            next_hidden_class_id: AtomicU32::new(0),
        }
    }

    pub fn get_property_cache(&self, ip: usize) -> PropertyInlineCache {
        let caches = self.property_caches.read().unwrap();
        caches.get(&ip).cloned().unwrap_or_else(|| {
            drop(caches);
            let mut write_caches = self.property_caches.write().unwrap();
            write_caches.entry(ip).or_default().clone()
        })
    }

    pub fn get_method_cache(&self, ip: usize) -> MethodInlineCache {
        let caches = self.method_caches.read().unwrap();
        caches.get(&ip).cloned().unwrap_or_else(|| {
            drop(caches);
            let mut write_caches = self.method_caches.write().unwrap();
            write_caches.entry(ip).or_default().clone()
        })
    }

    pub fn new_hidden_class_id(&self) -> HiddenClassId {
        let id = self.next_hidden_class_id.fetch_add(1, Ordering::Relaxed);
        HiddenClassId(id)
    }

    pub fn clear_all(&self) {
        let mut property_caches = self.property_caches.write().unwrap();
        let mut method_caches = self.method_caches.write().unwrap();
        for cache in property_caches.values_mut() {
            cache.clear();
        }
        for cache in method_caches.values_mut() {
            cache.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::get_symbol;

    #[test]
    fn test_property_cache_monomorphic() {
        let mut cache = PropertyInlineCache::new();
        let symbol_id = get_symbol("test");
        let hc_id = INLINE_CACHE.new_hidden_class_id();

        assert!(cache.insert(symbol_id, hc_id, 42));
        assert!(!cache.is_megamorphic());
        assert_eq!(cache.lookup(symbol_id, hc_id), Some(42));
    }

    #[test]
    fn test_property_cache_megamorphic() {
        let mut cache = PropertyInlineCache::new();
        let symbol_id = get_symbol("test");

        for i in 0..4 {
            let hc_id = INLINE_CACHE.new_hidden_class_id();
            assert!(cache.insert(symbol_id, hc_id, i));
        }

        assert!(cache.is_megamorphic());

        let new_hc_id = INLINE_CACHE.new_hidden_class_id();
        assert!(!cache.insert(symbol_id, new_hc_id, 99));
    }

    #[test]
    fn test_hidden_class_ids() {
        let id1 = INLINE_CACHE.new_hidden_class_id();
        let id2 = INLINE_CACHE.new_hidden_class_id();

        assert_ne!(id1, id2);
    }
}
