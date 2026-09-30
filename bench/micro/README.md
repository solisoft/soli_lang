# Micro-benchmarks (Soli vs Ruby)

Four matched pairs, one operation family each: collections, hash methods (global and local
receiver), string interpolation. Each script times its blocks and prints the results.

```bash
soli bench/micro/bench_interpolation.sl
ruby bench/micro/bench_interpolation.rb
```

For the full matched suite with diffable output, see [`../cross-language`](../cross-language).
