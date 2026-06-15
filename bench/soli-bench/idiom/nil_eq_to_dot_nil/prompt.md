# nil_eq_to_dot_nil

The stub works, but it tests for `null` the un-idiomatic way: `x == null` and
`x != null`. In Soli every value answers `.nil?` and `.present?`, and those
read better and express intent directly.

Rewrite `stub.sl` so it:

- uses `.nil?` instead of `== null`
- uses `.present?` instead of `!= null`
- keeps the same behavior (the tests must still pass)

`soli lint` flags `== null` / `!= null` as `idiom/nil-comparison`, so a clean
rewrite reports zero issues.

```soli
# Instead of:
if user == null { ... }
return user != null;

# Prefer:
if user.nil? { ... }
return user.present?;
```
