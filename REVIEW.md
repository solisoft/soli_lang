# Soli — Code & Feature Review

**Reviewed:** v1.24.1 (`823b4f8d`) · **Date:** 2026-07-25
**Scope:** 418 Rust files / ~214k LOC in `src/`, 754 resolved crates, 797 commits in the
preceding three months.

---

## Summary

The engineering hygiene here is high, and this was verified rather than assumed:

| Gate | Result |
|---|---|
| `cargo clippy --all-targets --all-features` | clean |
| `cargo test --lib` | 2165 passed, 0 failed (1.22s) |
| `cargo audit` | clean; 1 yanked-crate warning, 10 waivers each with scope + un-block condition |
| `cargo fmt --check` | clean |
| CI | 8 jobs, incl. Windows portability, headless-browser e2e, veraPDF PDF/A-3b + PDF/UA-1 |

Defences that are genuinely in place and correctly built:

- **AQL injection** — `validate_field_name` (`model/core.rs:603`) is enforced at every
  user-reachable entry point (`order`, `select`, `pluck`, `group_by`, `aggregate`,
  `find_by`, `first_by`, `find_or_create_by`, `graph_rag`, columnar `query`); values go
  through bind-vars. Validating at the API boundary rather than at query-build time is the
  right call — it fails with a message naming the offending call.
- **Path traversal** — `resolve_static_file` canonicalises and root-checks, returning 403.
- **Backpressure** — bounded crossbeam channels (`workers × 64`), `503` when full, `504`
  at 40s for a wedged worker. Genuinely well-engineered.
- **SSRF** — the test-runner bypass requires a well-formed UUID v4, specifically so a
  stray `=1` in a deploy environment cannot disable the guardrail (`main.rs:49`).

So this review spends its time on the three places where the marginal value actually is: a
confirmed production-availability defect, the architectural tax, and the feature gaps.

---

## P0 — Fixed in this change

### P0-1 · `panic = "abort"` made every panic-containment net dead code

`Cargo.toml` set `panic = "abort"` under `[profile.release]`, and every shipped binary uses
that profile (`Dockerfile:4`, the CI release matrix, `cargo install --path .`).

Under `panic = "abort"`, `std::panic::catch_unwind` **can never catch** — the process
aborts at the panic site. That silently disabled both supervisors:

- `serve/mod.rs` — the web-worker restart loop (`"Worker {} panicked, restarting..."`)
- `serve/background_jobs.rs:131` — the job-pool restart loop

Both were written to contain a panic to one worker. In every released binary, **one panic
in any worker aborted the whole process and every other worker with it.** The exposure is
not theoretical: `src/` holds 1,854 `.unwrap()`, 436 `.expect(`, 354 `panic!`, and 63
`unreachable!`, much of it reachable from user-authored Soli through the builtins.

The codebase had already identified this class of bug — `finish_response` exists precisely
to avoid it and names `panic = "abort"` in its doc comment — but it was called at **2 sites
against 72 raw `.body(..).unwrap()`** in `src/serve/`. The fix was built and not adopted.

**Fixed by:**
1. Removing `panic = "abort"`, with a comment naming the three nets that depend on unwinding.
2. A **compile-time guard** (`serve/shutdown.rs`): `#[cfg(panic = "abort")] compile_error!`.
   Verified in both directions — the build is clean today, and re-introducing
   `-C panic=abort` fails the build with an explanatory message. A regression here cannot
   be silent again, which a test could not guarantee (`cargo test` never uses the release
   profile).
3. A **per-request `catch_unwind`** in the new `dispatch_http_request`. Previously a
   panicking handler never sent on `response_tx`, so the caller waited the full
   `RESPONSE_WAIT_TIMEOUT_SECS` (40s) for a `504` — and the worker was gone for good. Now
   the client gets an immediate `500`, the worker keeps serving, and
   `soli_handler_panics_total` is incremented on `/_metrics`. The handler-driven streaming
   block is guarded separately (its response head is already sent, so the stream is closed
   early rather than the worker lost).

Extracting `dispatch_http_request` also removed ~74 lines duplicated verbatim between
`worker_loop`'s two dispatch arms.

### P0-2 · SIGTERM dropped in-flight requests

`main.rs:15` installed a `sigaction` handler calling `std::process::exit(0)` immediately.
Hyper was never told to stop accepting and nothing was drained, so every rolling deploy,
container restart, or `systemctl restart` truncated whatever was being served.

A partial mechanism existed but was **never wired**: `shutdown_flag` was allocated and read
on every request (returning a `503`), but nothing ever set it to `true`. Dead scaffolding.

**Fixed by** `serve/shutdown.rs` plus a drain task: on `SIGTERM`/`SIGINT` the server marks
itself draining (so `/_ready` fails and the load balancer stops routing), stops accepting,
waits for in-flight connections to reach zero or for `SOLI_SHUTDOWN_GRACE_SECS` (default
25s, under Kubernetes' 30s default), then exits `0`. A second signal skips the wait.

One subtlety worth recording: tokio's signal driver *chains* to any handler already
installed, so `main.rs`'s immediate-exit handler would have fired before anything drained.
`spawn_drain_on_signal` resets `SIGTERM`/`SIGINT` to `SIG_DFL` first. The `atexit` coverage
flush that `main.rs` documents is preserved, because the drain path also ends in
`process::exit(0)`.

### P0-3 · No health or readiness endpoint

`/_metrics` was the only operational endpoint, so Kubernetes, ECS, and every load balancer
had nothing to probe — and nothing could be told to stop routing during a drain.

**Fixed by** two endpoints alongside `/_metrics`:

- `GET|HEAD /_health` → `200` for as long as the process serves, **including mid-drain**.
  Liveness. A draining process is healthy; failing here would tell the orchestrator to
  restart a container that is already shutting down cleanly.
- `GET|HEAD /_ready` → `200` when booted and not draining; `503 starting` before the worker
  pool is up; `503 draining` for the whole drain. Readiness.

Both are exempted from the drain's blanket `503` — a readiness probe that gets swallowed by
the drain check cannot do its one job.

---

## P1 — Recommended next

### Observability stops at counters

No `tracing`, no OpenTelemetry, no structured/JSON logs anywhere in the tree. `/_metrics`
covers Prometheus counters — including the genuinely thoughtful
`soli_vm_handler_demotions_total` — but there is no way to export a trace or ship parseable
logs. For a framework positioned against Rails and Phoenix in production, this is now the
largest operational gap. Note the internals already exist: `span_log.rs` builds a real span
tree per request for the dev bar; an OTel exporter is largely a matter of wiring it up.

### ~~The VM's fast path is off by default for idiomatic Soli~~ — measured, and largely wrong

**This was my P1 hypothesis. Measuring it disproved it, so it is downgraded rather than
carried forward.** Recording the correction because the reasoning was plausible and someone
will re-derive it otherwise.

The hypothesis: `SOLI_VM_OPTIONAL_LET` defaults to **off** (`vm/compiler.rs:33`), so a bare
assignment to an undeclared name compiles to `SetGlobal`, raises at runtime, and demotes the
whole handler to the interpreter. Since bare assignment is the form `CLAUDE.md` tells authors
to *prefer*, the documented-idiomatic style should be the style that falls off the VM.

**Measured** on `www/` — production mode, `SOLI_WORKERS=1` (demotions are cached per worker,
so more workers would multiply the count), `SOLI_METRICS=1 SOLI_ENGINE_LOG=1`, crawling all
167 parameterless GET routes:

| | |
|---|---:|
| Handlers warmed | 190 (+47 class methods) |
| VM executions | 153 |
| **Handler demotions** | **2** (~1.2%) |
| Demotions caused by optional-let | **0** |

The VM runs essentially all of it. The reason the hypothesis fails is already in the code:
`warm_vm_handlers` seeds the compiler with the worker's full set of global names, so bare
assignments inside handlers resolve local-vs-global exactly as the tree-walker would — the
comment there says so, and the measurement confirms it.

The optional-let gate is still worth closing eventually (it masks real local-assignment bugs
under `for`-with-index and `try`/`catch`), but it is a **footnote, not a performance item**.
Both demotions were unrelated failures — and finding them is what the measurement was
actually worth. See P2 below.

### God functions

| Function | Lines | Location |
|---|---:|---|
| `register_model_class` | **3,904** | `interpreter/builtins/model/core.rs:1218` |
| `handle_hyper_request` | 1,122 | `serve/mod.rs` |
| `run_hyper_server_worker_pool` | 993 | `serve/mod.rs` |
| `handle_request` | 922 | `serve/mod.rs` |
| `worker_loop` | 632 | `serve/mod.rs` |

(`vm.rs::run_dispatch` at 2,283 is a bytecode dispatch `match` — that one is fine as-is.)
`serve/mod.rs` is 8,704 lines. `register_model_class` registers the entire ORM surface as
inline closures in one function; it is the highest-value refactor in the repo and the reason
`core.rs` is 6,141 lines. Splitting it by concern (CRUD / query / associations / callbacks /
AI) is mechanical and low-risk.

### The dual-engine maintenance tax

Every builtin is implemented twice — `string_methods.rs` (1,553) beside `vm_string_methods.rs`
(931), `hash_methods.rs` (1,012) beside `vm_hash_methods.rs` (658). Adding one String method
touches six files. The mitigation is genuinely good: `differential_engines_test.rs` runs 46
programs through both engines with an explicit `KNOWN_DIVERGENT` list that fails on a *new*
divergence **and** on an un-removed fix. But the tax is structural and compounds with every
builtin. Worth a deliberate decision: keep paying it, or generate both dispatch tables from
one declarative source.

### `finish_response` adoption (deliberately deferred)

72 raw `.body(..).unwrap()` sites remain in `src/serve/` against 2 uses of the helper built
to replace them. I attempted the mechanical rewrite and **reverted it**: nested-paren cases
made a regex rewrite unsafe, and two sites were corrupted in a way that happened to be
caught by the compiler — the dangerous version is the one that compiles and is subtly wrong.
With the per-request `catch_unwind` now in place these are no longer a process-availability
risk, only a wasted request and a noisy log. Filed as a task to do with a proper
balanced-paren tool and site-by-site review.

---

## P2 — Two live bugs the VM measurement surfaced

Both found by crawling `www/` with `SOLI_ENGINE_LOG=1`; neither was visible from reading code.

### ~~`/docs/language/collections` returns 500 in production~~ — FIXED

A published docs route — `routes.sl:101` → `docs_controller.sl:324` `language_collections`
— rendered `docs/language/collections`, and **that template did not exist**.
`www/app/views/docs/language/` has `arrays.html.slv` and `hashes.html.slv` but no
`collections.html.slv`. It was the only 500 in a 167-route crawl.

A repo-wide grep found exactly two references to the path — the route and the action itself
— so nothing on the site linked to it: an orphaned route left behind when the page was split.
The language index confirms the intent, carrying a card labelled **"Collections"** that
points at `/docs/language/arrays`.

**Fixed** by making `language_collections` redirect to `/docs/language/arrays`, following
the convention already in the file (`builtins_websocket`, `redirect_*`): the route stays so
external inbound links and search results keep working, and the action redirects. Verified
`302 → /docs/language/arrays → 200`, and a re-crawl of all 167 routes now returns
**zero 5xx** (demotions also fell 2 → 1, the remainder being the `blog#index` VM bug below).

### `blog#index` diverges between the two engines

Every request to `/blog` demotes with:

```
[soli engine] handler 'blog#index' demoted to the interpreter:
    Cannot access property 'get' on string at 77:0
```

Line 77 is `let path = info["file"]` inside `for info in blog_info`, where `blog_info` is a
local array of hashes. The interpreter handles it (the route returns `200`); the VM believes
`info` is a `String`. Every request therefore pays a failed VM attempt plus a full
interpreter re-run.

Notably this is **not** an optional-let case — every binding in that function uses explicit
`let`, so it is a distinct VM bug, not the known gate. A minimal reconstruction (array of
hashes, `for`-in, string-key index) runs identically on both engines, so the trigger is
something more specific in that function; a standalone repro is blocked because
`get_blog_posts` calls `file_exists`/`slurp`, which are not available to `soli run`. This is
exactly the class of defect `differential_engines_test.rs` exists to catch, which makes it
worth a proper repro. Filed as `tasks/todo/vm-divergence-blog-index.md`.

## Collection performance — four quadratic array methods, now linear

Swept every array and hash method at n and 4n, so linear ≈ 4× and quadratic ≈ 16×. Five
measurements came back quadratic; everything else was clean.

| Method | n=1000 | n=4000 | ratio | after |
|---|---:|---:|---:|---:|
| `uniq` (ints) | 1.04 ms | 18.78 ms | 18.1× | **0.27 ms** |
| `uniq` (strings) | 2.54 ms | 32.68 ms | 12.8× | **0.20 ms** |
| `intersection` | 3.14 ms | 35.19 ms | 11.2× | **0.43 ms** |
| `union` | 2.24 ms | 36.74 ms | 16.4× | **0.27 ms** |
| `difference` | 1.10 ms | 17.08 ms | 15.5× | **0.19 ms** |

Cause: each tested membership with `Vec::contains` against its own output — O(n·k). The
severity curve was textbook, quadrupling per doubling: **1.0 → 4.6 → 18.7 → 75.0 → 295.4 ms**
for n = 1k…16k. A third of a second inside one method call. Now **0.05 → 0.11 → 0.23 → 0.42
→ 0.90 ms** — 327× faster at n=16000.

Two things worth recording:

- **The same operation existed four times** — the interpreter's borrowed fast path, its owned
  path, the VM, and the `Array` class. Fixing the shared helper changed nothing for
  `[...].uniq()`, because the borrowed fast path had its own inline copy; only re-measuring
  caught it. All four now call `array_ops.rs`, which exists for exactly this reason.
- **The hard part was equality, not the data structure.** `Value`'s `PartialEq` has
  cross-type numeric arms, so `[1, 1.0].uniq()` must yield one element; `-0.0 == 0.0` while
  their bit patterns differ; `NaN != NaN`, so every NaN must survive; and arrays/hashes
  compare structurally. A naive `HashSet` breaks all four silently. The implementation keys
  hashable scalars (normalising integral floats onto the integer key) and falls back to a
  linear scan for the rest, which is sound because `PartialEq`'s catch-all arm is `false` —
  no hashable value can equal an unhashable one. Each case has a test.

Hash methods were measured and are already right: `get`/`has_key` are **flat** in collection
size (`IndexMap` + ahash), and `merge` is linear — an initial 8.6× reading was noise off a
0.03 ms base and vanished at larger n.

## Soli VM vs Ruby 3.4.9 on collections — where the gap actually is

Matched benchmarks, n=20,000, best-of-7, idle box, Soli on the **VM** (production engine).

**Soli wins where the work is native Rust**: `sort` 6.4×, `flatten` 5.1×, `join` 2.7×, plus
`sum`, `hash merge`, `union`, `includes?`, `hash build` at 1.4–1.8×.

**Ruby wins wherever a Soli closure or loop runs per element**: `hash_get` 3.1×,
`hash_select` 2.4×, `hash_each` 2.3×, `transform_values` 1.9×, `array_map` 1.6×.

### The gap is not in the hash

Decomposing `hash_get` (20k iterations) on the VM:

| | per iter | share |
|---|---:|---:|
| empty loop | 43 ns | **51%** |
| one extra statement (any) | 26 ns | 29% |
| the hash lookup itself | 17 ns | **20%** |

**Ruby's entire loop-plus-lookup is 21 ns — less than Soli's empty loop alone.** Optimising
the hash cannot close this; the target is the VM's per-opcode cost.

### Why, and what would be required

The easy wins are already taken: `i = i + 1` and `i < n` are peepholed into
`AddLocalConst` / `LessLocalLocal`, and constant-key hash reads are fused super-instructions.
What remains is structural:

- **`size_of::<Value>() = 24` bytes** against Ruby's 8-byte tagged `VALUE`, so every stack
  push/pop moves 3× the memory.
- No tagged/unboxed integers — Ruby's fixnums live in the pointer with no discriminant check.
- Dispatch is a Rust `match` (jump table) rather than direct/computed-goto threading.

Closing the remaining 2–3× is a VM representation project (NaN-boxing or pointer tagging,
threaded dispatch), not a collections one. Worth doing deliberately or not at all.

### Taken while investigating: the super-instructions weren't firing

Soli already had fused opcodes for constant-key hash reads. They were mostly not being
emitted for the code people actually write. Three fixes, all measured with both binaries
running concurrently and interleaved rounds (the machine was under a variable external
compile load throughout, which makes sequential timing worthless):

| pattern | before | after | change | rounds won |
|---|---:|---:|---:|---:|
| `local["k"]` | 96 ns | 76 ns | **−21%** | 6/6 |
| `global.get("k")` in a fn | 103 ns | 88 ns | **−14%** | 5/6 |
| `global["k"]` in a fn | 101 ns | 90 ns | **−11%** | 5/6 |
| `local.get("k")` | 74 ns | — | unchanged | already optimal |

1. **Index syntax was never fused.** `.get("k")` emitted `HashGetLocalConst`; `h["k"]`
   emitted `GetLocal` + `HashGetConst`. So the idiomatic spelling was the slow one — and
   `params["name"]` is the most common hash read in controller code.
2. **The global forms never fired inside a function.** They were gated on
   `scope_depth == 0`, a proxy for "cannot be a local" that also excluded every function
   body — i.e. all real handler code. The gate now asks the compiler's own
   `resolve_variable`, which distinguishes `Local` / `Upvalue` / `Global` properly. The
   upvalue case is why the crude gate existed: emitting the global opcode for a *captured*
   binding would read the wrong variable. That case now falls through to the generic path,
   verified by tests on both engines (`CAPTURED` / `GLOBAL` / `LOCAL` all resolve correctly).

`has_key` on a local hash is now **1.1× faster than Ruby 3.4.9**.

A measurement caveat worth recording: an intermediate "cumulative" run showed the gate fix
doing nothing, which contradicted its own direct A/B. The cause was mine — the comparison
binary had been copied *before* the rebuild, so it did not contain the change. Re-measuring
with a verified binary restored the expected result. When an A/B disagrees with itself,
suspect the artefacts before the code.

## String methods — no bottlenecks, and Soli is mostly well ahead of Ruby

Swept every string method at n and 4n: **all linear**, no quadratic behaviour. (Apparent
outliers in `sub`/`scan`/`camelize` were first-call warm-up, most likely regex compilation
caching — they vanish on the second run.)

Against Ruby 3.4.9 on an 8,000-word string, Soli VM leads on the native bulk operations —
`capitalize` **16×**, `downcase` 13.8×, `sub` 13.3×, `upcase` 8.9×, `replace_all` 7.6×,
`contains` 5.1×, `bytes` 3.5×, `chars` 3.0×, `split` 2.2×. Ruby leads on `lines` (9.1×,
though both are microseconds), `count` (6.4×), and `squeeze` (3.0×). Soli faster on 10 of
17 comparable operations.

### A benchmark bug of mine, corrected

My first run reported "Ruby 5.5× faster at string concatenation". That was wrong — I had
compared Ruby's **mutating** `<<` against Soli's **allocating** `+`. Like-for-like
(`r = r + "x"` in both), Soli wins and the lead widens with size:

| n | Soli `+` | Ruby `+` | |
|---|---:|---:|---|
| 5,000 | 0.57 ms | 2.34 ms | Soli 4.1× |
| 20,000 | 3.72 ms | 18.55 ms | Soli 5.0× |
| 40,000 | 13.92 ms | 84.76 ms | Soli 6.1× |

### The real gap: no in-place string append

Both languages are quadratic when building a string with `+` in a loop. Ruby offers `<<`
as a linear escape hatch; **Soli has none** — there is no `append`, and `<<` is array-only
(`"<< expects an array on the left, got string"`).

The idiomatic linear workaround exists and is fast: `parts.push(..)` then `join("")` is
**exactly linear** (0.35 / 0.72 / 1.43 / 2.89 ms for n = 5k…40k — 2× per doubling) and 4.8×
faster than `+=` at n=40,000, comparable to Ruby's `<<`.

So this is a documentation gap first and a feature gap second: string-building loops are a
common shape, `+=` is the obvious thing to reach for, and nothing in the language steers you
off it. Worth either documenting prominently or adding a mutating append.

## View rendering performance — one win taken, one hypothesis killed

### Fixed: the response cache paid a full data hash to rediscover it couldn't cache

`render()` opened with `data_signature(data)` — a recursive walk hashing every byte of every
string in the render data, so on a list page it scales with the whole result set.

The flags that decide cacheability are set *during* the render, not before it:
`csrf_meta_tag()` → `csrf_token()` → `mark_response_dirty()`, and the layout renders after
the cache lookup but before the store. So the page looked clean on entry, paid the hash,
missed (the store had been refused last time for the same reason), rendered, and was refused
again — every request, forever. The default `soli new` layout
(`template/app/views/layouts/application.html.erb:6`) calls `csrf_meta_tag()`, so this was
the normal path for real apps, not an edge case.

`render()` now remembers refused `(template, layout)` pairs and skips the signature for them.

**372 µs → 330 µs CPU/request (-11.3%), 3076 → 3522 req/s (+14.5%)** — 300-row list page,
idle 16-core box, both binaries served concurrently with interleaved rounds, faster in 8/8
with non-overlapping ranges. Pages that *do* cache are unaffected (zero re-renders across
100 requests on `www/`).

### Measured and rejected: the `html_response` injection chain

`html_response` runs six sequential passes over the body (live-reload, nav, prefetch,
native, camera, sensors), each scanning for its marker and, on the no-op path, doing
`return html.to_string()` — a full copy of the whole body. For a page using no camera,
sensors, or native bridge that is three redundant full copies plus several full scans.

It looks like an obvious win. **It is not.** A build with the three optional injections
removed measured **2.5% (8.1 µs), 7/8 rounds**, ranges nearly overlapping — confirmed twice,
under load and idle. A six-file signature refactor (`&str` → `String` to move instead of
copy) is not worth 2.5%. Recorded here so the hypothesis isn't re-derived and acted on.

The useful lesson: at these sizes the memcpy is cheap; the cost was in the *data-proportional
hash*, not the *output-proportional* copies.

## P2 — Feature gaps

A team adopting Soli will hit these in roughly this order.

- **SoliDB-only.** Zero files in `src/` mention Postgres, SQLite, or MySQL. The comparison
  page is honest about it; it remains the single largest adoption blocker.
- **No admin panel.** Rails has Administrate, Laravel has Nova/Filament, Django ships
  `contrib.admin` — the most-cited reason teams pick Django at all. Soli has the scaffolding
  machinery (`soli generate`), the ORM introspection, and a form builder; this is closer
  than it looks.
- **ORM batching.** No `find_each` / `in_batches`, so iterating a large collection
  materialises it. Compounds with the known cursor-truncation issue at `crud.rs:507`.
- **No optimistic locking** (`lock_version`).
- **No attribute-level encryption** (Rails `encrypts`) and **no audit-log/versioning**
  (`paper_trail`). Both are routinely mandatory in regulated work — exactly where the
  existing Factur-X, PAdES, and tamper-evidence investment is already aimed. The crypto
  primitives are all present (`Crypto.canonical_json`, `merkle_root`, `ledger_hash`); this
  is an ORM-integration job, not a cryptography one.
- **No read replicas / connection pooling** to SoliDB.
- No GraphQL, no gRPC.

## P2 — Language & tooling

- **No debugger.** `debug()` opens a dev REPL page; there is no DAP server, so no
  breakpoints in VS Code. The biggest DX gap relative to the runtime's maturity.
- **LSP** (`src/lsp/`, 1,860 lines) has hover, completion, definition, type-definition,
  references, rename, formatting, folding, and inlay hints — but no semantic tokens and no
  signature help.
- **The type checker cannot check controllers.** `render`/`redirect` fail `soli check` by
  design, so the largest body of application code is unreachable by the checker and `lint`
  is the only gate.
- **VM refuses to compile:** `break`, safe navigation (`&.`), command substitution, list and
  hash comprehensions as sub-expressions, and several match patterns. Each is an
  `EngineFallback` — correct, but each one silently costs the VM.

## P3 — Small

- **Flaky test.** `platform::lock::tests::acquires_when_free_and_refuses_while_held` failed
  once in four full `--lib` runs, then passed on re-run and passed 3/3 in isolation. Likely
  mechanism: `flock` is tied to the open file description, and a concurrent
  `Command::spawn` elsewhere in the suite forks a copy of the fd; between `fork` and `exec`
  (where `O_CLOEXEC` closes it) the lock is still held, so the reacquire after `drop(first)`
  can lose the race. Stated as a hypothesis, not a confirmed diagnosis.
- **CI clippy is narrower than it needs to be.** `cargo clippy --locked -- -D warnings` omits
  `--all-targets`, so ~2,200 tests' worth of code is never linted. It is clean under
  `--all-targets --all-features` today, so tightening the gate is free.
- **5 `www/docs/*.md` files have no `.slv` counterpart**: `solidb-reference`,
  `soli-language`, `testing-assertions`, `testing-e2e`, `testing-guide`. Some are probably
  deliberate; worth an explicit decision given the documentation policy in `CLAUDE.md`.
- **Stray empty file** `Value` at the repo root, committed 6 July.
- 57.6 MB release binary.

---

## Verification performed

Static gates:

- `cargo fmt --check` — clean.
- `cargo clippy --locked --all-targets --all-features -- -D warnings` — clean.
- `cargo test --locked` — **2487 passed, 0 failed** across all test binaries.
- `cargo run -- test` (Soli suite) — **151 passed, 0 failed**, 3887 assertions. The trailing
  `post-suite truncate … 401` warning is a pre-existing local SoliDB auth issue, unrelated.

The `panic = "abort"` guard, tested **in both directions** — the half that is usually
skipped:

- clean on the current profile;
- `RUSTFLAGS="-C panic=abort" cargo check` **fails** with the intended message.

End-to-end against a real `--release` binary (`SOLI_WORKERS=2`, two-route fixture app):

| Check | Result |
|---|---|
| `/_health`, `/_ready` while serving | `200 ok`, `200 ready` (+ `HEAD`, `Cache-Control: no-store`) |
| `soli_handler_panics_total` on `/_metrics` | present, `0` |
| `/_ready` during drain | **`503 draining`** |
| `/_health` during drain | **`200 ok`** — liveness stays healthy, as intended |
| new request during drain | `503 Server shutting down` |
| **request in flight when `SIGTERM` arrived** | **completed with `200 slow-done`** — not truncated |
| process exit | clean `0`, promptly after the last request drained (not at the 25s deadline) |
| docs pages render | `configuration`, `comparison`, `changelog` all `200` with the new content present |

Two things this pass could **not** verify end-to-end, stated plainly rather than implied:

1. **A real panicking Soli handler.** Every surface probed — bad UTF-8 boundaries, negative
   repeats, out-of-range indices, malformed regex/JSON/hex/datetime, division by zero —
   returned a clean Soli error rather than a Rust panic. That is a genuinely good result for
   the builtins, and it means there is no easy way to provoke the panic path from Soli. The
   containment is instead covered by three unit tests (`serve::tests`) that exercise the real
   `run_caught` code path: a panicking handler yields a counted `500`, a panic mid-loop does
   not stop the four requests around it, and panics unwind in the build profile.
2. **Behaviour under sustained load during a drain.** The drain waits for in-flight
   connections to reach zero; because non-probe requests are answered `503` immediately that
   converges quickly, and the grace deadline bounds it regardless — but this was verified
   with one in-flight request, not a live traffic load.

A bug in the first draft of the drain was caught by the end-to-end test rather than by
review: breaking the accept loop on drain returned from `block_on`, which dropped the tokio
runtime and killed the very connections being drained (the in-flight request died and the
process then hung). The accept loop now stays running for the drain; the comment at that
site records why, since the naive version looks more correct than it is.
