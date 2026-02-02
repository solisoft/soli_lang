//! String, Array, Hash, Set, Range, and Base64 built-in classes for SoliLang.
//!
//! These classes wrap the primitive Value types and provide methods on them.
//! When a literal like "hello", [1, 2, 3], or {"a": 1} is created,
//! the interpreter automatically wraps it in the appropriate class instance.

pub mod array;
pub mod hash;
pub mod range;
pub mod set;
pub mod traits;
pub mod utils;

pub use array::register_array_class;
pub use hash::register_hash_class;
pub use range::register_range_class;
pub use set::register_set_class;
pub use utils::{
    register_base64_class, register_collection_classes, wrap_array, wrap_hash, wrap_string,
};
