# group_by_array

Write a function `group_by(arr, key_fn)` that takes an array and a key-extractor
lambda, and returns a hash where each key is the extracted value and the value
is the array of items that produced that key. Preserve input order of first
occurrence in both the keys and the elements within each group.

```soli
group_by([1, 2, 3, 4], fn(x) x % 2)
// => { "1": [1, 3], "0": [2, 4] }

group_by(
  [{"name": "Alice", "team": "red"}, {"name": "Bob", "team": "blue"}, {"name": "Cara", "team": "red"}],
  fn(p) p["team"]
)
// => { "red": [{"name": "Alice", "team": "red"}, {"name": "Cara", "team": "red"}],
//      "blue": [{"name": "Bob", "team": "blue"}] }
```

**Constraints**
- No external libraries. Use the language builtins.
- Use a hash literal or `hash()` builder; do not rely on a global `Hash` class.
- Keys in the resulting hash are coerced to strings (Soli hashes stringify keys).
- Don't mutate the input array.

**Idiomatic Soli touches we want to see**
- A `fn` lambda, not a block-form `|x| { ... }`.
- Use `len()` to get the array length.
- Last expression in the function is returned implicitly.
