# zip_with

Write a function `zip_with(a, b, f)` that pairs the i-th element of `a` with
the i-th element of `b` and applies `f(a[i], b[i])`. Stop at the shorter of
the two arrays.

```soli
zip_with([1, 2, 3], [10, 20, 30], fn(x, y) x + y)
// => [11, 22, 33]

zip_with([1, 2, 3], [10, 20], fn(x, y) x + y)
// => [11, 22]

zip_with([], [1, 2], fn(x, y) x + y)
// => []
```

**Idiomatic touches we want to see**
- Use a `for` loop with `range` or a manual index.
- Use a `fn` lambda for the combiner.
