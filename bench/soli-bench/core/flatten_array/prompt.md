# flatten_array

Write a function `flatten(arr)` that takes a (possibly) nested array of integers
and returns a flat array. Only integers, but they may be nested one or more
levels deep.

```soli
flatten([1, [2, 3], [4, [5, 6]], 7])
// => [1, 2, 3, 4, 5, 6, 7]

flatten([1, 2, 3])
// => [1, 2, 3]

flatten([])
// => []
```

**Constraints**
- No external libraries.
- Don't use a third-party `flatten` if one exists — the point is to see how you
  express recursion/iteration in Soli.

**Idiomatic touches we want to see**
- A `for` loop or a recursive `fn` lambda.
- `len()` and `push()` for the result accumulator.
- Last expression returned implicitly.
