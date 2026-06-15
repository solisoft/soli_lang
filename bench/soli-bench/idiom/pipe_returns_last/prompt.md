# pipe_returns_last

The stub builds its result with a manual accumulator buried under four levels
of nested `if`s inside a `for`. Soli prefers a flat transformation chain —
`.filter(...).map(...).sort()` — where each step feeds the next and the final
expression is the result. It reads top-to-bottom and stays shallow.

Rewrite `stub.sl` so `admin_emails(users)` returns the same value using a
method/pipeline chain instead of nested loops and conditionals.

- Keep the same behavior (the tests must still pass).
- Collapse the nesting: `soli lint` flags `smell/deep-nesting` (depth > 4) on
  the stub, and a clean chain reports zero issues.

```soli
# Prefer a chain like:
users
    .filter(fn(user) ...)
    .map(fn(user) user["email"])
    .sort()
```
