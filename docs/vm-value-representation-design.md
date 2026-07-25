# Design note: should `Value` be tagged/NaN-boxed?

**Recommendation: no — not as the next step, and probably not at all in this form.**

This note exists because "shrink `Value`" was the obvious answer to Soli's remaining
gap against Ruby ZJIT on interpreted loops, and the design phase found it is the
wrong lever. Measurements, not estimates.

## The gap being addressed

| | per iteration of a bare `while` loop |
|---|---:|
| Soli VM | ~45 ns |
| Ruby 4.0.6 ZJIT | ~13 ns |

Every per-element loss traces here: Numeric 3.8x, calls 3.8x, Hash 2.2x.

## What forces `Value` to 24 bytes

| payload | size |
|---|---:|
| `DecimalValue` | **20 B** |
| `SoliStr` (`ecow::EcoString`) | **16 B** |
| every `Rc`/`Arc` | 8 B |
| `NativeFunction` | 8 B |

20 B + tag, 8-byte aligned → 24.

**Boxing `Decimal` alone does not help**: `SoliStr` at 16 B still rounds the enum to
24. Reaching 8 B requires boxing `SoliStr` too — which puts every short string back
on the heap, because `EcoString`'s entire value is inlining strings ≤ 15 bytes.

String is Soli's **strongest** category — 0.37x geometric mean, i.e. 2.7x faster
than Ruby, with `capitalize` at 15x and `replace_all` at 7x. That lead rests on
exactly the inlining a one-word `Value` would remove. The refactor trades the
best category to help the worst.

## The drop-glue argument is false

The stated case for tagging was that five `Value` variants hold an `Rc`, so every
stack slot carries drop glue, against Ruby's plain-word `VALUE`.

Measured directly: `AddLocalsInPlace` was changed to mutate the `i64`/`f64` payload
in place, skipping both the discriminant write and the drop of the old value on the
hottest arithmetic path in the VM. Result: **−1.8%, won 2/6 rounds.** No benefit.

And tagging would not remove it regardless — pointer variants still need `Drop`.
Ruby avoids refcounting through **tracing GC**, which is a different and far larger
change than representation.

## Cost of doing it anyway

9,023 `Value::` references across 188 files. Unlike the `NativeFn` slice change
(16 compiler errors, because closure signatures are inferred), every one of these
is a pattern match that a tagged word cannot support — all 9,023 become accessor
calls.

## Where the evidence actually points

Roughly 27 cycles per opcode at ~3 GHz. That is far above the 2–3 cycles of real
work per instruction, and neither representation nor drop explains it. The
remaining candidate is the **indirect branch in the dispatch `match`** — the
classic interpreter bottleneck, addressed by threaded dispatch (computed goto /
tail-call dispatch), not by shrinking values.

Caveat, and the reason this is a note rather than a plan: one cheap probe in that
direction — `get_unchecked` on the per-instruction frame fetch — measured **+0.4%,
4/8 rounds, i.e. noise**. That removes a *bounds check*, not the indirect branch,
so it does not refute the dispatch hypothesis, but it is not evidence for it
either.

## Suggested next step

Before committing to any large change, **profile**. `perf` is blocked on the
current machine (`perf_event_paranoid=4`); on a machine where it works, a single
profile of `bench/cross-language/bench_all.sl` under `--vm` would settle whether
the cost is branch mispredictions in dispatch. That is a few hours of work and it
determines whether the answer is threaded dispatch, a JIT, or nothing worth doing.

Committing to a 9,023-site refactor without that profile would be guessing — and
two of the three hypotheses examined here were already wrong.
