# Changelog

## [Unreleased]

### Performance

* **perf(interpreter):** unified member-call dispatch — `obj.method(args)` evaluates the receiver expression exactly once and dispatches on the value; arrays gain a direct fast path (the hash/string ones already existed), skipping the per-call `ValueMethod` boxing: `array_ops` bench **−8%**
* **fix(interpreter):** side-effectful method-call receivers no longer run twice — `make().map(f)` used to call `make()` two times because each call interceptor (model callbacks, hash, string) evaluated the object and re-dispatched on a type mismatch. Covered by `tests/language/method_receiver_single_eval_spec.sl` for array/hash/string/instance receivers, including interceptor-overlapping names (`delete`, `save`)
* **perf(interpreter):** direct instance-method invocation — `obj.method(args)` on non-model instances now binds `this` straight into the call environment instead of allocating a bound `Function` whose construction deep-cloned the entire method body AST on every call. Monomorphic method calls **−52%**, polymorphic call sites **−40%** (criterion, new `inline_cache` bench group). The bound-`Function` path remains for method-as-value (`f = obj.m`), named-argument calls, and model instances
* **perf(interpreter):** `is_model_subclass()` memoized on `Class` (was up to four superclass-chain walks with string compares per instance member access); instance fields switched from std `HashMap`/SipHash to ahash (hot property reads −6%)
* **perf(interpreter):** array methods that never run user closures (`take`, `sum`, `flatten`, set ops, `pluck`, …) now execute on a live borrow instead of an O(n) snapshot clone; closure-taking iterators (`map`, `each`, `sort` with a comparator, …) keep snapshot semantics so mutating the receiver mid-iteration stays well-defined
* **fix(interpreter):** `arr.sort(comparator)` whose comparator mutated the receiver (e.g. `arr.push(...)`) aborted the process with a `RefCell already borrowed` panic — `sort` was misclassified as a pure method and ran on a live borrow. It now iterates over a snapshot like the other closure-taking iterators. Covered by `tests/language/method_receiver_single_eval_spec.sl`
* **perf(serve):** SoliDB connection keep-warm — pooled DB connections idled out after 5s, so on a quiet server any request after a gap paid a fresh DNS + TCP (+ TLS) connect mid-request: intermittent ~400ms latency spikes. Pool idle is now 90s (`SOLI_DB_POOL_IDLE_SECS`) and serve mode runs a periodic read-only ping that keeps a live connection pooled and pre-warms the model DB at boot (previously only the SoliDB session store pre-warmed). Disable with `SOLI_DB_KEEP_WARM=0`
* **perf(serve):** session-store keep-warm — the `spawn_db_keep_warm` ping only covered the *model* DB host, so a network-backed session store (SoliDB on a different host, or SoliKV) had nothing exercising its pooled connection between requests. On a quiet server it idled out and the next request paid a cold reconnect — surfacing as intermittent spikes on trivial routes (a `/session/ping` heartbeat jumping ~6ms → ~70ms). Serve mode now runs a periodic read-only ping (`RETURN 1` for SoliDB, `PING` for SoliKV) against the session store too, on the same cadence as the model keep-warm. No-op for the in-memory/disk drivers. Disable with `SOLI_SESSION_KEEP_WARM=0`
* **perf(serve):** hot-reload version checks collapsed to a single generation-counter load per worker tick (was eight Acquire loads); WS presence ref counter relaxed ordering

### Added

* **feat(serve):** Instant Navigation — Turbo-Drive-style body swapping, on by default. A framework script (`/__soli/nav.js`, auto-injected like prefetch.js) intercepts same-origin GET link clicks, fetches the page (reusing an in-memory hover-prefetch cache that rides the existing `Purpose: prefetch` / `SOLI_PREFETCH_TTL` server machinery), swaps `<body>` in place merging title/stylesheets/meta, and manages history with `pushState`/`popstate` (scroll restore, cheap 304 refetch on back/forward). Inline body scripts re-run per visit, external scripts once per URL (no Alpine/htmx double-boot), all sequenced in document order with externals awaited (parser semantics — an inline `tailwind.config` after the Tailwind CDN script keeps working), then `Alpine.initTree` + `htmx.process` re-wire the new body only after the script chain settles; `DOMContentLoaded`/`load`/`alpine:init` listeners registered after the events already fired are replayed (jQuery-ready semantics), so existing init code wrapped in `DOMContentLoaded` and page bundles registering `Alpine.data` components via `alpine:init` keep working after swaps; the incoming body carries `x-ignore` until its scripts settle so Alpine's MutationObserver can't initialize it early; `soli:visit` / `soli:before-render` (cancelable) and `soli:load` events for userland hooks; opt-in View Transitions via `<meta name="view-transition" content="same-origin">`. Skips htmx-managed links, `data-method`, downloads, new-tab intent; falls back to a real navigation for non-HTML responses, cross-origin redirects, and `x-teleport` pages. Opt out per link (`data-no-nav`), per page (`<meta name="soli-nav" content="off">`), or globally (`SOLI_NAV=off`, which restores plain hover prefetch)
* **feat(serve):** `SOLI_SLOW_REQUEST_MS` — production slow-request logging: a request whose total time (queue wait + handler) crosses the threshold prints a full `[SLOW]` detail block (every `SOLI_LOG` channel plus the queue-wait split); faster requests stay silent. Composes with `SOLI_LOG`. The access line now shows queue wait (`(12.3ms + 0.4ms queue)`) when request logging is active, so a request stuck behind a busy worker is distinguishable from a slow handler
* **feat(lang):** `Int#to_s(base)` — Ruby-style radix conversion for bases 2–36: `255.to_s(16)` → `"ff"`, `255.to_s(2)` → `"11111111"` (lowercase digits, leading `-` for negatives, `i64::MIN`-safe). Complements the existing `"ff".hex` reverse direction
* **feat(lang):** explicit empty parens on zero-arg builtin methods now work in both engines (`n.abs()`, `x.to_f()`, `dt.year()`) — previously "Cannot call non-function value" on primitives while collections accepted them; the type checker also types bare zero-arg member access as the method's return type (`s.length` is an `Int`, matching runtime auto-invoke)

### Fixed

* **fix(serve):** WebSocket registry no longer holds the connections lock across `send().await` in `send_to` / `broadcast_all` / `broadcast_to_channel` / `broadcast_to_channel_except` / `close` — one slow or stalled client could block every other WS/LiveView operation (joins, presence, other broadcasts). Senders are cloned out under the lock and sends happen lock-free

### VM engine parity

The bytecode VM (production mode) now runs whole categories of code that previously errored and silently fell back to the tree-walking interpreter per request:

* **fix(vm):** primitive method dispatch (Int, Float, Bool, Null, Decimal) — `n.to_s(16)`, `f.round(2)`, `d.between?(...)`, `times`/`upto`/`downto` with closures, all via `call_*_method_impl` dispatchers shared with the tree-walker, so engines can't drift. Decimal negation (`-2.5D`) also fixed
* **fix(vm):** native instance classes (DateTime, Duration, …) — native methods are bound to their receiver via the same wrappers the tree-walker uses (the VM used to call them with the receiver missing from `args[0]`); Model-subclass *statics* (`User.where`, …) get the class bound and run on the VM. Model instance mutators (`record.save()`) deliberately raise an uncatchable `EngineFallback` so serve mode still re-runs them on the interpreter, where lifecycle callbacks fire
* **fix(vm):** user-defined classes work in VM scripts — compiled constructors and methods were silently dropped by `op_add_method` (`Person("Alice")` produced an empty instance). Constructors (incl. synthetic ones for field defaults), `this`-bound methods, statics, universal members (`class`, `is_a?`, `nil?`) and field assignment all dispatch natively now
* **fix(vm):** `super(...)` constructor chaining and `super.method(...)` — call frames record the *defining* class so multi-level hierarchies resolve correctly instead of looping
* **fix(vm):** `try`/`catch`/`rescue` now catch native-method errors (the run loop routes `RuntimeError`s through active handlers, binding the error text like the tree-walker). Also fixes a `rescue` compiler bug where the catch offset pointed past the fallback — the exception value leaked out as the rescue result even for user-level `throw`
* **fix(vm):** stored bound methods called with arguments (`m = arr.contains; m(5)`) read the wrong stack slot as the receiver
* **fix(cli):** `soli run --vm` seeds the VM from the full builtin environment like a production serve worker (was a 6-function hand-rolled subset where even `DateTime` was undefined) — `--vm` is now a faithful production simulator

### Fixes

* **fix(lang):** bare `Person(...)` instantiation now applies class field initializers (`role: String = "guest"`) — previously only the `new Person(...)` form did
* **fix(datetime):** chained DateTime results keep the full method map — `dt.add_days(3).format(...)` failed with "Cannot access property 'format'" because each method captured a half-built method-map snapshot; all DateTime/Duration instances now share one complete class
* **fix(datetime):** `Duration.between` stored the raw *nanosecond* diff as seconds — a 1-hour span read back as ~10⁹ hours via `total_hours`/`humanize`
* **fix(types):** DateTime/Duration checker whitelists synced with the runtime (`beginning_of_*`, `end_of_*`, `humanize` were rejected at check time); universal methods (`class`, `nil?`, `is_a?`, …) accepted on built-in class instances; empty-parens calls on zero-arg members type-check

* **fix(test):** `soli test --jobs N` no longer storms SoliDB's `/auth/login` — the runner logs in once and hands the JWT to every test-server child via `SOLIDB_JWT`, and a failed login backs off 30s instead of retrying on every query. Previously N parallel boots tripped SoliDB's per-IP login rate limit (20/min, shared `127.0.0.1` bucket) and a single failure became a self-sustaining 400 storm (475+ warnings per suite) that randomly pushed specs past their 10s HTTP timeouts

* **fix(test):** pre-created worker-DB collections keep their SoliDB type (`document`/`edge`/`blob`) — blob uploads (`doc_files`, `card_attachments`, …) 400'd against collections pre-created as plain documents; type mismatches are detected and repaired (drop + correctly-typed recreate)

* **fix(serve):** WebSocket upgrades work again — the h1/h2c auto-detect change (1cc2a7a, v1.8.3) served connections with hyper's plain `serve_connection`, which never performs the HTTP/1.1 protocol upgrade after a 101: every WebSocket (`/ws/*` routes, LiveView, live reload, presence) died with `[WS] WebSocket handshake error: Handshake not finished` and clients reconnect-looped forever. Now uses `serve_connection_with_upgrades` (h2 streams unaffected); covered by an e2e echo round-trip test
* **fix(vm):** safe navigation (`&.`) in a handler no longer aborts the whole server at warmup — the VM compiler now returns a compile error (handler falls back to the tree-walking interpreter) instead of hitting an `unimplemented!()` panic, which core-dumped the process under the release profile's `panic="abort"`

### Performance

* **perf(vm):** function and method calls are ~30% faster on call-heavy code — the VmClosure call fast path is inlined in `Op::Call` and compiled-method dispatch (the source span is computed only on the cold arity-error branch instead of every call, and the `call_value` double dispatch is gone); fib(32): 0.72s → 0.51s
* **perf(datetime):** DateTime/Duration methods that return instances no longer rebuild a full `Class` per result (a dozen allocations + a ~30-entry method-map clone each) — all instances share one `Rc<Class>`; ~25% faster DateTime-heavy code in both engines
* **perf(test):** per-run test database reset is ~200× faster — collections are truncated (a 1-25ms range delete each, in parallel) instead of dropping + recreating the whole database (~180ms *per collection*, serialized inside SoliDB: 7.3s on a 41-collection app). `SOLI_TEST_FRESH_DB=1` forces the old drop+recreate when a schema-level reset is wanted
* **perf(test):** new worker DBs pre-create the base DB's collections through one sequential queue *before* specs run, instead of lazily mid-request — a first `--jobs 16` run no longer blows random specs past the 10s timeout while SoliDB serializes hundreds of collection creations. The reset phase now reports per-DB progress (truncate/create counts and timings) instead of running silently
* **perf(value):** Soli strings now use `SoliStr = ecow::EcoString` in `Value::String`/`Value::Symbol`/`HashKey`/VM constants — strings ≤15 bytes are stored inline (constructing them no longer touches the heap) and longer strings are refcounted with O(1) clone. Passing/reading large strings (rendered partials, request bodies, template data) no longer deep-copies: ~5× faster on a 64KB-string passing benchmark; ~+17% server throughput on realistic browser-header requests
* **perf(serve):** single-pass header materialization — hyper's `HeaderMap` travels to the worker as-is and is converted to the `req["headers"]` hash exactly once (was: per-header owned copy on the async side plus a second copy on the worker)
* **perf(serve):** the Cookie header is parsed once per request (was twice: session-ID extraction and `req["cookies"]` each re-scanned it); SEC-077 `__Host-session_id` precedence preserved
* **perf(serve):** the `params` global reuses the `all` hash returned by the request-hash builder instead of re-probing the request hash by string key
* **perf(vm):** for-in over strings iterates by byte offset (no upfront `Vec<char>`), for-in over hashes indexes the live IndexMap (no upfront key-vector clone)
* **perf(interpreter):** for-in over arrays uses live bounds-checked indexing instead of snapshot-cloning the whole array; `for i in a..b` iterates the range directly instead of materializing it into an array first. **Behavior change:** mutation of the iterated array inside the loop body is now observed live in both engines (matching the VM, Ruby-style)

### Features

* **feat(vm):** list/hash comprehensions now execute on the bytecode VM at clean stack positions (a new compile-time stack-height gate) instead of always falling back to the interpreter; comprehensions used as a sub-expression still fall back
* **feat(vm):** experimental `SOLI_VM_OPTIONAL_LET=1` opt-in to run bare-assignment (optional-`let`) handlers on the VM — off by default until the remaining VM gaps are closed
* **perf(metrics):** Prometheus timing collection (lexing/parsing/VM/template) is now opt-in via `SOLI_METRICS=1`, removing per-dispatch `Instant::now()`/atomic overhead when unused. **Behavior change:** the `/_metrics` endpoint returns zeros until `SOLI_METRICS` is set
* **perf(routing):** the dynamic-route fallback no longer re-tests static routes (static paths already resolve via the O(1) exact-match index)
* **perf(lexer):** skip the keyword lookup for `?`/`!`-suffixed identifiers (`nil?`, `push!`, …), which can never be keywords
* **feat(lang):** add UUID (`uuid_v4`/`uuid_v7`, `UUID.v4`/`UUID.v7`), ULID (`ulid`, `ULID.generate`/`ULID.new`), and NanoID (`nanoid(size?, alphabet?)`, `NanoID.generate`/`NanoID.new`) ID generators
* **feat(jobs):** add `Webhook` job class (`enqueue`/`enqueue_in`/`enqueue_at`/`cancel`/`list`) and adopt `SOLI_WEBHOOK_SECRET` with `X-Webhook-Signature` (keeping `SOLI_JOBS_SECRET`/`X-Job-Signature` as legacy aliases)
* **feat(serve):** log production errors on the dev and OOP-controller paths too (breakpoints excluded)
* **feat(test):** extend the `as_user` E2E session helper to accept an optional second argument
* **feat(model):** accept Symbol arguments in DSL callbacks and relationships (`before_save :method`, `has_many :posts`, etc.) for Ruby-style shorthand ([#](https://github.com/solisoft/soli_lang/commit/436b4ff))
* **feat(parser):** `~` shorthand for `implements`; Ruby-style classes-oop docs ([6d157bb](https://github.com/solisoft/soli_lang/commit/6d157bb))
* **feat(dev-bar):** break down render time per middleware ([e2509af](https://github.com/solisoft/soli_lang/commit/e2509af))
* **feat(dev-bar):** add hierarchical flamegraph and per-template breakdown ([0119472](https://github.com/solisoft/soli_lang/commit/0119472))
* **feat(model):** add `includes_count` and cache preloaded relations ([28e0d23](https://github.com/solisoft/soli_lang/commit/28e0d23))
* **feat(testing):** add `with_session` builtin and expand session-helper docs ([3cfbbb7](https://github.com/solisoft/soli_lang/commit/3cfbbb7))
* **feat:** named route helpers, LiveView ticks, integration tests ([234889f](https://github.com/solisoft/soli_lang/commit/234889f))
* **feat(lang):** add Ruby-style `begin`/`rescue` aliases for `try`/`catch` ([fd16f5e](https://github.com/solisoft/soli_lang/commit/fd16f5e))
* **feat(dev-bar):** instrument response-producing native builtins as Fn spans ([6c71e44](https://github.com/solisoft/soli_lang/commit/6c71e44))
* **feat(dev-bar):** hierarchical view tree, render-id pairing, root request span ([e918af6](https://github.com/solisoft/soli_lang/commit/e918af6))
* **feat(serve):** preload public CSS/JS into in-memory cache for atomic deploys ([5103aec](https://github.com/solisoft/soli_lang/commit/5103aec))
* **feat(deploy):** add local rsync mode + read api key from env ([63efd30](https://github.com/solisoft/soli_lang/commit/63efd30))
* **feat(lang):** add `url_encode(value)` and `url_decode(string)` builtins — strict RFC 3986 component encoding on the way out, form-style decode (`+` → space, `%xx` → byte) on the way in
* **feat(lang):** add `index_of` and `each_with_index` methods on arrays ([efa42a5](https://github.com/solisoft/soli_lang/commit/efa42a5))
* **feat(test):** per-worker progress UI and smart --jobs default ([932ebb8](https://github.com/solisoft/soli_lang/commit/932ebb8))
* **feat(serve):** add SOLI_TRACE_BOOT env-gated boot tracing ([e72be73](https://github.com/solisoft/soli_lang/commit/e72be73))
* **feat(lang):** add postfix `rescue` operator for inline fallback values (`expr rescue fallback`)
* **feat(test):** add `db_name()` builtin for parallel-safe DB targeting
* **feat(test):** isolate parallel test workers with per-worker DB and server
* **feat(jobs):** background job system with `enqueue()`, `Job` class, and `async` keyword
* **feat(model):** `has_many` chainable methods (`.where()`, `.order()`, `.limit()`, `.select()`)
* **feat(model):** HABTM (has_and_belongs_to_many) relations with join table support
* **feat(respond_to):** content negotiation built-in for handling multiple formats (html, json, etc.) ([82c61ab](https://github.com/solisoft/soli_lang/commit/82c61ab))
* **feat(solidb):** improved SolidB client integration ([82c61ab](https://github.com/solisoft/soli_lang/commit/82c61ab))
* **feat(migration):** enhanced migration DSL ([82c61ab](https://github.com/solisoft/soli_lang/commit/82c61ab))
* **feat(uploads):** URL-driven image transforms on attachment endpoints ([ef7c2ef](https://github.com/solisoft/soli_lang/commit/ef7c2ef))
* **feat(uploads):** model-level uploader DSL with auto-routed attachments ([6102481](https://github.com/solisoft/soli_lang/commit/6102481))
* **feat(vm):** support hash attributes in `Class.new()` and fix function body compilation ([c128c23](https://github.com/solisoft/soli_lang/commit/c128c23))
* **feat(model):** `Model.create` returns instance; `_errors` array on failure, `nil` on success
* **feat(model):** `Model.find` raises `RecordNotFound` when id is missing (HTTP layer auto-converts to 404)
* **feat(repl):** display the result of `@sdql{ ... }` expressions ([1454b22](https://github.com/solisoft/soli_lang/commit/1454b22))
* **feat(template):** bind `locals` hash to every partial context (Rails-style `local_assigns`)
* **feat(serve):** conditional-GET revalidation on `render()` HTML responses with ETag support
* **feat(model):** `instance.save(hash?)` and `instance.update(hash?)` accept bulk-attribute hash

### Bug Fixes

* **fix(session):** the SoliDB session driver now pre-warms its backend connection at boot on the long-lived runtime, so `ensure_session` doesn't open the process's first SoliDB connection mid-request. The warmup is non-blocking (a slow or unreachable session DB never delays startup) and logs a classified outcome — `[timeout]` / `[connect]` / `[request]` with the full cause chain — to diagnose session-backend latency. No-op for the in-memory / disk drivers
* **chore(serve):** request access logs and boot-trace lines are now prefixed with a local wall-clock timestamp (`2026-06-01 14:23:45.123`) to make latency easier to correlate
* **fix(vm):** correct a class of control-flow / local-assignment bugs on the bytecode VM, found via a new tree-walker-vs-VM differential harness: a peephole that **inverted** `if`/`while` on a bare local (ran the wrong branch), `for`-loop closures capturing the loop variable, the index in `for v, i in …`, `a..b` range bounds (now exclusive of `b`, matching the interpreter), assignment and `return` inside a `catch` block being dropped, and a crash on `let x = <local>` / `||=`
* **fix(vm):** comprehensions and variable-binding `match` patterns no longer silently corrupt results or abort the worker when unsupported — they cleanly fall back to the tree-walking interpreter
* **fix(interpreter):** closures created in different iterations of a `for`/`while` loop now capture distinct per-iteration bindings instead of sharing one
* **fix(serve):** route OOP-controller **auto-render** (set `@vars`, let the matching view render with no explicit `render()` call) through `html_response`. It was hand-building the response with only `Content-Type`, silently dropping the `ETag`, `Cache-Control`, and the injected hover-prefetch `<script>` — so apps that rely on auto-render (the idiomatic MVC flow) got no prefetch and no conditional-GET caching on any page, while explicit `render()` calls did. Both paths now behave identically.
* **fix(prefetch):** serve speculative prefetch requests (`Sec-Purpose: prefetch`) a short `private, max-age` (default 30s, `SOLI_PREFETCH_TTL`) instead of `no-cache`, so the click reuses the prefetched HTML straight from the browser cache — no conditional GET, so a CDN (Cloudflare et al.) that won't relay a `304` can no longer turn hover-prefetch into a wasted full re-download. Normal navigations keep `private, no-cache`.
* **fix(prefetch):** emit weak ETag (`W/"..."`) so CDNs that re-encode (Brotli/gzip) don't strip it — strong ETags were being dropped at Cloudflare, breaking 304 reuse and turning the hover-prefetch feature into a cosmetic load
* **fix(metrics):** wire lexing/parsing/VM execution counters — they were defined but never incremented, always showing 0 ([#](https://github.com/solisoft/soli_lang/commit/436b4ff))
* **fix(image):** validate write paths against image jail without false negatives on non-existent targets ([368df5f](https://github.com/solisoft/soli_lang/commit/368df5f))
* **fix(jwt):** enforce HMAC secret floor before token header parsing; surface explicit PEM errors for RS256/EdDSA ([368df5f](https://github.com/solisoft/soli_lang/commit/368df5f))
* **fix(model):** tighten `is_unique_violation` to require HTTP 409 status — prevents silent misclassification of unrelated 5xx errors ([368df5f](https://github.com/solisoft/soli_lang/commit/368df5f))
* **fix(serve):** accept `1`/`yes` in addition to `true` for `SOLI_DISABLE_CSRF` env var ([368df5f](https://github.com/solisoft/soli_lang/commit/368df5f))
* **fix(template):** `js_escape` now escapes newlines, CR, and tab to prevent literal breakout from JS string context ([368df5f](https://github.com/solisoft/soli_lang/commit/368df5f))

### Documentation

* **docs(model):** document Arc<Mutex<FutureState>> threading concern ([368df5f](https://github.com/solisoft/soli_lang/commit/368df5f))
* **docs(solidb):** document SolidbState password retention in memory ([368df5f](https://github.com/solisoft/soli_lang/commit/368df5f))
* **docs(callbacks):** document delete callback gap ([368df5f](https://github.com/solisoft/soli_lang/commit/368df5f))

### Tests

* **test(kv):** KEYS test now requires `SOLI_KV_ALLOW_ADMIN=1` env var to run ([368df5f](https://github.com/solisoft/soli_lang/commit/368df5f))

* **fix(parser):** parse `|params|` in trailing brace blocks ([be792eb](https://github.com/solisoft/soli_lang/commit/be792eb))
* **fix(dev-bar):** make panel scrollable and pin header when expanded ([3c6449a](https://github.com/solisoft/soli_lang/commit/3c6449a))
* **fix(solidb):** make `Solidb(host, db)` construct and dispatch instance methods ([02702ce](https://github.com/solisoft/soli_lang/commit/02702ce))
* **fix(i18n):** correct `I18n.format_currency` carry bug — rounding to total cents first prevents `9.995` from formatting as `"9,100 €"` instead of `"10,00 €"` ([bec9c30](https://github.com/solisoft/soli_lang/commit/bec9c30))

### Performance

* **perf(model):** dedupe validation rule registration ([aa66cd1](https://github.com/solisoft/soli_lang/commit/aa66cd1))
* **perf(test):** cut `--jobs N` startup overhead and balance work across workers

### Tests

* **test(http):** replace httpbin.org with in-process mock server — faster, non-flaky, works offline
* **test:** improved error formatting with box-drawing characters ([41c14a6](https://github.com/solisoft/soli_lang/commit/41c14a6))
* **test:** added controller_spec tests for respond_to content negotiation ([82c61ab](https://github.com/solisoft/soli_lang/commit/82c61ab))
* **test:** auto-display coverage when tests pass ([9550941](https://github.com/solisoft/soli_lang/commit/9550941))

### Documentation

* **docs(scaffold):** rewrite generated CLAUDE.md for new-app conventions ([dfd28d5](https://github.com/solisoft/soli_lang/commit/dfd28d5))
* **docs(www):** add dev-bar and competing-with-big-frameworks blog posts ([7d4b892](https://github.com/solisoft/soli_lang/commit/7d4b892))
* **docs(middleware):** modernize syntax in middleware examples ([18bd5c3](https://github.com/solisoft/soli_lang/commit/18bd5c3))

## [0.80.1](https://github.com/solisoft/soli_lang/compare/0.80.0...0.80.1) (2026-04-23)

### Other
* **chore: release v0.80.1** ([92f653e](https://github.com/solisoft/soli_lang/commit/92f653e37473315226eeb25c8414b0cf5c958f4f))
* **chore: bump version to v0.80.1** ([9a2cdf7](https://github.com/solisoft/soli_lang/commit/9a2cdf7cdd000b300e75536eba3e2d31ba8987b1))

## [0.80.0](https://github.com/solisoft/soli_lang/compare/0.79.1...0.80.0) (2026-04-23)

### Bug Fixes
* **fix(template):** route paren-form `render(...)` through the core parser ([06508fe](https://github.com/solisoft/soli_lang/commit/06508fe1c12f93ef3f306a96067c1c23440cc137))

### Other
* **chore: bump version to v0.80.0** ([58989d9](https://github.com/solisoft/soli_lang/commit/58989d924461d6a973383e58c1d11ed7d87e4d76))

## [0.79.1](https://github.com/solisoft/soli_lang/compare/0.79.0...0.79.1) (2026-04-23)

### Tests
* **test: expand error page tests to cover all explicit status arms** ([3ac2995](https://github.com/solisoft/soli_lang/commit/3ac2995fb236233567157e4c3048073240322e22))

### Other
* **chore: release v0.79.1** ([afdf7f7](https://github.com/solisoft/soli_lang/commit/afdf7f71ff9c8c02001552d4fd8c8978ffe9bacd))

## [0.79.0](https://github.com/solisoft/soli_lang/compare/0.78.1...0.79.0) (2026-04-23)

### Features
* **add comment handling to static block extraction, controller inheritance, after_action hooks, and defensive partial tests** ([699a32a](https://github.com/solisoft/soli_lang/commit/699a32a1fa266cea03292bf956db9525c26bdcdb))

### Other
* **bump version to v0.79.0** ([11f2175](https://github.com/solisoft/soli_lang/commit/11f2175103f74d64449e83be1dc105a57b02516e))
* **update CHANGELOG for unreleased changes** ([5430ee2](https://github.com/solisoft/soli_lang/commit/5430ee27ff03ff18efc2740bc2aa460757114e60))