//! JIT profiler for hot path detection.
//!
//! Tracks function call counts and identifies hot functions that should be JIT compiled.

use std::collections::HashMap;

/// Threshold for considering a function "hot" and worth JIT compiling.
pub const JIT_THRESHOLD: u32 = 100;

/// Profiler that tracks function call counts.
#[derive(Debug, Default)]
pub struct Profiler {
    /// Call counts per function name.
    call_counts: HashMap<String, u32>,
    /// Functions that have been marked as hot.
    hot_functions: HashMap<String, bool>,
}

impl Profiler {
    /// Create a new profiler.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a function call.
    /// Returns true if the function just became hot (crossed the threshold).
    pub fn record_call(&mut self, function_name: &str) -> bool {
        let count = self
            .call_counts
            .entry(function_name.to_string())
            .or_insert(0);
        *count += 1;

        // Check if this function just became hot
        if *count == JIT_THRESHOLD && !self.is_hot(function_name) {
            self.hot_functions.insert(function_name.to_string(), true);
            return true;
        }

        false
    }

    /// Check if a function is hot (has been called enough times to warrant JIT).
    pub fn is_hot(&self, function_name: &str) -> bool {
        self.hot_functions
            .get(function_name)
            .copied()
            .unwrap_or(false)
    }

    /// Get the call count for a function.
    pub fn get_call_count(&self, function_name: &str) -> u32 {
        self.call_counts.get(function_name).copied().unwrap_or(0)
    }

    /// Mark a function as JIT compiled.
    pub fn mark_jit_compiled(&mut self, function_name: &str) {
        self.hot_functions.insert(function_name.to_string(), true);
    }

    /// Get all hot functions that haven't been JIT compiled yet.
    pub fn get_hot_functions(&self) -> Vec<String> {
        self.hot_functions
            .iter()
            .filter(|(_, &is_hot)| is_hot)
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// Reset the profiler state.
    pub fn reset(&mut self) {
        self.call_counts.clear();
        self.hot_functions.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_call_counting() {
        let mut profiler = Profiler::new();

        // Record some calls
        for _ in 0..50 {
            profiler.record_call("test_func");
        }

        assert_eq!(profiler.get_call_count("test_func"), 50);
        assert!(!profiler.is_hot("test_func"));
    }

    #[test]
    fn test_hot_detection() {
        let mut profiler = Profiler::new();

        // Record calls up to threshold
        for i in 0..JIT_THRESHOLD {
            let became_hot = profiler.record_call("hot_func");
            if i == JIT_THRESHOLD - 1 {
                assert!(became_hot, "Function should become hot at threshold");
            } else {
                assert!(!became_hot);
            }
        }

        assert!(profiler.is_hot("hot_func"));
        assert_eq!(profiler.get_call_count("hot_func"), JIT_THRESHOLD);
    }

    #[test]
    fn test_multiple_functions() {
        let mut profiler = Profiler::new();

        // Different functions
        for _ in 0..JIT_THRESHOLD {
            profiler.record_call("func_a");
        }
        for _ in 0..50 {
            profiler.record_call("func_b");
        }

        assert!(profiler.is_hot("func_a"));
        assert!(!profiler.is_hot("func_b"));
    }
}
