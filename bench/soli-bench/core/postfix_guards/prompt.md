# postfix_guards

Three small one-liner tasks. The point is to write them in **concise
guard forms** — ternary, `??`, postfix `unless` — not as verbose
`if/else` blocks.

1. `greet_adult(age)` — return `"hello adult"` if `age >= 18`, else `"hello minor"`.
   - **Use** the ternary form: `cond ? a : b`.

2. `safe_value(s)` — return `s` if it's not blank, else `"default"`.
   - **Use** the nullish-coalescing `??` operator (or `.blank?` truthy
     chain) — not an `if/else`.

3. `maybe_double(x)` — return `x * 2` **unless** `x` is `null`.
   - **Use** the postfix `unless` form on a *statement* and an explicit
     null return for the early-exit case.

```soli
greet_adult(30)    // => "hello adult"
greet_adult(12)    // => "hello minor"
safe_value("")     // => "default"
safe_value("ok")   // => "ok"
maybe_double(5)    // => 10
maybe_double(null) // => null
```

**Idiomatic touches we want to see**
- Ternary `cond ? a : b`.
- `??` for nullable defaults.
- Postfix `unless` for early-exit assignment.
- Bare assignment (`x = ...`), not `let x = ...`, when the type is obvious.
