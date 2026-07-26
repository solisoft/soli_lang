# Engine parity sweep

Runs one expression at a time through both engines and reports any whose output
differs. `differential_engines_test.rs` pins specific programs as a regression
gate; this is the wide net you cast when looking for *new* divergences.

```bash
./bench/engine-parity/sweep.sh < bench/engine-parity/expressions.txt
```

Span columns are normalised away, so an identical error reported at a different
offset is not counted — the engines legitimately differ there.

Keep the corpus deterministic. Anything random (`sample`, `shuffle`, `DateTime.now`)
differs between *runs*, not between engines, and only produces false positives.

This only became possible once `soli -e` started honouring `--vm`. Before that
both sides of every comparison ran the tree-walking interpreter, which is how
several divergences survived: a hash entry holding a function could not be called
under the VM, `inspect` dropped the quotes on nested strings, `hash.to_s()`
existed in only one engine, and `[].pop()` raised in one and returned null in the
other.

## The second axis: `soli check` versus the runtime

`checkscan.sh` asks a different question of the same kind of corpus — does the
type checker reject a call the runtime accepts?

```bash
./bench/engine-parity/checkscan.sh < bench/engine-parity/arity-probes.txt
```

Both engines can agree perfectly and still leave a method unreachable, because a
declared arity that is narrower than the implementation turns a working call into
a check-time error. That axis found eleven such methods in one cycle: `count`,
`index_of`, `scan`, `partition`, `rpartition`, `get`/`fetch` with a default,
`center`/`ljust`/`rjust`/`lpad`/`rpad` with a pad string, `truncate` with an
omission marker, and `squeeze` with a character set.

It also finds the opposite problem — an argument the runtime *accepts and
ignores*, which is worse than one it rejects. `"ff".to_i(16)` returned 0 instead
of 255 for exactly that reason.

**A `CHECKER` hit is a candidate, not a verdict.** The probe corpus is generated
mechanically with stand-in arguments, so a rejection can equally mean the probe
passed the wrong *type* — `{"a": 1}.has_value?("x")` is a genuine type error on a
`Hash<String, Int>`, not a missing declaration. The script now filters out rejections whose message is a type
mismatch, which removes most of that noise, but confirm each hit by hand before
widening a declaration: loosening one to `Type::Any` to silence a bad probe makes
the checker weaker, which is the opposite of the point. Three of the seven hits in
the first generated run were exactly this, and `cargo clippy` caught them as
unreachable duplicate match arms.

Both scripts currently report zero actionable findings.

## The third axis: can hostile input panic?

`panicscan.sh` runs deliberately awful arguments — negative widths, out-of-range
indices, zero chunk sizes, `i64::MIN`/`MAX` — through both engines and reports
anything that reaches a Rust panic rather than a Soli error.

```bash
./bench/engine-parity/panicscan.sh < bench/engine-parity/hostile-inputs.txt
```

Release builds unwind and a per-request guard turns a panicking handler into a
500, so a panic is no longer fatal to the process — but it is still never the
intended way to report bad input, and it aborts whatever was in flight.

Currently zero panics across 47 expressions in both engines, and the two agree on
every one. It did surface something the other two axes cannot see, because it is
neither a divergence nor a checker gap: integer arithmetic wraps silently on
overflow, so `9223372036854775807 + 1` is negative and `2.pow(64)` is `0`. Filed
as `tasks/todo/integer-overflow-wraps-silently.md` — it is a language-level
decision, not a bug fix.

## Running in CI

All three run on every push, as the `engine-parity` job in
`.github/workflows/ci.yml`. Each exits non-zero on a finding, which was worth
checking rather than assuming: `sweep.sh` originally only printed a count and
always exited 0, so as a gate it could have been green forever. All three were
then verified to go red against a planted finding, using a stub binary that
fakes the exact signature each one hunts for.
