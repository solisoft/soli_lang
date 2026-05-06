# SEC-058: `<%=` has no context-specific escaping (attribute / JS / URL)

- **Severity:** Medium
- **Status:** Todo
- **Location:** `src/template/renderer.rs:326-348`

**Issue:** `<%= ... %>` escapes the same five chars regardless of whether the output lands in HTML body, attribute, `<script>` block, or URL. Pasting `<%= user_data %>` inside an unquoted attribute (`<div class=<%= cls %>>`) or inside a `<script>` tag is XSS even with HTML escaping applied.

**Fix:** Add `j()` / `attr()` / `url()` helpers; document context rules; add a lint rule against unquoted-attribute interpolation.
