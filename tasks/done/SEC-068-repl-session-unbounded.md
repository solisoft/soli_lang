# SEC-068: `repl_session` thread-local store has no global cap

- **Severity:** Low
- **Status:** Todo
- **Location:** `src/serve/repl_session.rs:23-92`

**Issue:** Each thread keeps its own UUID-keyed map; cleanup is probabilistic (1-in-50). With many tabs/sessions across many threads memory grows unbounded for 30 min. Dev-only, but a misconfigured `ALLOW_REMOTE` on a public host turns this into a DoS knob.

**Fix:** Bound `sessions.len()` per thread; evict LRU.
