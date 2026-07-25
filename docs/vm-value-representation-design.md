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

## Where the evidence actually points — measured

Roughly 27 cycles per opcode, far above the 2–3 cycles of real work. That is not
representation, so it was tested directly, without a profiler: hold the arithmetic
constant and vary only the number of dispatches per loop iteration.

```
adds/iter      ms      ns/iter   marginal
    1        7.946       39.7
    2       11.408       57.0     +17.3
    3       13.978       69.9     +12.9
    4       16.757       83.8     +13.9
```

Each added statement is a *fused, in-place* add — about one cycle (~0.3 ns) of real
work. Its measured marginal cost is **14.7 ns, roughly 45x the work it performs.**

**The cost is per-dispatch, not per-unit-work.** ~44 cycles per opcode is consistent
with an indirect-branch misprediction (~20 cycles) plus poor instruction-cache
locality across a ~300-arm `match`.

This also explains why NOP compaction was the session's largest win (−26.7%): it
removed dispatches, which is the thing that costs.

## Consequences for the goal

Ruby ZJIT completes an entire loop iteration in ~13 ns — less than **one** Soli
dispatch. Beating it therefore requires attacking dispatch, and only two things do:

1. **Reduce cost per dispatch.** Threaded dispatch is the classic fix. Rust has no
   computed `goto`; the options are tail-call dispatch (`become`, unstable) or a
   function-pointer table, neither of which LLVM guarantees to compile as intended.
   A cheaper first probe: **split the dispatch `match` into a small hot inner match
   (~20 opcodes) with a cold fallback**, for instruction-cache locality and better
   prediction. Contained to one function — nothing like 9,023 sites — and directly
   testable against the table above.
2. **Reduce number of dispatches.** Super-instructions, which is what the peephole
   already does. Beware the failure mode: fusing an entire loop shape keyed to a
   benchmark would make `int_loop` look native without making Soli faster. That is
   metric-gaming, not optimisation.

Note what this rules out: shrinking `Value` does not reduce dispatch count or
dispatch cost. It would reduce bytes moved per push/pop — real, but second-order
against a 45x work-to-overhead ratio, and paid for by heap-allocating short strings.

## Tried: profile-guided optimisation — makes it worse

PGO is the obvious "free" test of the branch-layout / I-cache hypothesis: no source
changes at all. It was tried end to end (instrumented build, profile collected from
`bench_all.sl`, `llvm-profdata merge`, `-Cprofile-use` rebuild).

**Result: `int_loop` −15.2%, won 0/6 rounds. Consistently slower.**

The PGO binary is 154 MB against 64 MB for the plain release build. That bloat is a
plausible cause in itself — it works against the instruction-cache locality the
hypothesis is about. Whatever the mechanism, "just turn on PGO" is a regression here,
not a win, and it should not be the first thing the next attempt reaches for.

Two practical notes for anyone retrying it: `-Cllvm-args=...` alongside
`-Cprofile-use` breaks the `openssl-sys` and `libwebp-sys` build scripts, and the
instrumented run leaves ~105 `.profraw` files (~59 MB merged) that need cleaning up.

## Tried: hot/cold dispatch split — also slower

The note previously proposed this as the cheap probe worth doing. It was done.

`run_dispatch` had ~140 arms inline. 50 of them — every arm that is neither hot nor
steers the loop with `return`/`break`/`continue` — were moved into a
`#[cold] #[inline(never)] fn dispatch_cold`, cutting 467 lines out of the hot
function.

**Result: `int_loop` −3.9%, won 1/6 rounds. Slower.**

Correctness held (2501 tests, 151 specs, engine parity), so this is a clean
negative: shrinking the hot function's instruction footprint does not help. Taken
together with the PGO result, neither instruction-cache locality nor branch layout
explains the ~44 cycles per dispatch.

## Suggested next step

Six hypotheses have now been tested and killed by measurement: optional-let
demotion, drop glue, `Value` size, the frame-fetch bounds check, PGO, and the
hot/cold dispatch split. What survives is the number itself — **~44 cycles per
dispatch against ~1 cycle of work** — and no cheap explanation for it.

The remaining candidates both change the shape of the VM rather than tune it:

* **Threaded dispatch.** Rust has no computed `goto`; tail-call dispatch (`become`)
  is unstable and a function-pointer table is not guaranteed to compile as
  intended. Worth a spike, but it is a spike, not a tweak.
* **A JIT.** This is what ZJIT is, and it is why an entire Ruby iteration fits
  inside one Soli dispatch.

Both are product decisions about what Soli's runtime should be, not optimisation
tasks. Nothing smaller than that moved the number, and this note records the six
attempts so the next person starts from evidence instead of the same intuitions.
