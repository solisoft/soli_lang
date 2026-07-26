# Cross-language benchmarks (Soli vs Ruby)

Matched micro-benchmarks over Array, Hash, String, Numeric and Control-flow operations.
Both scripts print `category|operation|best_ms` so the outputs can be diffed directly.

```bash
soli --vm bench_all.sl            # Soli, production engine
ruby      bench_all.rb            # Ruby, interpreter
ruby --yjit bench_all.rb          # Ruby + YJIT
ruby --zjit bench_all.rb          # Ruby + ZJIT (Ruby 4)
```

Rules the two files follow, so the comparison stays fair:

* identical input sizes and identical data (`n = 20_000`; an 8,000-word string),
* best-of-7 timings, warmed once before measuring,
* like-for-like operations — notably `String.concat_plus` uses `r = r + "x"` on **both**
  sides. (An earlier draft compared Ruby's mutating `<<` against Soli's allocating `+`,
  which made Ruby look 5.5x faster when like-for-like Soli is ~5x faster. Don't do that.)
* Ruby is credited with its **best** mode, not its slowest.

Results are published at `/docs/getting-started/benchmarks`; regenerate that page's numbers
from these outputs rather than editing them by hand.

## Regenerating

`render_docs.py` rewrites the table bodies in both documentation surfaces. It needs all
four engines, **including Ruby 4 with `--zjit`** — the published page compares against
Ruby's best mode, and re-running with an older Ruby swaps the engine under the comparison
and makes the page *less* accurate rather than more. If Ruby 4 is not available, leave the
page alone: a stale-but-consistent comparison beats one whose two halves were measured
against different interpreters.

The Soli side alone can be checked at any time by diffing two `soli --vm bench_all.sl`
runs, which is enough to catch a regression from a compiler change without touching the
published numbers.
