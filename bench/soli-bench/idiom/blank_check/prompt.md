# blank_check

The stub checks for an empty string with `name == ""`. The idiomatic Soli
test is `.blank?`, which folds nil *and* empty-string into one check (and its
inverse `.present?`).

Rewrite `stub.sl` so it:

- uses `.blank?` instead of comparing to `""`
- keeps the same behavior (the tests must still pass)

`soli lint` flags `== ""` / `!= ""` as `idiom/prefer-blank`, so a clean rewrite
reports zero issues.

```soli
# Instead of:
if name == "" { ... }

# Prefer:
if name.blank? { ... }
```
