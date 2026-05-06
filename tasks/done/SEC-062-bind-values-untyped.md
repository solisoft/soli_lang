# SEC-062: Hash-keyed bind values pass through untyped

- **Severity:** Medium
- **Status:** Todo
- **Location:** `src/interpreter/builtins/model/core.rs:1285-1302` (`Model.where` bind hash), `:1664-1666` (`find_by` value)

**Issue:** Bind values come from `value_to_json(v)`, so a JSON object/array submitted by the user becomes the bind value. SolidB likely treats `@val` as a literal so a `{"$ne": null}`-style operator bypass isn't directly exploitable, but if user code does `User.where("doc.role == @r", {r: req.params["role"]})` and the attacker submits `role` as `{}` or an array, comparison silently mismatches expected logic and security checks may pass for unintended rows.

**Fix:** Restrict bind values to scalars by default; require explicit opt-in for array/object binds.
