//! Hidden-class ids.
//!
//! Every hidden class (`hidden_class.rs`) takes a process-unique id from
//! [`INLINE_CACHE`]. The polymorphic property/method caches that once lived
//! here beside the counter were never wired into either engine and are gone.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::LazyLock;

pub static INLINE_CACHE: LazyLock<InlineCacheRegistry> = LazyLock::new(InlineCacheRegistry::new);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HiddenClassId(pub u32);

#[derive(Debug)]
pub struct InlineCacheRegistry {
    next_hidden_class_id: AtomicU32,
}

impl InlineCacheRegistry {
    fn new() -> Self {
        Self {
            next_hidden_class_id: AtomicU32::new(0),
        }
    }

    pub fn new_hidden_class_id(&self) -> HiddenClassId {
        let id = self.next_hidden_class_id.fetch_add(1, Ordering::Relaxed);
        HiddenClassId(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hidden_class_ids() {
        let id1 = INLINE_CACHE.new_hidden_class_id();
        let id2 = INLINE_CACHE.new_hidden_class_id();

        assert_ne!(id1, id2);
    }
}
