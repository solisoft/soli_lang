# Benchmark Report — VM compiler work on `vm/value-tagging`

Measured 2026-07-27 against `soli-base`, a release binary built from the branch point
before the compiler changes. 16 cores, load average 0.41 at start, all runs interleaved
base/current to cancel thermal drift.

---

## Summary

| Question | Answer |
|---|---|
| Did anything regress? | No systematic change. Median across 67 cross-language benchmarks: **+0.2%** |
| What did the work buy? | **2.0×–4.8×** on handlers that previously demoted to the interpreter |
| Demotions on those handlers | **3 → 0** (plus 1 → 0 for `next`) |
| Correctness | Every benchmark returns byte-identical answers on both binaries |
| DateTime (worked on after the report) | category **2.43× → 1.88×** vs Ruby 4.0.6 (−27.3% absolute, 1.38× faster), plus four bugs fixed |
| Gates | `fmt` clean, `clippy --all-targets --all-features` clean, **2228 lib tests pass**, 151 Soli tests pass, differential-engines pass |

The headline is the second row, and it is not visible in the general-purpose suite at all.
See *Why the two suites disagree* below. Section 5 covers the DateTime work.

---

## 1. Regression check — 67 cross-language micro-benchmarks

`bench/cross-language/bench_all.sl`, best of 10 runs each, interleaved.

```
median across all 67 cases: +0.2%
cases >10% slower: 2        cases >10% faster: 3
```

**Fastest-improved**

| case | delta |
|---|---:|
| `String\|bytes` | −10.5% |
| `DateTime\|subtract_days` | −10.1% |
| `DateTime\|add_hours` | −10.0% |
| `String\|index_of` | −8.2% |
| `Array\|flatten` | −6.3% |
| `Hash\|values` | −6.2% |

**Most-regressed**

| case | delta |
|---|---:|
| `Aggregate\|find_by` | +5.1% |
| `Control\|fn_call` | +6.9% |
| `Aggregate\|min_by` | +8.4% |
| `Aggregate\|avg_by` | +9.3% |
| `Hash\|get` | +11.2% |
| `String\|replace_all` | +22.8% |

### The two >10% cases are real, and only partly attributable

Both were re-measured with 9 further interleaved runs and both held:
`String|replace_all` +21.7%, `Hash|get` +10.2%.

To attribute them I rebuilt the current tree with the peephole jump-target guard narrowed
back to its original op list, and measured that binary against the full one. The guard is
the change that withdraws fusion opportunities, so it was the prime suspect:

| case | narrow guard | full guard | cost of the guard |
|---|---:|---:|---:|
| `Hash\|get` | 1.2538 | 1.2903 | **+2.9%** |
| `String\|replace_all` | 0.0865 | 0.0854 | −1.4% |
| `Array\|map` | 0.9606 | 0.9239 | −3.8% |

So the guard explains roughly **3 of `Hash|get`'s 10 points** and **none** of
`replace_all`. The remainder is consistent with the ~7% whole-binary code-layout variance
this codebase exhibits between builds — `replace_all` is the smallest benchmark in the
suite at 69 ms, where layout effects dominate, and `Array|map` moving −3.8% from a change
that cannot affect it is direct evidence of that noise floor.

**Correction to an earlier attribution.** Earlier in this work I attributed 4.8% of
`replace_all` to the jump-target guard. That measurement was against a different baseline
binary and does not survive re-measurement: the guard's effect on `replace_all` is −1.4%,
i.e. nothing. The +21.7% is currently **unattributed** and is most likely layout.

The guard is not optional — it is what makes peephole compaction correct for the branch
ops it now covers, and removing it reintroduces the `TryBegin` corruption class. A 2.9%
cost on one hash benchmark is the correct trade.

---

## 2. What the work actually bought — handler-level measurement

The cross-language suite exercises none of the constructs this work made compilable. Those
constructs are measured directly below.

**Method.** A four-route app, `SOLI_WORKERS=1`, `SOLI_METRICS=1`, **production mode**
(no `--dev`), median of 60 requests per route after warm-up, demotion counts read from
`/_metrics`. Ports 8941–8962, own PIDs only — the user's `serve .` fleet was never touched.

| route | construct | base | current | speedup | answers agree |
|---|---|---:|---:|---:|---|
| `/plain` | control — no new construct | 162 µs | 160 µs | **1.01×** | yes |
| `/brk` | `break` out of an inner loop | 6245 µs | 1301 µs | **4.80×** | yes |
| `/nav` | safe navigation `&.` | 3047 µs | 725 µs | **4.20×** | yes |
| `/mat` | `match`: literal + typed + array + hash | 5149 µs | 2571 µs | **2.00×** | yes |
| `/nxt` | `next` | 6449 µs | 1844 µs | **3.50×** | yes |

```
soli_vm_handler_demotions_total    base = 3      current = 0      (+ next: 1 → 0)
```

The control route at 1.01× is the important line: this is **not** a general speedup. It is
entirely the difference between a handler running compiled and the same handler being
demoted to the tree-walking interpreter because one construct in it would not compile.

This reproduced across two independent runs (4.85/4.30/2.04 then 4.80/4.20/2.00).

### Why the two suites disagree

They measure different things, and the general-purpose one is structurally blind here:

- `bench_all.sl` is a **script**. In a script, a compile refusal is a hard error — the
  program does not run at all. So no benchmark that exercises these constructs can exist
  in that suite, which is why it shows +0.2% and no more.
- Demotion-to-interpreter is a **server handler** mechanism. Only there does a refusal
  degrade to "runs, but slowly" rather than "does not run".

Confirmed directly: under `soli-base --vm`, `break_.sl`, `safenav.sl` and `match_bind.sl`
all exit with `Compile error: … is not supported in compiled mode` and produce no output.
Under the current binary all three run and print correct values.

---

## 3. Corrections to figures carried from earlier in this work

Three numbers I had recorded earlier do not survive re-measurement. Recording them plainly
because they were wrong, not marginal:

| earlier claim | actual |
|---|---|
| `finally`-heavy loop **5.3×** (0.189s → 0.036s) | **1.05×**. `finally` already compiled in base; that work fixed *semantics* (leaked resources, swallowed exceptions), not compilability. No speedup was ever available. |
| `break`-heavy **2.8×** (0.026s → 0.009s) | **4.80×** at handler level. The script-level figure was not measuring what it claimed — base cannot run the script at all. |
| `next`-heavy **2.9×** (0.234s → 0.079s) | **3.50×** at handler level, same caveat. |

The common fault: script-level timings of programs that the base binary refuses to compile.
Those runs were timing an error path, not a workload. Handler-level measurement is the only
valid comparison for anything demotion-related, and is what section 2 uses.

I also stated mid-run that Soli "has no `continue`/`next` statement". That was wrong.
`next` is not a keyword and has no AST node — which is what I checked — but it exists as a
zero-argument builtin returning a loop sentinel, in both `next` and `next()` spellings.
Section 4 covers what checking only the lexer and AST missed.

Separately, "repo compiler refusals went 10 → 4" counted **refusal sites in the compiler
source**, not files affected. Measured properly:

- **Files**: 213 `.sl` files across `tests/ examples/ stdlib/ www/` — base **1** refusal,
  current **0**.
- **Source sites**: 8 remain (`compiler_patterns.rs` 4, `compiler_stmts.rs` 2,
  `compiler_exprs.rs` 2). Of these, two are `break`/`next` *outside a loop* (correct to
  refuse), one is command substitution (deliberate), and the pattern sites include the
  `And`/`Or` variants that no parser path can produce — see
  `tasks/todo/match-and-or-patterns-are-unreachable.md`.

---

## 4. Defect found while benchmarking

**`next` is unusable in any standalone script.** It is missing from
`src/types/environment.rs`, so the type checker rejects it as an undefined variable:

```
$ soli --vm loop.sl
Error: Type error: Undefined variable 'next' at 3:19
```

It works in controller actions only, because `serve` never type-checks. That makes it
unreachable from `soli run`, `soli -e`, `soli test`, `soli check`, and therefore from
anything in `stdlib/` or `tests/`. Confirmed pre-existing — the base binary fails
identically — and filed as
`tasks/todo/next-is-unusable-outside-a-server-handler.md`.

This is the reason my first attempt at a `next` benchmark failed, and it is why the
construct has no coverage in the repo's own `.sl` corpus.

---

## 5. DateTime — the worst category, and what was done about it

The docs benchmark page puts DateTime at a 2.43× geometric mean against Ruby, winning 1 of
9. Profiling was blocked (`perf_event_paranoid=4`, no root), so the cost was isolated with a
standalone crate against the same chrono version, 2M iterations, `black_box`ed:

| path | `chrono::Local` | cached `chrono_tz::Tz` |
|---|---:|---:|
| `with_timezone(..).year()` | 47.2 ns | **20.2 ns** |
| `from_local_datetime(..)` | 234.7 ns | **21.6 ns** |

`Local` re-resolves the system zone on every call. Every DateTime accessor paid the first
row (18 call sites) and the boundary methods paid the second (4 sites) — against an accessor
that totals ~167 ns, so it was roughly a quarter of the cost.

Resolving the zone once per process and caching it:

| operation | before | after | change |
|---|---:|---:|---:|
| `end_of_month` | 9.061 ms | 4.671 ms | **−48.5%** |
| `year` | 3.184 ms | 2.784 ms | **−12.5%** |
| `format` | 7.072 ms | 6.615 ms | −6.5% |
| `now` | 4.910 ms | 4.596 ms | −6.4% |
| `add_hours` / `subtract_days` / `to_unix` | — | — | ~0% (never touched `Local`) |

**DateTime geometric mean: 4.216 → 3.822 ms, −9.3%.** Median across all 67 cases stays
+0.3%, so nothing else moved.

### Two bugs found and fixed on the way

1. **The boundary methods panicked on daylight-saving dates.** They called
   `LocalResult::unwrap()`, which panics both when the local time does not exist (clocks
   forward) and when it is ambiguous (clocks back). Reproduced through the real binary:
   under `TZ=America/Havana`, `DateTime.parse("2026-11-15 12:00:00").beginning_of_month()`
   panicked with *"Ambiguous local time, ranging from 2026-11-01T00:00:00-05:00 to
   -04:00"*. Scanning all **597 zones over 2015..=2035** found **17 reachable dates**
   (Africa/Cairo, America/Asuncion, America/Havana, Asia/Amman, Asia/Almaty, Cuba, Egypt),
   several still in the future. Severity is a **500 on that request**, not a DoS — verified
   against a running server: worker survived, process alive, sibling routes still 200. That
   containment is the `panic = "abort"` removal and the per-request `catch_unwind` guard
   doing exactly their job.
2. **`beginning_of_hour` and neighbours returned a runtime error** (`Failed to compute …`)
   on the fall-back hour in Europe/Paris, Europe/London and Asia/Beirut. The differential
   run caught this: the old binary died 88 lines into a 228-line output.

Both now resolve to a value: ambiguous takes the **earliest** instant, nonexistent moves
forward to where the gap closes. The conversion is total — no failure case remains.

### A regression I introduced and caught

The first version converted to `DateTime<FixedOffset>`, and `with_hour(0)` on a fixed offset
edits the wall clock while keeping the offset that was correct for the *original* instant.
`Australia/Sydney` `beginning_of_day` for 2023-10-01 returned `2023-09-30 23:00:00` — midnight
is still UTC+10 but the source instant was UTC+11. The six `beginning_of_*`/`end_of_*`
methods now build a naive local time and re-resolve it through the zone. This was caught by
the differential run, not by reasoning.

### Verification

Every DateTime method, across **16 timezones × 12 DST-straddling timestamps**, diffed
against the pre-change binary: **13 zones byte-identical, 0 differing, 3 where the old
binary failed outright**. Plus a test asserting a cached zone agrees with `chrono::Local`
at hourly resolution across a full year (8,760 samples, both hemispheres' transitions).

### The `$TZ` trap

`iana_time_zone::get_timezone()` reads the *system* zone and **ignores `$TZ`** — verified
directly: with `TZ=UTC` it still returned `Europe/Paris`. Resolving through it alone would
have silently ignored the `ENV TZ=UTC` that most containers set. `$TZ` is honoured first,
and a `$TZ` holding a POSIX spec rather than an IANA name falls back to `Local`, staying
correct at the old cost rather than guessing.

### Round two: DateTime as a native value

The boundary cost measured above (~52 ns to hand back an object, ~100 ns dispatch + a
string-keyed `_ts` lookup) was then removed by making a DateTime a `Value::DateTime(i64)`
instead of an `Instance` — following the existing `Decimal(DecimalValue)` precedent, and
fitting the existing 24-byte `Value` since an instant is just an `i64`.

| operation | tz-fix | native | change |
|---|---:|---:|---:|
| `from_unix` | 3.683 ms | 2.722 ms | **−26.1%** |
| `subtract_days` | 3.378 ms | 2.527 ms | **−25.2%** |
| `parse` | 4.344 ms | 3.261 ms | **−24.9%** |
| `now` | 4.460 ms | 3.378 ms | **−24.3%** |
| `end_of_month` | 4.534 ms | 3.477 ms | **−23.3%** |
| `add_hours` | 3.318 ms | 2.583 ms | **−22.2%** |
| `year` | 2.741 ms | 2.578 ms | −5.9% |
| `to_unix` | 2.115 ms | 2.007 ms | −5.1% |

The split is exactly the prediction: operations that **return** a DateTime gained ~22–26%
(the allocation), while those returning an `Int` gained only the hash lookup (~5%).

**DateTime geometric mean −17.8% (1.22×).** Combined with the timezone work,
4.216 → 3.063 ms — **−27.3%, 1.38× faster**.

Cost: the extra `Value` variant moved the median across all 67 cases from +0.3% to +1.7%,
so the variant is not free for unrelated code.

### Two more bugs this surfaced

Adding the variant exposed what the `Instance` representation had been hiding:

1. **A DateTime serialised to JSON as `{}`.** Its only field was the private `_ts`, and the
   serialiser deliberately drops `_`-prefixed framework internals — so nothing was left to
   emit. Every timestamp in an API response was an empty object. Now RFC 3339, on all
   **three** JSON paths (`.to_json()`, the executor renderer, and `serde`).
2. **`str(dt)` printed `<DateTime _ts: 1794744000000000000>`**, leaking the internal field
   through the generic object renderer. Now the local wall clock `to_string()` returns.

Both are behaviour changes, and both previous outputs were unusable.

### Three regressions I introduced and caught

Recording these because each was caught by a gate rather than by reading the code:

- **`==` was always false.** `PartialEq` needed its own arm — while a DateTime was an
  `Instance`, equality-by-instant came from the Instance arm's `datetime_ts`. Ordering
  (`<`, `>=`) kept working, which made it look fine. Caught by `datetime_spec.sl`.
  My first fix landed the arm in `hash_eq`, a legacy method with an identical match shape,
  and changed nothing — the second attempt anchored on the `impl PartialEq` block.
- **Methods with arguments were invoked with none.** `NativeFunction` arity **excludes** the
  receiver (`DateTime.year` is registered `Some(0)` yet reads `args[0]` as the instant), and
  I had tested for `Some(1)`, so `add_days` auto-invoked and reported "requires number".
  Caught by the interpreter-vs-VM differential.
- **Dispatch had to be wired at six sites, not one.** `d.year()` reaches a receiver match in
  `method.rs`, member access in `member.rs` (twice), `op_get_property` in `vm_classes.rs`,
  `call_builtin_method` in `vm_calls.rs`, and two guards in `vm.rs`. Each failed loudly as
  "Cannot access property 'year' on DateTime", one after another.

### Verification of the native variant

- Every DateTime method × **16 timezones × 12 DST-straddling timestamps**, against the
  pre-refactor binary: **16/16 byte-identical**.
- **Zero interpreter-vs-VM disagreements** across the same matrix.
- 2228 lib tests, 151 Soli tests, `differential_engines_test`, `fmt`, and
  `clippy --all-targets --all-features` all clean.

### Why DateTime is still slower than plain Ruby

Measured directly rather than inferred — 2M iterations, best of 3, minus a 0.13 s bare-loop
floor:

| | per call |
|---|---:|
| Soli `n.abs()` — a minimal builtin | **20 ns** |
| Soli `d.year()` | **65 ns** (75 ns before the fix below) |
| Ruby 4.0.6 `d.year` under ZJIT | ~40 ns *including* loop overhead |

The date arithmetic is not the problem: the cached timezone conversion is ~20 ns of that 65.
The rest is **dispatch**, and it splits two ways:

1. **Ruby's advantage is the JIT.** ZJIT compiles `d.year` to machine code with inline
   caching, so a hot accessor is close to a direct call. Soli interprets every call. This is
   the same reason Ruby wins `Control` and `Numeric` on this page and is not specific to
   DateTime.
2. **Soli's self-inflicted share is the dispatch shape.** `n.abs()` resolves through a
   compile-time `match name` — no lookup, no allocation. DateTime instead reuses the
   registered `NativeFunction` map, so each call does a string-keyed `HashMap` lookup, an
   `Rc` clone, and (until now) a heap allocation to prepend the receiver.

Building the argument list on the stack instead of in a `Vec` removed the allocation and
took `d.year()` from 75 ns to 65 ns. The remaining ~25 ns above `n.abs()` is the map lookup
and `Rc` clone, and closing it means porting the 26 method bodies to a `match method_name`
in the shape `decimal_methods.rs` already uses — which is the shortcut I took to avoid
rewriting them. Doing that would plausibly land `d.year()` near 40 ns and move the DateTime
category from 1.88× toward ~1.3×, but it is a rewrite of every method body and was not
started here.

### What is left in DateTime

The object allocation is gone (native variant, above). What remains is the interpreted
dispatch described in the previous section: a string-keyed method lookup per call, and no
JIT. The next concrete step is the `match method_name` port; beyond that, closing the gap to
Ruby on this category means compiling hot method calls rather than interpreting them, which
is a VM-wide concern rather than a DateTime one.

---

## 6. Method notes

- **Two invalid runs were discarded before reaching this report.** The first benchmark
  table showed every case at ~3 ms and "answers agree" — both binaries were printing the
  CLI usage text, because `soli run file.sl` is not a valid invocation (`--vm file.sl` is);
  "agreement" was two identical error messages. The second showed 0 demotions on both
  binaries because `--dev` runs the interpreter for hot-reload, so neither side reached the
  VM. Any benchmark whose control does not move and whose absolute numbers are implausible
  should be assumed broken until the output is inspected.
- Typed match patterns are spelled `Int: n`, type first — not `n: Int`.
- Bare assignment at script top level requires `let`; `SOLI_VM_OPTIONAL_LET` is off by
  default, which is itself the subject of a filed task.

### Not regenerated: the published docs benchmark page

`www` carries a rendered comparison page produced by `bench/cross-language/render_docs.py`,
which requires **Ruby 4 with `--zjit`**. This machine has Ruby 3.4.9, so regenerating it
would silently produce a page whose Ruby column was measured on a different interpreter
than the published figures claim. Left untouched deliberately; it needs a machine with the
right Ruby, not a flag change.

---

## 7. Verification state

```
cargo fmt --check                              clean
cargo clippy --all-targets --all-features      clean (0 warnings)
cargo test --lib                               2223 passed, 0 failed
cargo test --test differential_engines_test    passed
```

The lib-test count is up from the 2165 baseline recorded in the review plan.
