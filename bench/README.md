# Benchmarks

Every benchmark and eval harness lives here, one directory each.

| Directory | Measures |
|---|---|
| [`frameworks/`](frameworks/) | Soli against Rails, Express, Django, Laravel… on the same workloads — the numbers behind `www/docs/benchmarks.md` |
| [`cross-language/`](cross-language/) | Matched Soli vs Ruby micro-benchmarks with diffable output |
| [`micro/`](micro/) | Smaller Soli vs Ruby pairs for single operation families |
| [`engine-parity/`](engine-parity/) | Tree-walker vs VM output on the same expressions (run in CI) |
| [`soli-bench/`](soli-bench/) | How well a model writes Soli: `soli test` / `soli lint` as the grader |
| [`agents/`](agents/) | The same coding agent builds the same apps on Soli, Rails and Next.js; hidden HTTP checks grade them |

Not here: [`../benches/`](../benches/) holds the Rust `cargo bench` targets (Criterion),
where Cargo expects them.
