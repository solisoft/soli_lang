# dedup_preserve_order

Write a function `dedup(arr)` that returns the input array with duplicates
removed, preserving the order of first occurrence.

```soli
dedup([1, 2, 2, 3, 1, 4, 3])
// => [1, 2, 3, 4]

dedup([])
// => []

dedup(["a", "a", "a"])
// => ["a"]
```

**Idiomatic touches we want to see**
- Use a `Set`-like marker (Soli doesn't have a Set builtin — use a hash
  keyed by the stringified value, or a parallel "seen" array if values
  aren't hashable).
- A `for` loop building up the result.
- Implicit return.
