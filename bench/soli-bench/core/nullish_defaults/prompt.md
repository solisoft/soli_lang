# nullish_defaults

Write three small functions, all using the nullish-coalescing `??` operator.

1. `with_default(x, default)` — return `x` if non-null, else `default`.

2. `coalesce(a, b, c)` — return the first non-null value among `a`, `b`, `c`.

3. `safe_lookup(hash, key)` — return `hash[key]` if the key exists, else `"missing"`.
   - **Do not** use a `has_key` check — let `??` carry the fallback.

```soli
with_default(null, 0)        // => 0
with_default(42, 0)          // => 42
coalesce(null, null, "c")    // => "c"
coalesce(null, "b", "c")     // => "b"
coalesce("a", "b", "c")      // => "a"
safe_lookup({"a": 1}, "a")   // => 1
safe_lookup({}, "x")         // => "missing"
```

**Idiomatic touches we want to see**
- The `??` operator in all three.
- For `coalesce`, chain it: `a ?? b ?? c`.
- Bare assignment when the type is obvious.
