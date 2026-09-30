---
description: Run one spec (fast) or the full suite with coverage. Argument is a spec path or "all".
argument-hint: <spec-path|all>
---

If `$1` is empty or equal to `all`, run:

```
soli test --coverage --coverage-min 90.0
```

Otherwise run:

```
soli test $1
```

Surface failures verbatim — do not summarize them away. If the run is green, report which spec(s) ran and the coverage percentage.
