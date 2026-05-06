# SEC-073: `solidb` instance state holds plaintext password in process-global map

- **Severity:** Low
- **Status:** Todo
- **Location:** `src/interpreter/builtins/solidb.rs:49-66, 353-360`

**Issue:** Credentials live in memory keyed by an `_id` in a `RwLock<HashMap>`. `close()` removes them but they otherwise persist for the process lifetime. Not exploitable on its own; relevant if memory dumps or core dumps reach attackers.

**Fix:** Zero out the password buffer immediately after the JWT bootstrap; store only the derived token; consider `secrecy::SecretString` or a `Drop` impl that wipes on free.
