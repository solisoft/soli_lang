# Engine parity sweep

Runs one expression at a time through both engines and reports any whose output
differs. `differential_engines_test.rs` pins specific programs as a regression
gate; this is the wide net you cast when looking for *new* divergences.

```bash
./bench/engine-parity/sweep.sh < bench/engine-parity/expressions.txt
```

Span columns are normalised away, so an identical error reported at a different
offset is not counted — the engines legitimately differ there.

This only became possible once `soli -e` started honouring `--vm`. Before that
both sides of every comparison ran the tree-walking interpreter, which is how
several divergences survived: a hash entry holding a function could not be called
under the VM, `inspect` dropped the quotes on nested strings, `hash.to_s()`
existed in only one engine, and `[].pop()` raised in one and returned null in the
other.
