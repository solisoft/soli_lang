# SEC-069: `Factory.create_with` ignores the registered factory

- **Severity:** Low
- **Status:** Todo
- **Location:** `src/interpreter/builtins/factories.rs:46-48`

**Issue:** Returns the user-supplied overrides verbatim instead of merging them with the registered factory's defaults. Test-only, but masks bugs in fixtures and gives a false sense of test coverage.

**Fix:** Merge `overrides` over the factory's default attributes; add a regression test.
