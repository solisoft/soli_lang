# includes_check

The stub tests membership with long chains of `==` (and `!=`) against the same
value. The idiomatic form lists the options once and asks the array:
`["a", "b", "c"].includes?(x)`.

Rewrite `stub.sl` so it:

- replaces `x == "a" || x == "b" || x == "c"` with `[...].includes?(x)`
- replaces `x != "a" && x != "b" && x != "c"` with `![...].includes?(x)`
  (or `unless [...].includes?(x)`)
- keeps the same behavior (the tests must still pass)

`soli lint` flags 3+ same-value comparison chains as `idiom/prefer-includes`,
so a clean rewrite reports zero issues.
