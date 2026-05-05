//! Environment file loading utilities

use std::path::Path;

/// Load environment variables from .env files in the application directory.
/// This loads .env first, then .env.{APP_ENV} if APP_ENV is set.
pub fn load_env_files(folder: &Path) {
    load_env_file(folder, ".env", false);

    if let Ok(app_env) = std::env::var("APP_ENV") {
        load_env_file(folder, &format!(".env.{}", app_env), true);
    }
}

/// Load a single .env file
///
/// # Arguments
/// * `folder` - The directory containing the .env file
/// * `filename` - The name of the .env file
/// * `override_existing` - Whether to override existing environment variables
///
/// `SOLI_PROTECT_ENV` (comma-separated list of variable names) names env
/// vars the parent process explicitly set and that this loader must NOT
/// override, even when `override_existing` is true. Used by the parallel
/// test runner so each per-worker `SOLIDB_DATABASE=test_wN` survives the
/// `.env.test` reload that happens during server startup.
pub fn load_env_file(folder: &Path, filename: &str, override_existing: bool) {
    let env_file = folder.join(filename);
    if !env_file.exists() {
        return;
    }

    let Ok(content) = std::fs::read_to_string(&env_file) else {
        return;
    };

    let protected: Vec<String> = std::env::var("SOLI_PROTECT_ENV")
        .ok()
        .map(|s| {
            s.split(',')
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .collect()
        })
        .unwrap_or_default();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };

        let key = key.trim();
        let value = value.trim().trim_matches('"').trim_matches('\'');

        // SEC-052: validate the key against `[A-Za-z_][A-Za-z0-9_]*`.
        // A malformed key (spaces, `=`, shell metacharacters) is almost
        // always a typo or a probe; passing it through to `set_var`
        // would either silently fail or — depending on platform — set a
        // weird env entry that downstream code can't read back cleanly.
        if !is_valid_env_key(key) {
            eprintln!(
                "[env_loader] skipping {} entry with invalid key {:?} (must match [A-Za-z_][A-Za-z0-9_]*)",
                filename, key
            );
            continue;
        }

        // SEC-052: reject values containing NUL / CR / LF. `str::lines`
        // already strips trailing CRLF and NL, but a bare `\r` in the
        // middle of a line (or a `\0`) survives and would later become
        // a header-split / log-injection vector when the value flows
        // into HTTP responses, structured logs, or shell-mode
        // System.run. Reject loudly rather than strip — silent
        // sanitization hides the upstream bug.
        if value.bytes().any(|b| matches!(b, b'\0' | b'\r' | b'\n')) {
            eprintln!(
                "[env_loader] skipping {} entry {:?}: value contains NUL / CR / LF",
                filename, key
            );
            continue;
        }

        let is_protected = protected.iter().any(|p| p == key);
        let already_set = std::env::var(key).is_ok();

        if (override_existing && !is_protected) || !already_set {
            // SEC-033: `set_var` is `unsafe` because of multi-thread UB on
            // Rust 2024 / glibc. This call is safe because every caller
            // of `load_env_file[s]` runs at single-threaded boot, before
            // worker threads are spawned: `serve::serve_folder` (line 249),
            // `cli::commands::test_runner::main` (line 300), and the REPL
            // entry. The `setenv`/`dotenv` runtime builtins that wrapped
            // this call from worker code were removed in SEC-033.
            unsafe { std::env::set_var(key, value) };
        }
    }
}

/// SEC-052: an env var name must match `[A-Za-z_][A-Za-z0-9_]*`. POSIX
/// permits more than this, but the .env files we parse are
/// developer-authored config — anything outside this conservative shape
/// is almost always a typo, a stray `=`, or a probe attempt.
fn is_valid_env_key(key: &str) -> bool {
    let mut bytes = key.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == b'_') {
        return false;
    }
    bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_env_keys_accepted() {
        for k in ["FOO", "FOO_BAR", "_PRIVATE", "API_KEY_2", "a", "_", "X9"] {
            assert!(is_valid_env_key(k), "expected accepted: {k:?}");
        }
    }

    #[test]
    fn invalid_env_keys_rejected() {
        // SEC-052: spaces, `=`, leading digits, dashes, shell metas,
        // empty — all rejected.
        for k in [
            "", "FOO BAR", "FOO=BAR", "1ST", "a-b", "$FOO", "FOO;rm", "FOO\rBAR", "FOO\nBAR",
        ] {
            assert!(!is_valid_env_key(k), "expected rejected: {k:?}");
        }
    }

    #[test]
    fn loader_skips_value_with_embedded_cr_or_lf() {
        // SEC-052: a `.env` line whose value contains a bare CR (or NL,
        // when constructed in-process) must not be exported to the env;
        // it would become an HTTP-header-split / log-injection vector
        // downstream.
        let dir = tempfile::tempdir().unwrap();
        let key = "SEC052_TEST_CR_LF_VALUE";
        // Pre-clear in case a stale value is still set in this proc.
        unsafe { std::env::remove_var(key) };

        // Write a .env containing a value with an embedded CR.
        let content = format!("{key}=bar\\rLD_PRELOAD=/tmp/x\n").replace("\\r", "\r");
        std::fs::write(dir.path().join(".env"), content).unwrap();

        load_env_file(dir.path(), ".env", true);
        assert!(
            std::env::var(key).is_err(),
            "expected {key} to remain unset (was {:?})",
            std::env::var(key)
        );
    }

    #[test]
    fn loader_skips_invalid_key() {
        // SEC-052: a `.env` line with a malformed key must be skipped.
        // Use an in-process unique key root to avoid collisions across
        // parallel tests.
        let dir = tempfile::tempdir().unwrap();
        // Embed a leading digit (invalid) and confirm nothing is set.
        let probe = "1SEC052_TEST_BAD_KEY";
        unsafe { std::env::remove_var(probe) };
        std::fs::write(dir.path().join(".env"), format!("{probe}=value\n")).unwrap();

        load_env_file(dir.path(), ".env", true);
        assert!(std::env::var(probe).is_err());
    }
}
