# SEC-070: `partial` / `render_partial` accept dynamic name from controller code

- **Severity:** Low
- **Status:** Todo
- **Location:** `src/interpreter/builtins/template.rs:915-939`

**Issue:** `is_safe_template_name` correctly blocks `..` and absolute paths, but a user-controlled partial name (e.g. `render_partial(req.params["view"])`) still lets an attacker pick which arbitrary partial in the project to render — potentially leaking data from a partial intended for a different controller's view (lateral disclosure, not traversal).

**Fix:** Recommend (and lint for) literal-string partial names; document the risk in the templating docs.
