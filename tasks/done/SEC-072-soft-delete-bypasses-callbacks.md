# SEC-072: Soft-delete `instance.delete()` bypasses `before_delete` callbacks

- **Severity:** Low
- **Status:** Todo
- **Location:** `src/interpreter/builtins/model/core.rs:2374-2417`

**Issue:** Authorization or audit logic hooked in `before_delete` is silently skipped for soft-deleted models. Documentation/policy issue rather than a primitive bug, but apps relying on `before_delete` for access control have a hole.

**Fix:** Run `before_delete` for both hard and soft deletes; document the contract; or rename the soft-delete path so the difference is explicit.
