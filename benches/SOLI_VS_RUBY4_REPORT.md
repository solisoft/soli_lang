# Soli vs Ruby microbench report

| | |
|---|---|
| **Date (UTC)** | 2026-08-06T07:58:12Z |
| **Host** | `Linux 6.8.0-134-generic x86_64` |
| **Soli** | `Soli 1.28.0` (`/home/olivier.bonnaure@delupay.com/workspace/soli/lang/target/release/soli`) |
| **Ruby** | `ruby 4.0.6 (2026-07-14 revision 03b6d3f889) +PRISM [x86_64-linux]` |
| **Repeats** | 7 (median reported) |
| **Soli path** | release binary, `soli <script.sl>` |
| **Ruby path** | MRI 4.x, process wall time / self-reported ms |

Lower is better. **Speedup** = Ruby_ms / Soli_ms (>1 means Soli is faster).

## Whole-program wall time

Times include process startup + parse/compile + run for both sides.
Ruby MRI process boot is ~30–35 ms on this host, so short programs are
startup-dominated on the Ruby side; Soli starts faster.

| Benchmark | Soli (ms) | Ruby 4 (ms) | Speedup |
|---|---:|---:|---:|
| `array_ops` | 6.7 | 34.6 | 5.20× |
| `hash_ops` | 6.5 | 37.6 | 5.81× |
| `string_ops` | 6.5 | 39.9 | 6.11× |
| `loop_sum` | 8.4 | 34.6 | 4.11× |
| `fib_iterative` | 6.2 | 35.6 | 5.78× |
| `fib_recursive` | 12.9 | 35.2 | 2.72× |
| `class_ops` | 6.2 | 34.4 | 5.52× |
| `inheritance_deep` | 7.6 | 34.9 | 4.57× |
| `pipeline_ops` | 7.0 | 35.2 | 5.03× |

## JSON (self-timed loops, excludes process startup)

Each program times only the parse/stringify loops (same iteration counts).

| Benchmark | Metric | Soli (ms) | Ruby 4 (ms) | Speedup |
|---|---|---:|---:|---:|
| `json_ops` | stringify | 4.9 | 6.0 | 1.23× |
| `json_ops` | parse | 10.9 | 8.0 | 0.74× |
| `json_ops_large` | stringify | 4.9 | 10.3 | 2.11× |
| `json_ops_large` | parse | 15.4 | 12.0 | 0.78× |

## Notes

- **JSON** numbers are pure loop time (best comparison for the recent JSON work).
- **Whole-program** times include Soli parse+VM compile vs Ruby MRI boot; short
  programs are heavily startup-dominated on MRI (flat ~35 ms floor here).
- Ruby uses stdlib `JSON` (C extension, very mature). Soli uses hand-rolled
  `parse_json` + sonic-rs stringify.
- On this run, **stringify is faster on Soli**; **parse is still faster on Ruby**.
- Array/hash/string ops are language-level loops over built-ins, not C microkernels.
- Recent Soli work in this tree: one-pass JSON parse, join/string/hash to_string,
  EcoString identity methods, owning model JSON conversion.

## How to reproduce

```bash
mise install ruby@4.0.6
cargo build --release
RUBY_BIN=$(mise exec ruby@4.0.6 -- which ruby) REPEATS=7 ./scripts/compare_ruby.sh
```
