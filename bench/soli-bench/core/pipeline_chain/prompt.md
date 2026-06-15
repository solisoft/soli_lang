# pipeline_chain

The point of this task is to write the body in **pipeline form**, not nested
calls. Rewrite the following chained transformation as a single `|>` pipeline:

```soli
# Equivalent (don't write this — write the pipeline form):
# let result = filter(map([1, 2, 3, 4, 5, 6, 7, 8, 9, 10], fn(x) x * 2), fn(x) x > 5);
#
# Required behaviour:
#   - take 1..10,
#   - multiply each by 2,
#   - keep only those > 5,
#   - sum the result.
#
# Implement `piped_sum()` that returns the sum, and `piped_steps()` that
# returns the intermediate array (so we can assert on it).
```

```soli
piped_sum()     // => 80   (6+8+10+12+14+16+18+20 - wait: 6+8+10+12+14+16+18+20 = 104;
                 //   but only those > 5 after the double: 2,4,6,8,10,12,14,16,18,20
                 //   filter > 5: 6,8,10,12,14,16,18,20  sum = 104)

piped_steps()   // => [2, 4, 6, 8, 10, 12, 14, 16, 18, 20]
```

**Idiomatic touches we want to see**
- A `|>` chain with `map` and `filter`.
- `sum` (or a manual reduce) for the final stage.
- Use `[1..10]` or `range(1, 11)` for the source — your call.
