# SEC-061: SOAP error path emits unescaped exception via `format!` JSON

- **Severity:** Medium
- **Status:** Todo
- **Location:** `src/interpreter/builtins/soap.rs:338`

**Issue:** `format!("{{\"error\": \"{}\"}}", e)` — `e.to_string()` is interpolated into the JSON string without escaping. Any `"` in the error message produces invalid JSON the caller may then `JSON.parse` (and crash), or an attacker on the upstream side can inject keys into the error object.

**Fix:** Build the JSON via `serde_json::json!({"error": e.to_string()}).to_string()`.
