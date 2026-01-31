//! String manipulation built-in functions have been removed.
//! String methods are now only available via the String class.
//! Example: "hello".upcase() instead of upcase("hello")

use crate::interpreter::environment::Environment;

/// Register all string manipulation built-in functions.
/// NOTE: Most string functions have been moved to the String class.
/// These functions are kept for backward compatibility but may be removed in a future version.
pub fn register_string_builtins(_env: &mut Environment) {
    // All string methods are now available via the String class.
    // Example: "hello".split(",") instead of split("hello", ",")
}
