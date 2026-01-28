//! Symbol table for string interning - enables O(1) string comparison.
//!
//! All strings are interned at compile time and assigned a unique SymbolId.
//! This replaces HashMap<String, T> with HashMap<SymbolId, T> for better performance.

use lazy_static::lazy_static;
use std::collections::HashMap;
use std::sync::RwLock;

lazy_static! {
    /// Global symbol table - all interned strings are stored here.
    /// SymbolIds are indices into this vector.
    pub static ref SYMBOL_TABLE: RwLock<SymbolTable> = RwLock::new(SymbolTable::new());
}

/// A unique identifier for an interned string.
/// O(1) comparison and copying.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SymbolId(pub u32);

impl std::fmt::Display for SymbolId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SymbolId({})", self.0)
    }
}

/// The global symbol table that stores all interned strings.
#[derive(Debug)]
pub struct SymbolTable {
    strings: Vec<&'static str>,
    map: HashMap<&'static str, SymbolId>,
}

impl SymbolTable {
    fn new() -> Self {
        Self {
            strings: Vec::new(),
            map: HashMap::new(),
        }
    }

    /// Intern a string and return its SymbolId.
    /// If the string already exists, returns the existing SymbolId.
    /// The string is leaked to have a 'static lifetime.
    pub fn intern(&mut self, s: &str) -> SymbolId {
        if let Some(&id) = self.map.get(s) {
            return id;
        }
        let id = SymbolId(self.strings.len() as u32);
        let leaked: &'static str = Box::leak(s.to_string().into_boxed_str());
        self.strings.push(leaked);
        self.map.insert(leaked, id);
        id
    }

    /// Get the string for a SymbolId.
    /// Returns None if the id is invalid.
    pub fn get(&self, id: SymbolId) -> Option<&'static str> {
        self.strings.get(id.0 as usize).copied()
    }

    /// Get the SymbolId for a string, without interning.
    /// Returns None if the string is not in the table.
    pub fn lookup(&self, s: &str) -> Option<SymbolId> {
        self.map.get(s).copied()
    }

    /// Get the number of interned strings.
    pub fn len(&self) -> usize {
        self.strings.len()
    }

    /// Check if the symbol table is empty.
    pub fn is_empty(&self) -> bool {
        self.strings.is_empty()
    }

    /// Clear all interned strings (useful for testing).
    pub fn clear(&mut self) {
        self.strings.clear();
        self.map.clear();
    }
}

/// Get a SymbolId for a string, creating one if it doesn't exist.
pub fn get_symbol(s: &str) -> SymbolId {
    // Fast path: check if already interned with read lock
    if let Some(id) = SYMBOL_TABLE.read().unwrap().lookup(s) {
        return id;
    }
    // Slow path: need to intern with write lock
    SYMBOL_TABLE.write().unwrap().intern(s)
}

/// Look up a SymbolId for a string (returns None if not interned).
pub fn lookup_symbol(s: &str) -> Option<SymbolId> {
    SYMBOL_TABLE.read().unwrap().lookup(s)
}

/// Get the string for a SymbolId.
pub fn symbol_string(id: SymbolId) -> Option<&'static str> {
    SYMBOL_TABLE.read().unwrap().get(id)
}

/// Get or create a SymbolId from a Value (for Value::String).
pub fn value_to_symbol(value: &super::value::Value) -> Option<SymbolId> {
    match value {
        super::value::Value::String(s) => Some(get_symbol(s)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_symbol_interning() {
        let mut table = SymbolTable::new();

        let id1 = table.intern("hello");
        let id2 = table.intern("world");
        let id3 = table.intern("hello"); // Same string, should get same ID

        assert_eq!(id1, id3);
        assert_ne!(id1, id2);
        assert_eq!(table.get(id1), Some("hello"));
        assert_eq!(table.get(id2), Some("world"));
    }

    #[test]
    fn test_symbol_comparison() {
        let id1 = get_symbol("test");
        let id2 = get_symbol("test");
        let id3 = get_symbol("other");

        assert_eq!(id1, id2); // Same string -> same ID
        assert_ne!(id1, id3); // Different string -> different ID

        // SymbolId comparison is O(1) integer comparison
        assert!(id1 == id2);
        assert!(id1 != id3);
    }
}
