# match_classify

Write a function `classify(x)` that uses **pattern matching** to return:

- `"zero"` if `x == 0`
- `"positive-small"` if `0 < x < 10`
- `"positive-large"` if `x >= 10`
- `"negative"` if `x < 0`
- `"other"` for everything else (booleans, strings, hashes, …)

```soli
classify(0)    // => "zero"
classify(5)    // => "positive-small"
classify(42)   // => "positive-large"
classify(-3)   // => "negative"
classify("hi") // => "other"
```

**Idiomatic touches we want to see**
- A `match` expression, not a chain of `if/else if`.
- A literal pattern (`0 => ...`) and binding patterns with guards
  (`n if n.is_a?("int") && n > 0 => ...`). Guards must type-check before
  comparing — `>` on a non-number throws.
- A wildcard arm `_ => ...`.
