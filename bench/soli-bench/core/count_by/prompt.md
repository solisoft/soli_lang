# count_by

Write a function `count_by(arr, key_fn)` that returns a hash where each key
is the result of `key_fn(item)` and each value is the count of items that
produced that key.

```soli
count_by([1, 2, 3, 4, 5, 6], fn(x) x % 2)
// => { "0": 3, "1": 3 }

count_by(
  ["apple", "apricot", "banana", "blueberry", "cherry"],
  fn(s) s[0]
)
// => { "a": 2, "b": 2, "c": 1 }
```

**Idiomatic touches we want to see**
- A `for` loop, `has_key` for "have we seen this key before".
- Return a hash, not a 2-D array.
