# let_overuse

In Soli `let` is optional — a bare `name = value` already creates the binding.
Reach for `let` only when it earns its keep (a type annotation, or hoisting a
variable before an `if`/`match` that assigns it in each branch). Sprinkling
`let` over every line is noise, and because `let` is optional a typo'd name
silently becomes a *new* read that the linter catches as `smell/undefined-local`.

The stub wraps each value in a needless `let` and contains a typo that the
linter flags. Rewrite `stub.sl` so it:

- drops the unnecessary `let` bindings (use the parameters / bare assignment
  directly)
- fixes the typo so no name is read-but-never-assigned
- keeps the same behavior (the tests must still pass)

`soli lint` reports zero issues on a clean rewrite.
