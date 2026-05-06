# SEC-060: `http_log` records full URLs including userinfo / `?api_key=…`

- **Severity:** Medium
- **Status:** Todo
- **Location:** `src/interpreter/builtins/http_log.rs:38-84`; callers in `src/interpreter/builtins/http_class.rs` pass `url.to_string()` unsanitized

**Issue:** URLs like `https://user:token@api/...?api_key=xyz` are recorded verbatim in the per-request log served by the dev bar. Anyone seeing a dev page or a leaked screenshot sees the secrets.

**Fix:** Strip `userinfo` and well-known sensitive query parameters (`api_key`, `token`, `access_token`, `secret`) before logging.
