//! The process environment. Natively it is `std::env`; the edge build has none,
//! so the host passes its bindings (a Worker's `env`) to `set` once at boot.

use std::sync::OnceLock;

static ENV: OnceLock<std::collections::HashMap<String, String>> = OnceLock::new();

/// Set the process environment a Worker cannot have (its `env` bindings are
/// handed over by the host instead). First call wins.
pub fn set(vars: Vec<(String, String)>) {
    let _ = ENV.set(vars.into_iter().collect());
}

/// `std::env::var`, answered from `set`'s table once one is set.
pub fn var<K: AsRef<std::ffi::OsStr>>(key: K) -> Result<String, std::env::VarError> {
    match ENV.get() {
        Some(vars) => key
            .as_ref()
            .to_str()
            .and_then(|k| vars.get(k).cloned())
            .ok_or(std::env::VarError::NotPresent),
        None => std::env::var(key),
    }
}

#[cfg(test)]
mod tests {
    // Natively nothing calls `set`, so `var` must be `std::env::var` — the
    // edge-only table must not stand in the way (or call back into itself).
    #[test]
    fn reads_the_process_environment_until_a_table_is_set() {
        assert_eq!(super::var("PATH").ok(), std::env::var("PATH").ok());
        assert!(super::var("SOLI_SURELY_UNSET_VARIABLE_FOR_THIS_TEST").is_err());
    }
}
