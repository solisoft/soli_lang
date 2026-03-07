//! Global regex compilation cache.
//!
//! Compiling a regex is expensive. This module caches compiled regexes
//! in a global shared cache so repeated use of the same pattern avoids
//! recompilation across all threads.

use std::sync::{LazyLock, RwLock};

use ahash::AHashMap;
use regex::{Regex, RegexBuilder};

/// Maximum number of cached regexes before eviction.
const MAX_CACHE_SIZE: usize = 128;

/// Maximum regex complexity (nesting level) to prevent ReDoS.
const REGEX_NEST_LIMIT: u32 = 10;

/// Maximum size of the compiled regex in bytes.
const REGEX_SIZE_LIMIT: usize = 100_000;

static REGEX_CACHE: LazyLock<RwLock<AHashMap<String, Regex>>> =
    LazyLock::new(|| RwLock::new(AHashMap::with_capacity(32)));

static SAFE_REGEX_CACHE: LazyLock<RwLock<AHashMap<String, Regex>>> =
    LazyLock::new(|| RwLock::new(AHashMap::with_capacity(16)));

/// Get a cached regex, compiling it on first use.
#[inline]
pub fn get_regex(pattern: &str) -> Result<Regex, String> {
    // Fast path: read lock
    {
        let cache = REGEX_CACHE.read().unwrap();
        if let Some(re) = cache.get(pattern) {
            return Ok(re.clone());
        }
    }
    // Slow path: compile and insert with write lock
    let re = Regex::new(pattern).map_err(|e| format!("invalid regex: {}", e))?;
    let mut cache = REGEX_CACHE.write().unwrap();
    // Double-check (another thread may have inserted while we waited)
    if let Some(re) = cache.get(pattern) {
        return Ok(re.clone());
    }
    if cache.len() >= MAX_CACHE_SIZE {
        cache.clear();
    }
    cache.insert(pattern.to_string(), re.clone());
    Ok(re)
}

/// Get a cached regex with ReDoS safety limits (for user-facing Regex class).
#[inline]
pub fn get_safe_regex(pattern: &str) -> Result<Regex, String> {
    // Fast path: read lock
    {
        let cache = SAFE_REGEX_CACHE.read().unwrap();
        if let Some(re) = cache.get(pattern) {
            return Ok(re.clone());
        }
    }
    // Slow path: compile and insert with write lock
    let re = RegexBuilder::new(pattern)
        .nest_limit(REGEX_NEST_LIMIT)
        .size_limit(REGEX_SIZE_LIMIT)
        .build()
        .map_err(|e| format!("Invalid regex pattern: {}", e))?;
    let mut cache = SAFE_REGEX_CACHE.write().unwrap();
    if let Some(re) = cache.get(pattern) {
        return Ok(re.clone());
    }
    if cache.len() >= MAX_CACHE_SIZE {
        cache.clear();
    }
    cache.insert(pattern.to_string(), re.clone());
    Ok(re)
}
