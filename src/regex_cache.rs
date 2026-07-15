//! Global regex compilation cache.
//!
//! Compiling a regex is expensive. This module caches compiled regexes
//! in a global shared cache so repeated use of the same pattern avoids
//! recompilation across all threads.
//!
//! All public entry points apply ReDoS safety limits (`nest_limit` +
//! `size_limit`). User-facing string methods (`gsub`, `match`, `scan`, …),
//! the VM string path, and the `Regex` class share this policy — unbounded
//! `Regex::new` is never used for request-controlled patterns.

use lru::LruCache;
use std::num::NonZero;
use std::sync::{Arc, LazyLock, Mutex};

use regex::{Regex, RegexBuilder};

/// Maximum number of cached regexes before LRU eviction.
const MAX_CACHE_SIZE: NonZero<usize> = NonZero::new(128).unwrap();

/// Maximum regex complexity (nesting level) to prevent ReDoS.
const REGEX_NEST_LIMIT: u32 = 10;

/// Maximum size of the compiled regex in bytes.
const REGEX_SIZE_LIMIT: usize = 100_000;

/// Shared cache: `Arc` so cache hits only clone a refcount, not recompile.
static REGEX_CACHE: LazyLock<Mutex<LruCache<String, Arc<Regex>>>> =
    LazyLock::new(|| Mutex::new(LruCache::new(MAX_CACHE_SIZE)));

/// Compile with ReDoS limits and cache. `err_prefix` keeps the historical
/// error message shape for each public API.
fn get_cached(pattern: &str, err_prefix: &str) -> Result<Regex, String> {
    // Fast path: cache hit — `Regex` is Arc-backed; clone is refcount only.
    {
        let mut cache = REGEX_CACHE.lock().unwrap();
        if let Some(re) = cache.get(pattern) {
            return Ok(re.as_ref().clone());
        }
    }

    let re = RegexBuilder::new(pattern)
        .nest_limit(REGEX_NEST_LIMIT)
        .size_limit(REGEX_SIZE_LIMIT)
        .build()
        .map_err(|e| format!("{}{}", err_prefix, e))?;
    let arc = Arc::new(re);

    let mut cache = REGEX_CACHE.lock().unwrap();
    // Another thread may have inserted while we compiled; prefer the
    // cached entry so we don't thrash the LRU with duplicates.
    if let Some(existing) = cache.get(pattern) {
        return Ok(existing.as_ref().clone());
    }
    cache.put(pattern.to_string(), Arc::clone(&arc));
    Ok(arc.as_ref().clone())
}

/// Get a cached regex with ReDoS safety limits.
///
/// Used by string methods, VM string ops, validation, and assertions.
/// Prefer this (or [`get_safe_regex`]) over any unbounded compile.
#[inline]
pub fn get_regex(pattern: &str) -> Result<Regex, String> {
    get_cached(pattern, "invalid regex: ")
}

/// Get a cached regex with ReDoS safety limits (user-facing `Regex` class).
///
/// Same limits and cache as [`get_regex`]; distinct error prefix for API
/// compatibility.
#[inline]
pub fn get_safe_regex(pattern: &str) -> Result<Regex, String> {
    get_cached(pattern, "Invalid regex pattern: ")
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- get_regex ----------

    #[test]
    fn get_regex_compiles_valid_pattern() {
        // Use a unique pattern per test to avoid relying on prior cache state.
        let re = get_regex(r"^test_\d+$").expect("valid pattern compiles");
        assert!(re.is_match("test_42"));
        assert!(!re.is_match("nope"));
    }

    #[test]
    fn get_regex_returns_invalid_pattern_error_with_prefix() {
        // Unbalanced bracket — definitely invalid.
        let err = get_regex("[unterminated").unwrap_err();
        assert!(err.starts_with("invalid regex:"), "got: {err}");
    }

    #[test]
    fn get_regex_repeated_call_returns_working_regex() {
        // Two consecutive calls for the same pattern should both yield
        // working regexes (the second is the cache-hit path). Pattern is
        // unique to this test so we don't see eviction effects from
        // unrelated parallel tests.
        let p = r"^repeat_test_\w+$";
        let r1 = get_regex(p).unwrap();
        let r2 = get_regex(p).unwrap();
        assert!(r1.is_match("repeat_test_abc"));
        assert!(r2.is_match("repeat_test_abc"));
    }

    #[test]
    fn get_regex_caches_independent_patterns() {
        let a = get_regex(r"^cache_a_\d+$").unwrap();
        let b = get_regex(r"^cache_b_\d+$").unwrap();
        assert!(a.is_match("cache_a_1"));
        assert!(!a.is_match("cache_b_1"));
        assert!(b.is_match("cache_b_1"));
        assert!(!b.is_match("cache_a_1"));
    }

    #[test]
    fn get_regex_rejects_pathological_nesting() {
        // Deeply nested groups exceed nest_limit and must fail closed.
        let nested = "(".repeat(20) + "a" + &")".repeat(20);
        let err = get_regex(&nested).unwrap_err();
        assert!(
            err.starts_with("invalid regex:"),
            "expected nest-limit error, got: {err}"
        );
    }

    // ---------- get_safe_regex ----------

    #[test]
    fn get_safe_regex_compiles_valid_pattern() {
        let re = get_safe_regex(r"^safe_\w+$").expect("valid pattern compiles");
        assert!(re.is_match("safe_token"));
        assert!(!re.is_match("nope!"));
    }

    #[test]
    fn get_safe_regex_returns_error_with_safe_prefix() {
        let err = get_safe_regex("(unbalanced").unwrap_err();
        assert!(err.starts_with("Invalid regex pattern:"), "got: {err}");
    }

    #[test]
    fn get_safe_regex_repeated_call_returns_working_regex() {
        let p = r"^safe_repeat_\d+$";
        let r1 = get_safe_regex(p).unwrap();
        let r2 = get_safe_regex(p).unwrap();
        assert!(r1.is_match("safe_repeat_99"));
        assert!(r2.is_match("safe_repeat_99"));
    }

    #[test]
    fn get_regex_and_get_safe_regex_share_cache() {
        // Both APIs share one limited cache; a hit via either path works.
        let p = r"^shared_cache_\d+$";
        let via_get = get_regex(p).unwrap();
        let via_safe = get_safe_regex(p).unwrap();
        assert!(via_get.is_match("shared_cache_1"));
        assert!(via_safe.is_match("shared_cache_1"));
    }
}
