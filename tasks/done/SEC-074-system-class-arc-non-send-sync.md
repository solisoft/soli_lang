# SEC-074: `system_class` uses `arc_with_non_send_sync` lint silencer

- **Severity:** Low
- **Status:** Todo
- **Location:** `src/interpreter/builtins/system.rs:26-60`

**Issue:** The `Arc<Mutex<FutureState>>` is moved across `thread::spawn`. The mpsc receiver is `Send`, but the lint-silencing pattern obscures whether other `FutureState` variants stay single-threaded. A misuse here is undefined behaviour, not just a lint.

**Fix:** Make `FutureState` actually `Send + Sync` (or use a thread-safe future primitive); remove the `#[allow]` once the data type is provably safe.
