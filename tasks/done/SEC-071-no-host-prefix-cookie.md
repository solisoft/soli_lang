# SEC-071: Session cookie has no `__Host-` prefix and no `SameSite=Strict` knob

- **Severity:** Low
- **Status:** Todo
- **Location:** `src/interpreter/builtins/session.rs:417-425`

**Issue:** Cookie name is `session_id` with `Path=/; HttpOnly; SameSite=Lax; Max-Age=24h`. `SameSite=Lax` is reasonable, but no `__Host-` prefix means a sibling subdomain can overwrite the cookie (cookie tossing). No `SameSite=Strict` option is exposed for high-security apps.

**Fix:** Offer `__Host-session_id` (when `Secure` + `Path=/`) and a `SameSite` config knob (`Strict`/`Lax`/`None`).
