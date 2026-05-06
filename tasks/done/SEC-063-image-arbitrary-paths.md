# SEC-063: `Image.new` / `Image.plan` accept arbitrary local paths

- **Severity:** Medium
- **Status:** Todo
- **Location:** `src/interpreter/builtins/image.rs:584-602, 605-620`

**Issue:** If a controller passes `params["path"]` directly, an attacker reads or processes any file the worker can open (no allowlist, no path canonicalization). The same applies to `to_file`/`save_to` — write-anywhere primitive when paths come from request data. Adjacent to SEC-006 but specific to the image module.

**Fix:** Document the trust contract; root paths under a config-controlled directory by default; require an explicit `Trusted` opt-out.
