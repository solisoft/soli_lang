//! Global regex compilation cache.
//!
//! Compiling a regex is expensive. This module caches compiled regexes
//! in a global shared cache so repeated use of the same pattern avoids
//! recompilation across all threads.

use lru::LruCache;
use std::num::NonZero;
use std::sync::{LazyLock, Mutex};

use regex::{Regex, RegexBuilder};

/// Maximum number of cached regexes before LRU eviction.
const MAX_CACHE_SIZE: NonZero<usize> = NonZero::new(128).unwrap();

/// Maximum regex complexity (nesting level) to prevent ReDoS.
const REGEX_NEST_LIMIT: u32 = 10;

/// Maximum size of the compiled regex in bytes.
const REGEX_SIZE_LIMIT: usize = 100_000;

static REGEX_CACHE: LazyLock<Mutex<LruCache<String, Regex>>> =
    LazyLock::new(|| Mutex::new(LruCache::new(MAX_CACHE_SIZE)));

static SAFE_REGEX_CACHE: LazyLock<Mutex<LruCache<String, Regex>>> =
    LazyLock::new(|| Mutex::new(LruCache::new(MAX_CACHE_SIZE)));

/// Get a cached regex, compiling it on first use.
#[inline]
pub fn get_regex(pattern: &str) -> Result<Regex, String> {
    let key = pattern.to_string();

    // Try to find existing entry (get() updates LRU order, needs exclusive access)
    {
        let mut cache = REGEX_CACHE.lock().unwrap();
        if let Some(re) = cache.get(&key) {
            return Ok(re.clone());
        }
    }

    // Slow path: compile and insert with exclusive lock
    let re = Regex::new(pattern).map_err(|e| format!("invalid regex: {}", e))?;
    let mut cache = REGEX_CACHE.lock().unwrap();
    // LRU eviction happens automatically when at capacity
    cache.put(key, re.clone());
    Ok(re)
}

/// Get a cached regex with ReDoS safety limits (for user-facing Regex class).
#[inline]
pub fn get_safe_regex(pattern: &str) -> Result<Regex, String> {
    let key = pattern.to_string();

    // Try to find existing entry
    {
        let mut cache = SAFE_REGEX_CACHE.lock().unwrap();
        if let Some(re) = cache.get(&key) {
            return Ok(re.clone());
        }
    }

    // Slow path: compile and insert
    let re = RegexBuilder::new(pattern)
        .nest_limit(REGEX_NEST_LIMIT)
        .size_limit(REGEX_SIZE_LIMIT)
        .build()
        .map_err(|e| format!("Invalid regex pattern: {}", e))?;
    let mut cache = SAFE_REGEX_CACHE.lock().unwrap();
    // LRU eviction happens automatically when at capacity
    cache.put(key, re.clone());
    Ok(re)
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

    // ---------- get_safe_regex ----------

    #[test]
    fn get_safe_regex_compiles_valid_pattern() {
        let re = get_safe_regex(r"^safe_\w+$").expect("valid pattern compiles");
        assert!(re.is_match("safe_token"));
        assert!(!re.is_match("nope!"));
    }

    #[test]
    fn get_safe_regex_returns_error_with_safe_prefix() {
        // Different prefix from the unsafe variant — let's pin it.
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
    fn safe_and_unsafe_caches_are_distinct() {
        // The two caches use different RegexBuilder configs, so an
        // identical pattern lives in both independently.
        let p = r"^both_caches_\d+$";
        let unsafe_re = get_regex(p).unwrap();
        let safe_re = get_safe_regex(p).unwrap();
        assert!(unsafe_re.is_match("both_caches_1"));
        assert!(safe_re.is_match("both_caches_1"));
    }
}
