# SEC-065: `http.rs` is dead code with divergent SSRF logic

- **Severity:** Low
- **Status:** Todo
- **Location:** `src/interpreter/builtins/http.rs:1-138`

**Issue:** `register_http_builtins` is empty, so this file's `validate_url_for_ssrf` is never called — but its existence and divergence from `http_class.rs` invites future drift, where a maintainer "fixes" only one of the two checks.

**Fix:** Delete `src/interpreter/builtins/http.rs` after confirming nothing else imports it.
