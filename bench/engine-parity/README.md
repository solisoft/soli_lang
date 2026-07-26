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

Both scripts currently report zero.
