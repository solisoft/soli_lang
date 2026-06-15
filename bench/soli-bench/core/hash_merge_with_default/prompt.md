# hash_merge_with_default

Write a function `merge_with_default(base, override, default_value)` that:

- Returns a new hash containing every key from `base` and `override`.
- For each key present in `override`, the value is `override[k]` (when non-null),
  else `default_value`.
- For each key present only in `base`, the value is `base[k]`.

```soli
merge_with_default(
    {"a": 1, "b": 2},
    {"b": null, "c": 3},
    -1
)
// => { "a": 1, "b": -1, "c": 3 }
```

**Idiomatic touches we want to see**
- Build a fresh hash; do not mutate the inputs.
- Iterate `base.keys()` or the hash directly.
- Use `??` to fall back to `default_value` for nullish overrides.
- Return the hash implicitly.
