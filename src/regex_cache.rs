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
