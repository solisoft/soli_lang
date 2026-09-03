# Changelog

## [Unreleased]

## [2.0.5] - 2026-09-03

### Fixed

- **`has_many` on an unsaved owner no longer hits the database.** The accessor
  returns a builder carrying the always-false filter `1 == 0`, but
  `update_all`, `delete_all`, `count` and `exists?` still sent the query. With
  no database reachable that surfaced as an error instead of the documented
  no-op, which is what failed the 2.0.3 and 2.0.4 CI test job. The builder now
  answers those without a round-trip.

## [2.0.4] - 2026-09-03

### Fixed

- **Sessions on the disk, SoliDB and SoliKV drivers survived exactly one
  request.** Since 2.0.3 the request resolver no longer mints a session for an
  unknown cookie and asks the store `exists(id)` instead. The trait's default
  `exists` probes a key with `get`, which on these three stores reads one entry
  of the session's data — a key nobody ever sets — so every existing session
  looked missing: a login wrote the session, the redirect landed, and the next
  request started over with none. Only the in-memory store had its own
  `exists`. Each persistent store now answers `exists` from the stored session
  and its expiry.

## [2.0.3] - 2026-09-03

### Security

A full-repository security audit produced 57 findings; all are fixed here.
Details below are grouped by what an operator or app author has to know about.

**Behaviour changes that may need action**

- **`--dev` refuses to start when `APP_ENV` is `production`.** It used to skip
  every production boot check silently — no `SOLI_APP_HOSTS`, no session-secret
  floor, security headers off, `/__solidev` diagnostics exposed — with nothing
  in the output to say so. Drop `--dev`, or set `APP_ENV` to something else.
- **`enable_trust_proxy` is no longer on by default in new apps.** An
  `X-Forwarded-*` header is only trustworthy behind a proxy that rewrites it;
  on a directly-exposed app it let any client spoof the request authority and
  scheme, and hand every request a fresh identity so per-IP rate limits (the
  login throttle included) never tripped. Uncomment it when you deploy behind a
  proxy, and name the hops with the new `SOLI_TRUSTED_PROXIES` (a comma-separated
  list of IPs or CIDRs); with it set, `X-Forwarded-*` is honoured only for
  requests that really came from those hops.
- **`cors()` refuses `credentials: true` together with a wildcard origin**, the
  way rack-cors does. Browsers reject `Allow-Origin: *` with credentials — that
  refusal is the safety net — and Soli sidestepped it by echoing back whatever
  Origin asked, making every cookie-authenticated route on the path readable and
  writable by any website. A wildcard rule also no longer satisfies the CSRF
  origin gate; only an explicitly listed origin does.
- **`.where({...})` refuses a request-supplied hash or array as a value.** The
  hash form reads a nested hash as operators (`{"gt": 10}`) and an array as
  `IN`, which is indistinguishable by shape from what a JSON body can send: a
  client posting `{"token": {"ne": null}}` turned an equality check on a secret
  into `!= null` and bypassed it. Values that arrived with the request are now
  tracked, and one that would act as an operator raises with the field named.
  Developer-written operator hashes are unaffected.
- **`Model.create` / `Model.update` never persist `_key`, `_id`, `_rev`,
  `_from`, `_to`, or an STI `type` from their input**, whether or not the model
  declares `attr_accessible`. A client could otherwise choose document ids or
  make a row hydrate as a privileged subclass.
- **The static `Model.update(id, hash)` now runs validations**, against the
  merged record rather than the patch, and returns `{"_errors": [...]}` when
  they fail. It previously wrote straight past every rule the model declared.
- **The email-confirmation link no longer signs the visitor in**, and expires
  after 48 hours. It is a `GET` reached from an email, so a link scanner, a
  shared mailbox or a forwarded message received the session.
- **A password reset now invalidates existing sessions** through a
  `session_version` stamp checked by the generated `load_current_user`; an
  attacker who was already signed in used to survive the victim's reset.
- **`/_metrics` is no longer public.** With `SOLI_METRICS_TOKEN` set it requires
  that bearer token; without it, only loopback and private-range peers.
- **`button_to(..., {"confirm": ...})` emits `data-confirm`** instead of an
  inline `onclick`, and `form_with({"url": ...})` / `button_to` refuse
  `javascript:` targets.
- **LiveView rooms are opt-in per component**: declare them with
  `live_rooms("desk")` in `config/routes.sl`. `?room=` on an undeclared
  component handed its full rendered HTML, and the right to drive its events,
  to anyone who guessed the name.

**Denial of service**

- WebSocket frames on app-defined routes are enqueued without blocking. A
  blocking `send` from a tokio task parked an OS thread per frame, and a few
  flooding sockets could wedge the entire server — accepts, HTTP and health
  checks included. Adds a per-connection message rate limit, a per-IP and
  global WebSocket connection cap, a global TCP connection cap, and a body-read
  timeout for trickled uploads.
- Worker and job threads get a 64 MiB stack (`SOLI_WORKER_STACK_MB`). The
  interpreter's 256-frame budget did not fit the default 2 MiB, so ordinary
  recursion — or a walker over a deeply nested JSON body — aborted the whole
  process rather than failing one request.
- `range()`, `a..b` and `string * n` are bounded (`SOLI_MAX_RANGE_LEN`,
  `SOLI_MAX_STRING_ALLOC_BYTES`) and reject negative counts. A request-supplied
  bound could ask the allocator for gigabytes, and an allocation failure aborts
  the process.
- `str()`, string interpolation, `==`, `flatten` and the debug environment dump
  are depth-bounded, so a cyclic value (`parent` ↔ `children`) no longer
  overflows the native stack.
- Integer `+ - * / %` and `Decimal` arithmetic are checked in both engines.
  Release builds silently wrapped where debug builds panicked, and
  `i64::MIN / -1` panicked in both.
- `Array.sort` tolerates an inconsistent comparator instead of panicking, and
  `DateTime.format` rejects an invalid pattern instead of panicking.
- `Deflate.inflate` returns large binary output as base64 rather than one
  24-byte value per byte, and its default cap drops to 8 MiB: a 65 KB SAML
  payload reached 1.8 GB resident.
- `paginate({"per": n})` and vector-search `top_k` are clamped
  (`SOLI_MAX_PAGE_SIZE`), `Model.limit` rejects negatives, and query/form
  parameter counts are capped (`SOLI_MAX_PARAM_PAIRS`).
- A handler now has a wall-clock execution budget, 30s by default
  (`SOLI_HANDLER_TIMEOUT_SECS`, `0` disables). A runaway loop used to remove a
  worker permanently.

**Information disclosure**

- The production `[ERROR]` log no longer prints the request's globals verbatim.
  Redaction applied only to top-level variable names, so the carefully redacted
  `request:` snapshot was followed by `req.headers.cookie`,
  `req.headers.authorization` and `params.password` in the clear.
- LiveView instance ids no longer embed the raw session id, which was sent to
  page JavaScript in every render — an `HttpOnly` bypass for any XSS on the
  page — and written to the server log.
- `~/.soli/credentials` is written `0600`.
- Baseline `Referrer-Policy: strict-origin-when-cross-origin` is emitted, and
  the SEC-056 baseline headers are emitted at all: a thread-local cache seeded
  with the same version as the global counter looked valid and empty forever, so
  default-configured apps shipped without `X-Frame-Options` or `nosniff`.

**Server-side request forgery and path traversal**

- Web Push endpoints, webhook deliveries and PDF image sources go through the
  same SSRF gate and DNS-filtering client as `HTTP.*`. All three previously used
  bare clients that followed redirects anywhere; webhook delivery ran on the
  client deliberately exempted from the blocklist.
- The `Solidb` client percent-encodes every REST path segment. An unencoded key
  let `db.delete("posts", "../../../databases/production")` reach a different
  endpoint entirely, with the app's own credentials.
- `Image.plan` / `ImagePlan.save_to`, the spreadsheet readers and writers, and
  the PDF font directories go through the SEC-006 filesystem jail.
- The request `Host` is validated against `SOLI_APP_HOSTS` before any `*_url`
  helper builds an absolute URL, so a spoofed `Host` cannot aim a password-reset
  link at another site.

**Realtime**

- A LiveView client may only send `_assigns` keys the server rendered as
  `soli-assign-*`, and may only address `_component`s present in its own markup.
  Both were unrestricted: one message could overwrite any root-state key
  (identity, tenant, price) or invoke a component handler that was never mounted.
- WebSocket and LiveView handlers now run with the socket's session installed,
  and the event carries the `headers`, `query` and `params` the docs already
  promised. Previously `session_get` returned null inside them, leaving a
  client-supplied id as the only available identity.

**Other**

- New `json_script(value)` for embedding JSON in a `<script>` body;
  `json_stringify` does not escape `</script>`.
- `t()` escapes interpolated values for keys ending in `_html`.
- Named-route helpers percent-encode their parameters.
- `jwt_verify(token, secret)` refuses a PEM or SSH key as the HMAC secret.
- `KV.cmd` denies `FCALL`, `MODULE`, `MIGRATE`, `RESTORE`, `SELECT`, `SWAPDB`,
  the replication verbs and the introspection verbs.
- Mail attachment `content_type` is CRLF-checked and shape-validated; IMAP
  literals and POP3 responses are size-capped.
- An unknown `session_id` cookie no longer creates and persists an empty
  session, and the in-memory store honours expiry on read.
- Per-IP rate limiting aggregates IPv6 to a /64
  (`SOLI_RATE_LIMIT_IPV6_PREFIX`) and evicts the least recently active bucket.
- The limitation that query-builder reads inside `transaction { ... }` see
  *committed* state (only `find` and the document verbs run through the
  transaction) is now documented in the models reference, including its
  consequence for uniqueness validations: two records violating a uniqueness
  rule can both pass validation inside one block. Routing those reads through
  the transaction would need a transactional cursor endpoint the SoliDB API does
  not expose.
- `delete_all` / `update_all` report database errors instead of returning null.
- The generated OIDC provider rejects an `id_token` used as an access token,
  rotates refresh tokens atomically, and confirms `GET /oauth/logout`.
- A locked account is indistinguishable from a wrong password and its password
  is not verified at all; the owner is notified by email when the lock trips.
- Responses built as a hash get a default `Content-Type`.

### HTTP client

- **`headers` in the options hash of every `HTTP.*` verb.** `HTTP.get`,
  `post`, `put`, `patch`, `delete`, `head`, `get_json`, `get_jsonp`,
  `post_json`, `put_json`, `patch_json` and the batch helpers `get_all` /
  `get_all_json` now honour a `"headers": { name => value }` key next to
  `timeout` — the shape the docs already showed but the runtime silently
  dropped, so an `Authorization` header only reached the wire through
  `HTTP.request`. A caller header replaces the builtin's own default
  (`Content-Type` on the body verbs, `Accept: application/json` on the JSON
  variants), matched case-insensitively, so a request never carries two.
  String values go out verbatim, other scalars are stringified, `null`
  skips the header; a name or value reqwest would refuse (a space in the
  name, CR/LF in the value) raises up-front with the offending header
  named — never its value, which is usually the credential — instead of
  failing at send time as an opaque builder error. `Content-Length` and
  `Transfer-Encoding` are refused on every path (`HTTP.request`'s flat
  hash included, which previously validated nothing): hyper sends a
  caller-supplied length verbatim, and a mismatch desyncs the upstream
  connection — a request-smuggling primitive when the header is forwarded
  from an inbound request.
  `HTTP.request(method, url, headers)` keeps its flat headers hash and
  additionally merges a nested `headers` key, so the one options shape
  works everywhere.
- **The static checker knows the whole `HTTP` class.** `soli check` /
  `soli -e` / top-level scripts type-check before running, and the typed
  `HTTP` declaration listed only `get`/`post`/`put`/`delete`/`request`/
  `get_all`/`get_all_json`/`get_jsonp`, each without the options
  parameter — so `HTTP.get(url, {"timeout": 5})` failed with `Wrong number
  of arguments: expected 1, got 2` and `HTTP.get_json(...)` with `Cannot
  access member 'get_json' on type 'HTTP'` before a byte was sent. Every
  runtime verb is now declared with its trailing options hash, and
  indexing a `Future` (`HTTP.get(url)["status"]`) is accepted the same
  way member access on one already was, instead of `cannot index
  Future<String>`.

## [2.0.2] - 2026-09-02

### ORM

- **`db.timeout(secs)` and `db.query(sdbql, binds, {timeout})` — the 10s ceiling on raw SDBQL too.**
  `.timeout` on a QueryBuilder already raised the HTTP cap for one Model
  read; a `Solidb` client had no equivalent, so `db.query` of a heavy
  aggregation died at ~10s (`query() expects 1 or 2 arguments` if you
  passed an options hash; `Cannot access property 'timeout'` if you
  chained). `db.timeout(60).query(sdbql)` sets the budget on the client
  (chainable, persists until changed); `db.query(sdbql, binds, {"timeout": 60})`
  overrides one call. Seconds as `Int` or `Float`; a zero, negative,
  non-numeric, or unknown option key raises. Inside `grouped(fn() { … })` a
  **read** `db.query` against the same host and database joins the coalesced
  batch, so the largest `.timeout` any member asked for covers it — a write
  via `db.query` still runs immediately.

## [2.0.1] - 2026-09-02

### ORM

- **`.timeout(secs)` on a query — raise the 10s ceiling on one slow read.**
  Every read reaches SoliDB over HTTP, and that client allows a request 10s
  (`build_internal_client`), a hardcoded backstop with no way around it: a
  report over a large collection or a multi-aggregate `group_by` failed with
  `Error: HTTP error: error sending request for url …` no matter how healthy
  the database was. `Order.where(…).timeout(120).all()` now gives that one
  query two minutes, as does the `Model.timeout(secs)` static entry point.
  Chainable and position-independent like `.limit`; seconds as `Int` or
  `Float` (`0.5` is valid); a zero, negative, or non-numeric value raises
  rather than being ignored, so a typo cannot silently leave the default in
  place. Lower it too — `.timeout(2)` fails fast instead of making a user
  wait. The override is scoped to the one request and reverted on completion,
  error included, so it never leaks into the next query on the thread. Inside
  a `grouped(fn() { … })` block the reads are one request, which runs under
  the **largest** `.timeout` any member asked for — a slow member is not
  capped by the fast reads sharing its round-trip. **No effect on the SQL
  adapters**: Postgres/MySQL/SQLite use their own connection pool, so the
  10s HTTP cap does not exist there and there is no statement timeout to set
  in its place — the call is accepted for portability, and the server's own
  `statement_timeout` / `max_execution_time` remains the way to bound a SQL
  query.

### CI

* **Four jobs were compiling from scratch on every run.** `test`, `clippy`,
  `browser` and `verapdf` used `actions/cache/restore` keyed on
  `${{ github.sha }}` with no `restore-keys` — and nothing in the repository
  ever ran `actions/cache/save`, so that key was never written and the restore
  missed every time. `gh cache list` confirmed it: no `Linux-cargo-<sha>` entry
  had ever existed. All four now use `Swatinem/rust-cache`.
* **`windows-check` had no cache at all**, and at 15.2 min it is the critical
  path — every other job finishes inside it, so its cold rebuild set the length
  of the whole run. It caches now too.
* **Cache writes are restricted to `main`** on all six cargo caches (`save-if`).
  The repository was at 12.2 GB against GitHub's 10 GB limit, which evicts in
  LRU; letting every PR branch push a multi-GB entry is how caches start
  displacing each other. 5.7 GB of provably superseded entries were deleted at
  the same time — stale fuzz build caches from an old `Cargo.lock` hash,
  superseded corpus snapshots, and tag-scoped copies from `v2.0.0`.


## [2.0.0] - 2026-08-31

### Docs

- **Rust internals guide** (`/docs/internals`) — crate map, lexer/parser/AST, interpreter, VM, serve, SQL adapters, and a type/method catalog for junior Rust contributors. Pages are **Markdown views** (`www/docs/internals/*.md`), not duplicated `.html.slv`.

### Hardening

- **Stack-overflow proofing across the language runtime.** A stack overflow aborts the process without unwinding — beyond the reach of the per-request `catch_unwind` fault isolation — so every recursive surface now has a depth guard that returns a catchable error instead: the lexer→parser pipeline (expressions, statements, match patterns, type annotations; max depth 64), template block parsing (`<% if %>`/`<% for %>`/`content_for`/builder blocks) and partial/component includes (max 64), Soli-to-Soli call recursion in both engines (32 debug / 256 release frames), and JSON conversion + serialization of runtime-built deep structures (512 levels). Deeply nested source or templates now yields a clean parse/render error; runaway recursion raises `call stack too deep …`, which `try/catch` can intercept.
- **Constructor-body errors are no longer swallowed.** The tree-walking executor discarded any error raised inside a constructor body (including the new depth guard), so a failing constructor silently produced a half-initialized instance. Errors now propagate.
- **A template naming a helper that needs arguments no longer aborts the process.** The paren-free form (`<%= now.to_iso %>`) is documented as being for *zero-argument* callables, but the renderer auto-called **every** callable it evaluated, with an empty argument list. So `<%= patch %>` ran the `patch` request helper's body with no arguments; helpers registered variadic (`arity: None` — nothing upstream checks the count) read `args[0]` directly and panicked, which aborts rather than raising. The renderer now applies the same arity rule as a bare name in code, and the request helpers bounds-check their arguments and raise `… is missing a required argument`. Found by the `template_parse_render` fuzz target; the reproducer is a tracked seed.
- **Fuzzing infrastructure.** Three libFuzzer targets under `fuzz/` — `parse_program`, `template_parse_render`, `json_roundtrip` — seeded from `tests/`, `examples/`, `evals/`, and shipped views; a `fuzz.yml` workflow runs them nightly (10 min/target) and as a 20 s smoke on PRs touching lexer/parser/template/JSON paths, uploading crash artifacts on failure.
- **Unwrap-count ratchet in CI.** `scripts/lint_unwraps.sh` freezes `.unwrap()`/`.expect()` counts per exposed module (template engine, lexer, parser, JSON conversion); CI fails when a count grows and lowers the bar permanently when fixes land.
- **Production `soli serve` fails closed without `SOLI_APP_HOSTS` and a 32+ character `SOLI_SESSION_SECRET`.** When `APP_ENV` is `production` or `prod`, boot refuses to start if the public-hostname list is missing/empty or the session secret is missing/short; the error names the variable. `--dev` and non-production env still boot without them.
- **`soli new` requires per-form CSRF tokens.** The generated `.env` sets `SOLI_CSRF_TOKENS=require`, so a browser form post without `_csrf_token` / `X-CSRF-Token` is 403. Existing apps are unchanged (runtime default stays unset). JSON APIs are not token-gated; `skip_csrf` still opts a path out.
- **`security/unfiltered-mass-assignment` lint.** `Model.create(params)` / `.update` / `.create_many` in `app/controllers/` or `app/services/` with the raw request hash is a warning; `permit` / `_permit_params` / a hash literal is clean.
- **File-mode HTTP responses no longer unwrap a poisoned builder.** `soli serve` on a plain directory built `Location` / body responses with `.body(..).unwrap()`. A path that injected CR/LF into `Location` panics the worker. Those sites now go through `finish_response` and return 500. Inventory: `scripts/inventory_panics.sh`. Defaults vs. remaining operator knobs: [Production security defaults](www/docs/security/defaults.md).

### Builtins

- **`Url` class.** Parse, build, join, and rewrite URLs without string surgery: `Url.parse`, `Url.build`, `Url.join`, `Url.params`/`param`/`set_param`, percent-encoding helpers.
- **`Logger` class.** Leveled structured logging to stderr (`debug` < `info` < `warn` < `error`) with optional key-value fields, text or JSON line format, `SOLI_LOG_LEVEL` default, and a capture mode for specs.
- **`Retry.with_backoff` / `Retry.within`.** Exponential-backoff retries for flaky outbound calls, plus a deadline variant; engine-embedded Soli so blocks just work.
- **`CircuitBreaker`.** Per-name circuit breaker (closed/open/half-open) with a process-global store shared by both engines and all workers; callback-free `allow`/`record_success`/`record_failure` API.
- **`Toml` / `Yaml` classes.** Parse and generate TOML and YAML over the same Value model as JSON.
- **`Semaphore` class.** Named process-global counting semaphores with explicit tokens — "at most N of these running at once" without DB round-trips.
- **`Money` class.** Currency-aware amounts over decimals with ISO-4217 minor units, mismatch-detecting
  arithmetic, lossless largest-remainder allocation, and localized formatting. Amounts are quantized to
  the currency's minor units on every operation, so `m["amount"]` is always exactly what
  `Money.format(m)` displays; currency codes are case-insensitive and validated as three ASCII letters.
- **The new classes type-check.** `Money`, `Url`, `Logger`, `Toml`, `Yaml`, `CircuitBreaker`, `Semaphore`
  and `Retry` were installed at runtime but unknown to the type checker, so every documented example
  failed with `Undefined variable 'Money'` unless it was run with `--no-type-check`.
- **`Retry` works in views and helpers.** It was registered only in the interpreter constructors, so
  `Retry.with_backoff(...)` inside a `.html.slv` view or an `app/helpers/*.sl` raised
  `Undefined variable: Retry` while every other new class resolved there.
- **`Retry.within` backs off.** It ignored `factor` and `max_delay` and retried at a constant 0.25s
  (documented default: 0.5s), so a long `deadline` meant hundreds of tight retries against a service
  that was already down. It now backs off exactly as `with_backoff` does, bounded by elapsed time.
- **A half-open circuit admits one probe.** Nothing recorded that a probe was already running, so the
  instant the cool-down elapsed *every* concurrent caller was let through — a thundering herd onto the
  dependency that had just failed. A probe that never reports back no longer wedges the circuit either,
  and a failed probe re-opens immediately rather than waiting for the threshold again.
- **`CircuitBreaker.configure` accepts fractional `reset_after`.** The Float branch scaled to
  milliseconds and then passed the number to `Duration::from_secs`, so `{"reset_after": 0.5}` held the
  circuit open for 500 *seconds*.
- **A per-key semaphore name no longer fills the store.** Slots were created per name and never
  reclaimed, so a pattern like `"import-#{tenant}"` permanently filled the 1000-name store, after which
  every `try_acquire` for a new name *raised* — a 500 inside a request handler. Unheld slots are now
  reclaimed when the store hits its cap; the slot itself survives a drain, so a name still keeps the
  limit its first caller fixed.
- **`Retry.within` stops at its deadline.** It checked the deadline and then slept a full delay, so
  `{"deadline": 2, "base_delay": 1.5}` ran for 4.5s and a 10s deadline could block ~18s at the default
  `max_delay`. The sleep is clamped to the remaining budget.
- **A money hash read back from JSON or the database gets its currency normalized too.** Normalizing
  only in `Money.new` left `{"currency": "jpy"}` from a payload taking the wrong minor-unit exponent
  (2 instead of 0) and missing its symbol.
- **A deeply nested JSON body is rejected instead of killing the worker.** `json_parse` (and the
  request-body parser behind `req["json"]`) uses the hand-rolled parser, which recursed one frame per
  nesting level with no cap — so `[` × 100k aborted the process outright. A native stack overflow does
  not unwind, so the per-request `catch_unwind` never saw it. Past 512 levels the parse now returns an
  ordinary catchable error, and the request path treats it as an unparseable body. The consuming
  `serde_json` → Value conversion is capped too; its by-reference twin already was.
- **A success reported while a circuit is open no longer closes it.** `record_success` closed
  unconditionally, so a call that started before the circuit tripped and finished late reopened the
  floodgates — defeating the half-open probe. It now closes only from half-open and clears the count
  when closed.
- **`configure()`d circuits survive store pressure.** The reclaim predicate dropped exactly the
  healthy circuits, configured ones included, so boot-time tuning silently reverted to the default
  threshold under per-tenant naming. Configured, tripped and failing circuits are never evicted, and
  when nothing is reclaimable the store refuses to grow rather than exceeding its cap — an untracked
  name then fails open, so a full store cannot start refusing healthy traffic.
- **`Semaphore.reset(name)`** drops a name and every token held on it. `release` was the only way to
  free a slot, so a token leaked by a job that raised before releasing wedged that name for the life of
  the process — the nightly job simply stopped running until a restart.
- **Text-mode log lines cannot be forged.** The message and each field value were spliced raw into one
  line, so a newline in user input (`Logger.info(params["email"])`) wrote a complete extra record,
  `[ERROR]` and all. Both are escaped now; JSON mode was already safe.
- **`Toml.parse` no longer leaks serde's datetime marker.** A TOML date came back as
  `{"$__toml_private_datetime": "..."}` instead of the timestamp, so `config["when"]` was a one-key
  hash — and dates are everywhere in real TOML.
- **`Url.build` stops losing data.** A `query` hash silently dropped arrays and nested hashes (a
  filter URL lost its filters); they now expand to the bracket names request params use. `username` /
  `password` are honoured, so `Url.build(Url.parse(u))` keeps credentials instead of stripping them,
  and an unknown key is an error rather than a silent no-op that hides a typo.
- **`DateTime` answers `is_a?`.** It was the one universal member still missing, so generic dispatch
  (`if v.is_a?("string") { … }`) raised on a DateTime rather than answering false.
- **`Url` decoding matches request params.** `+` now decodes to a space (so `Url.params` and
  `req["params"]` agree on the same query), and an escape that is not valid UTF-8 is kept verbatim
  instead of being replaced with an empty string. `Url.set_param` rewrites only the named param and
  leaves every other pair's text byte-identical — it used to round-trip the whole query, which blanked
  a param it could not decode and turned a `+` in an untouched value into `%2B`.
- **A `DateTime` answers the universal members.** `inspect`, `to_s`, `class`, `nil?`, `blank?` and
  `present?` were hard errors on a DateTime — it resolved only its own registered methods — so the REPL
  echoing a result raised `Cannot access property 'inspect' on DateTime` for `DateTime.now()`. Both
  engines share the one definition, so both are fixed.
- **JSON log lines are always valid JSON.** A field value with no JSON form was spliced in unquoted, so
  a function field emitted `{"cb":Function}` and a non-finite float `{"ratio":NaN}` — breaking every
  downstream parser for that line.

### Language / VM

- **Sub-expression comprehensions and binding `match` compile on the VM** by wrapping the construct in a zero-arg lambda so the result/subject sits at a real local slot. Nested `[x for x in xs]` and `out.push(match i { n => … })` no longer demote the whole handler.
- **`grouped(fn() { … })` and `Model.transaction { … }` run on the VM.** The compiler no longer
  refuses the block forms; the native-call path uses the same begin/flush and begin/commit
  helpers as the tree-walker, invoking the block through `invoke_callable`. Because a
  transaction can now commit on the VM, a handler that fails *after* committing is no longer
  re-run on the tree-walker — the retry would repeat the committed writes.
- **Reading a `grouped` deferred result works everywhere on the VM.** Property access already
  resolved it; iteration (`for post in @posts`), indexing (`@posts[0]`) and reading it back
  out of an instance field now do too, matching the tree-walker.
- **`SOLI_FAIL_ON_VM_DEMOTION=1` stops the server when the VM refuses a handler**, so CI cannot
  ship a new refuse silently. It fires only on an engine-fallback refusal — not on the
  handler's own errors (a `throw`, a 404 `RecordNotFound`) — and exits the process rather
  than panicking, which the per-request `catch_unwind` would have turned into a 500.
  `SOLI_ENGINE_LOG=1` still logs every demotion. Note the bytecode VM only runs outside
  `--dev`, so neither applies to `soli serve --dev` or `soli test`.
- **Command substitution (backticks) compiles on the VM** as `System.shell`. Property access on a `Future` or a `grouped` `Deferred` auto-resolves, matching the tree-walker, so `` `printf hello`.stdout `` stays on the bytecode path.
- **Model instance methods run on the VM.** `record.save()` / `update` / `touch` and the rest no longer
  demote the handler so `before_save` can fire in the tree-walker. Method-name lifecycle callbacks are
  compiled as methods with the record bound as `this`. A **closure-form** callback (`before_save do … end`)
  still falls back: it needs the scope it captured, which the bytecode path cannot reconstruct. The
  refusal is decided *before* the write, so a handler never demotes with the row already written.
- **`Model.create` / `Model.update` run on the VM** with the same before/after callback wrap (temp
  instance, hash round-trip, veto returns an instance with `_errors`).
- **Class `method_missing` and model state-machine members run on the VM.** `UserMailer.welcome(user)`
  no longer demotes the handler; `order.pay` / `paid?` / `can_pay?` dispatch on the bytecode path for a
  machine with no `guard` and no transition hooks. A machine that declares any of those falls back,
  for the same captured-scope reason as closure-form callbacks — checked before the state is written.
  `pay!` persists through the same `save` wrap as a normal write.
- **`record.delete()` / `Model.delete(id)` with `dependent:` or attachments stay on the VM.** Cascades
  (including nested child callbacks and cycle no-ops) and `detach_all_uploads` run from the bytecode
  wrap instead of demoting the handler.
- **A handler is no longer re-run on the tree-walker once it has written to the database.** The
  no-retry tripwire covered `Model.transaction { … }` commits only; it now also covers any write made
  outside a transaction (`create` / `save` / `update` / `delete`, plus the bulk SQL paths that commit
  on their own). A bare `Post.create(params)` followed later in the same handler by a construct the VM
  refuses used to insert the record twice.
- **`after_create`-only (and `after_update`-only) callbacks now fire.** A model declaring an `after_*`
  callback with no matching `before_*` one silently skipped it in the tree-walker — so it ran in
  production and not under `--dev` or `soli test`. Both engines now run it, matching the documented
  callback table.
- **Class `method_missing` no longer shadows reflection and dynamic finders.** On a class defining a
  static `method_missing` (the `Mailer` shape), `Foo.send("bar")`, `Foo.methods()`, `Foo.class_eval(…)`
  and `User.find_by_email("x")` dispatched into `method_missing` on the VM and returned a wrong value.
  `method_missing` is now the last resort on both engines, as the tree-walker always ordered it.
- **Columnar models still refuse the document API on the VM.** `create` / `update` / `delete` on a
  columnar model raised through the shared choke point, but the new callback- and cascade-wrapping
  paths called the native directly and skipped it, silently running the document API.
- **A `grouped` deferred result materialises when read into a container.** The property fast paths
  pushed the placeholder straight onto the stack, so `render("posts/index", { "posts": @posts })`
  handed a deferred to the template even though iterating or indexing it resolved correctly.
- **`Model.transaction(some_fn)` opens a transaction on both engines.** The VM intercepts any callable,
  while the tree-walker only recognised a literal lambda or a `do … end` block — so that call committed
  a real transaction in production and ran untransacted under `--dev`. A bare identifier is now
  recognised too (a computed callee still differs, and is documented as such).

### SQL connection security

- **Postgres and MySQL connections speak TLS.** Neither client had a TLS
  implementation compiled in: the Postgres pool was built with `NoTls` and the
  `mysql` crate resolved with no TLS backend, so `?sslmode=require` was accepted
  and then ignored and every managed database (RDS, Cloud SQL, Neon,
  PlanetScale, Aiven) needed a proxy or a private network in front of it. Both
  now use **rustls with the `ring` provider** — the provider the mail and HTTP
  clients already use, so no system OpenSSL is linked and a cross-compiled
  binary keeps TLS.
  - **libpq's ladder, and libpq's semantics.** `disable` / `prefer` / `require` /
    `verify-ca` / `verify-full`, with MySQL's spellings (`DISABLED` …
    `VERIFY_IDENTITY`) parsing to the same rungs. Encryption and identity stay
    separate: `require` encrypts without checking who answered, and verification
    begins at `verify-ca` — so a URL behaves as it does under `psql`, and a
    self-signed server keeps working.
  - **`prefer` is the default.** A server offering TLS gets an encrypted
    connection with no configuration; one that does not still connects.
    `sslmode=disable` asks for the previous cleartext behaviour.
  - **A CA file replaces the built-in roots** (`sslrootcert=` on Postgres,
    `ssl-ca=` on MySQL) and is refused, not ignored, when the mode would never
    consult it. Both options are lifted off the URL before the driver's own
    parser sees it — `tokio_postgres` rejects `sslrootcert` as unknown, and
    `mysql` rejects any parameter it does not list.
  - **A mandatory mode cannot be satisfied by a Unix socket.** The MySQL driver
    skips TLS on a socket outright and prefers a socket for a `localhost` URL,
    which would have satisfied `REQUIRED` in cleartext; `REQUIRED` and up now
    take TCP. Postgres does not negotiate TLS on a socket either, so `require`
    fails rather than pretending.
  - **A TLS failure names its cause.** The pool used to retry until its timeout
    and report *timed out waiting for connection*; a mandatory mode is now
    probed once as the pool opens, so the error reads `connection "primary"
    asked for sslmode=require: error performing TLS handshake: server does not
    support TLS` immediately. `pg_error` also renders a driver error's whole
    source chain, not just its top line.
  - New deps: `tokio-postgres-rustls` (feature `ring`), the `mysql` crate's
    `rustls-tls-ring` feature, and `rustls-pki-types`' `std` feature for reading
    a PEM CA file. `RUSTSEC-2025-0134` (rustls-pemfile unmaintained, reached
    only through the MySQL driver's CA reader) is waived in `.cargo/audit.toml`.
- **The SQL adapters are a CI gate.** Their tests skip when no server answers,
  and a skipped test still reports `ok` — `cargo test` swallows the message for a
  passing test — so a green build never actually meant Postgres and MySQL had
  been exercised. CI now runs both as service containers, with TLS switched on
  in the Postgres one so the encrypted path is asserted rather than assumed, and
  `SOLI_REQUIRE_DB=1` turns every would-be skip into a failure. Locally, without
  the flag, the suite skips exactly as before.

### PDF

- **SVG images are embedded as vectors.** Template `image` elements whose
  source is SVG are converted with svg2pdf and placed as Form XObjects
  instead of being rasterised through resvg/tiny-skia. Logos stay sharp at
  any placed size; `<text>` still uses `font_dirs`.
- **Fixed: an embedded SVG produced an unreadable PDF.** The imported Form's
  own resources — the ICCBased colour space svg2pdf always attaches, plus a
  `FontFile2` for `<text>` and any raster the SVG references — were written
  inline in the resource dictionary. PDF permits a stream only as an indirect
  object, so every SVG-bearing document came out corrupt (qpdf: *expected
  endobj*; poppler could not parse the object; the artwork did not render).
  Nested resource streams are hoisted into their own objects and referenced.

### Docs

- **Blog: Stripe Checkout.** A how-to for taking payments in a Soli app
  (session + signed webhook). There is no `soli generate stripe`.
- **Blog: What's on `main` since v1.29.0.** A tour of the unreleased cycle
  (SQL, jobs, LiveView, unless, auth).
- **`/ai` landing page.** Public “Soli is built for AI” page (conventions,
  token efficiency, agent contract, in-binary LLM/RAG) — same role as
  rubyonrails.org/ai.
- **Agents on Soli.** Stage 1 model evals: `evals/` fixture + 12 atomic
  tasks, `scripts/evals/run.py` with frozen runners (`claude -p`,
  OpenCode + DeepSeek, Grok Build), empty `www/data/ai_evals.json`, table
  on `/ai` (no invented scores until a paid run is committed).

### Language

- **Mixin modules.** `module Name … end` is a mixin (and a namespace for nested classes). `include Greetable` copies the module's instance methods onto the class; `extend Greetable` copies them as class methods. Module methods are also callable on the module itself (`Greetable.hello()`). `new` on a module raises. File `import`/`export` is unchanged.
- **Concern hooks.** `included do` / `extended do` replay their body on the host class (same class-body DSL as a Model, so `validates` / `has_many` work). `class_methods do` installs class methods on the includer. `def self.included(base)` / `def self.extended(base)` are called with the host class.
- **Block `unless … end`.** `unless` is a first-class statement (`StmtKind::Unless`),
  not a desugared `if !cond`. Multi-line guards parse and stay `unless` through
  `soli fmt` (a short body still collapses to postfix `expr unless cond`).
  `else` is allowed; `elsif` is not. Postfix `expr unless cond` is unchanged.

### Fixed

- **A nested-object `update` inside a Postgres transaction committed it early.**
  The recursive-merge path (read-modify-write under `FOR UPDATE`) issued a raw
  `BEGIN`/`COMMIT` on the connection an enclosing `Model.transaction` was
  already holding, so its `COMMIT` committed the caller's work and the caller's
  rollback became a no-op; on the error side its `ROLLBACK` discarded writes the
  caller had already made. It nests with a `SAVEPOINT` when a transaction is
  open on the same connection.
- **Ordering a grouped query sorted text instead of numbers.** Grouped rows are
  selected as text so all dialects return one shape, and a bare alias in
  `ORDER BY` binds to that output column — so
  `Order.group_by("status").order("n", "desc").limit(3)` ranked a count of `9`
  above `100`. Ordering targets the underlying expression now: the aggregate
  itself, or the table-qualified group column.
- **Grouped keys lost their column's type.** Parsing every group key turned a
  text key of `"00042"` into `42` and merged `"01"` with `"1"`; keeping every
  key as text fixed that but turned `Order.group_by("year")` on an integer
  column into `{"year": "2024"}`, breaking arithmetic and `== 2024`. The schema
  decides: numeric columns yield numbers, text columns keep their exact text.
- **`Model.paginate` swallowed query errors.** It read the in-band
  `"Error: …"` value as `0`, reporting `total: 0, total_pages: 1` with an error
  string in `records` for a missing table. Both its count and its records query
  raise, matching `Model.count`.
- **A webhook whose delivery thread could not be spawned was stranded.** The
  failure path returned the worker slot but left the job on the in-flight list,
  so lease renewal extended it forever and the row sat in `running` unclaimable.
- **Job retention and the queue filter scanned the whole table.** `prune_done`
  pre-counted with a `SELECT` before deleting (the `DELETE` already reports the
  count), and `queues()` selected one document per non-terminal job to collect a
  handful of distinct names — now a `GROUP BY`.
- **`SOLI_CSRF_TOKENS=require` 403'd the jobs dashboard's own buttons.** The
  dashboard authenticates with Basic auth rather than a cookie session, so it has
  no session token to embed in its retry/cancel forms. `/__soli/jobs` keeps the
  Origin/Referer gate and is exempt from the mandatory-token layer only.
- **Merge updates behaved differently on every SQL adapter.** Postgres used
  `jsonb ||` (shallow, stores a null); SQLite and MySQL use RFC 7396 (merges
  nested objects, *removes* a null key). So `update({"prefs": {"theme": "dark"}})`
  destroyed the rest of `prefs` on Postgres, and `update({"deleted_at": null})`
  left two different documents. RFC 7396 is the defined behaviour everywhere
  now: a flat patch stays one atomic statement on Postgres, a nested one takes a
  row-locked read-merge-write. `update_all` follows the same rule and refuses a
  nested-object patch on Postgres. Verified against live Postgres and SQLite.
- **An app on a non-public Postgres schema saw every table as missing.**
  `table_exists` hardcoded `table_schema = 'public'` while every other schema
  query uses `current_schema()`, so reads returned empty and the job poller never
  claimed — a record created and immediately looked up raised `RecordNotFound`.
- **A repointed connection kept using the old database.** Adapter pools were
  cached by connection *name* alone, so changing a URL handed back the previous
  pool. All three adapters key on name and URL.
- **Job retention stopped working once history built up.** `prune_done` scanned a
  500-row window, and `dead` rows are terminal and never pruned — 500 of them
  filled it permanently. `Job.queues()` emptied itself the same way. Both are
  scoped at the database now.
- **`Cron.every` registered schedules that never fired.** `"90 minutes"`,
  `"25 hours"` and `"40 days"` each emitted a `*/N` beyond its field's range,
  which the parser rejects; `"90 seconds"` was truncated to 60. Each is an error
  now, and every in-range value is covered by a test.
- **Grouped-query keys were coerced to numbers.** A group key of `"00042"` came
  back as `42` and `"01"`/`"1"` collapsed into one bucket, despite the doc
  comment saying keys stay text. Only aggregates are parsed now.
- **Column-mode `group_by` dropped `ORDER BY`, `LIMIT` and `OFFSET`** that its
  document-mode twin honours. The grouped SELECT list carries SQL aliases now so
  ordering can name a group key or an aggregate.
- **`LIKE` on a non-text column was rejected by the database.** The pattern's
  placeholder was cast to the column's type, producing `$1::text::uuid`, and
  `uuid LIKE uuid` is not an operator.
- **`db:migrate down` could roll back the wrong migration.** If the newest
  applied migration's file was missing, an *older* one was reverted instead of
  refusing. It stops with the orphaned version named.
- **`db.execute` self-deadlocked inside a transaction on SQLite.** It opened a
  second connection to the same file, and SQLite takes a database-wide write
  lock. It runs on the pooled connection now.
- **Two visitors joining a LiveView room at once orphaned one of them.** Mounting
  checked then registered as two steps, so the second registration replaced the
  first and left that socket open but unreachable. Attach-or-register is atomic
  now. Room uploads are scoped to the session that sent the event rather than the
  instance's creator.
- **A failed LiveView handler left a `soli-disable-with` button disabled
  forever** — the `error` frame never cleared the pending state.
- **`Model.count` returned an error string instead of raising.** On a `table "…"`
  model whose table was missing the failure came back as `"Error: …"` where a
  number belongs, so `try { Model.count() } catch` never fired. It raises now,
  and a `table` declaration on a connection that cannot serve it is refused
  rather than silently falling back to document storage.
- **`define_method` / `alias_method` did not work under `--vm`**, and the type
  checker rejected them on both engines. Both are implemented for the VM (a
  compiled body lands in the bytecode method table), and the checker knows the
  reflection surface, so neither needs `--no-type-check`.
- **`include` of a module declared inside a function failed under `--vm`.** The
  compiler resolved the name as a global; `include`/`extend` now use the same
  resolution as any other name read.
- **`module Foo` could not be typed into the REPL** — `module` was missing from
  the block-opener check.
- **`db:schema:dump` / `db:schema:load` did not read `.env`,** unlike
  `db:create` and `db:migrate`.
- **Non-ASCII `database.toml` values were mangled** — env expansion
  reinterpreted each byte as a character.
- **The eval harness was missing from a clean clone.** An unanchored `tasks*`
  rule in `.gitignore` also matched `evals/tasks/`, so all twelve fixtures were
  untracked and `scripts/evals/run.py` raised `FileNotFoundError`.
- **A method's implicit return produced `null` under `--vm`.** Free functions
  returned their trailing expression; methods compiled it, popped it, and fell off
  the end — so any method in the documented implicit-return style returned `null`
  in compiled mode, silently. Constructors still return the instance.
- **An index assignment evaluated to the container under `--vm`.** `h[k] = v` as a
  function's last statement returned the hash, and `h["k"] = v` returned `null`,
  instead of `v`. All four hash-set opcodes yield the assigned value now.
- **Nested-index peepholes dropped a stack slot.** `AddNestedIndex` /
  `SetNestedIndex` treated the trailing `Pop` as optional but push nothing where
  the replaced sequence leaves a value, so a function ending in
  `total = total + h[ks[k]]` returned a stray local. All four require the `Pop`.
- **`include` on a non-`Model` class failed the type checker.** Nothing under
  `src/types/` read `ClassDecl.includes`, so `soli check` rejected `u.greet()` for
  `class User { include Greetable }`. Module members (transitive includes and
  `class_methods do` blocks included) fold into the class type in a pass of their
  own, so declaration order does not matter.
- **A module's own method lost to the module it includes.** Transitive includes
  were applied first and both copies were first-wins, so
  `module Named { include Base; def label() {"named"} }` resolved to `Base`'s.
- **A nested concern's `included do` never ran against the class.** Hooks fired
  only for directly-named modules, so `module Auditable { include Timestamps }` +
  `class Post { include Auditable }` registered `Timestamps`'s hook on `Auditable`
  and never applied it to `Post`. Hooks fire for every module that joins the
  class, innermost first.
- **Concern hook bodies could not see application code under `--vm`.** They ran in
  a throwaway interpreter holding builtins only. It is seeded from the running
  program's globals now, and a bytecode function reached from a hook dispatches
  back through the VM.
- **Module hook metadata was lost to caching and re-execution.** The side table
  was thread-local while the compiled-module cache is process-global, and it was
  read destructively. It is process-global and read non-destructively.
- **A typo'd in-file `connection "name"` reported success while migrating the
  wrong database.** Unknown names were not resolved against the registry, and the
  resulting error was swallowed into "not SQL", so every step took its SoliDB
  branch — printing *Applied* while recording the version in the default
  database. In-file names are resolved at load time, with known ones listed.
- **A numeric `IN` list defeated every sibling predicate.** It compiled to
  unparenthesised disjuncts, and siblings join with `AND`, which binds tighter —
  so `Post.where({"id": [1,2,3], "status": "open"})` became
  `id=1 OR id=2 OR (id=3 AND status='open')`. The implicit soft-delete and STI
  scopes were defeated the same way, leaking soft-deleted rows.
- **A string `.where` chained onto a hash `.where` was discarded on SQL.**
  `User.where({"active": true}).where("doc.age >= @min", {"min": 18})` returned
  minors with no error. The raw filter is now told apart from the SDBQL echo a
  hash `.where` also produces, and the mixed form is refused with the hash
  equivalent named.
- **`Model.all`, `all_json`, `count` and `delete_all` ignored a model's own
  `connection`.** They tested the ambient default, so a `connection "reporting"`
  model read from SoliDB while `.where(…).all` reached Postgres; `delete_all`
  listed keys from one database and deleted from the other.
- **A slow webhook host could make a job run twice.** Delivery ran inline on the
  poller, blocking the tick past the lease (stalling lease renewal and cron with
  it) so a second poller re-claimed the in-flight job. It runs on its own thread,
  holding a worker slot and staying on the in-flight list.
- **`soli fmt` emitted lines `soli lint` rejected.** The postfix-collapse width
  estimate had no case for an interpolated string and scored every one as 8
  characters, so a long guard collapsed past the 120-char limit — a freshly
  generated app failed its own lint.
- **Non-ASCII `database.toml` values were mangled.** Env expansion reinterpreted
  each byte as a character, so a password with `é` arrived as `Ã©`.
- **`soli new` produced an app with no compiled stylesheet.** The Tailwind
  compiler ran with `current_dir(folder)` while its input path was also relative
  to `folder`, so the path resolved twice and `public/css/` was committed empty.
- **The documented `--no-default-features` build did not compile.** With every SQL
  feature off, one call site had no arm constraining its success type. CI built
  only `--all-features`.

### REPL

- **Ctrl+C twice exits the TUI REPL.** The first press still cancels the current
  line and now prints `^C  (press Ctrl+C again to exit)`; a second press with no
  other key in between saves history and quits. Any other key disarms it, and
  key-release events (Windows / kitty protocol) are ignored so one press is never
  counted twice.

### Testing

- **`soli test` drops its worker databases when the suite finishes.** The
  teardown used to truncate every collection and leave the databases in place,
  so a machine running many projects accumulated one empty `*_spec` database
  per worker per app, forever. The suite now drops them (issued in parallel,
  still serialised server-side) and the next run recreates them from the
  template. `SOLI_TEST_KEEP_DB=1` restores the truncate behaviour when the
  tight test loop matters more than the leftovers —
  `SOLI_TEST_FRESH_DB=1` is only meaningful in combination with it now.

### Jobs

- **`--dev` polls the job queue every 5s instead of every second.** `soli new`
  scaffolds `app/jobs/`, so every dev app starts the job engine whether it uses
  jobs or not, and each tick costs a lease-renew + cron check + claim round-trip
  against the database — several dev servers on one shared database spent most
  of its traffic on idle polling. Production still ticks at `1000` ms, `soli
  jobs` (a process started to run jobs) always uses the configured interval, and
  setting `SOLI_JOBS_POLL_MS` overrides the dev pacing too.

- **Production `/__soli/jobs`.** The queue dashboard (list, cancel, retry)
  is no longer `--dev` only. In production set `SOLI_JOBS_USER` +
  `SOLI_JOBS_PASSWORD` (HTTP Basic) and/or `SOLI_JOBS_TOKEN` (`Authorization:
  Bearer`). Unconfigured production answers `404`; a wrong password is `401`
  with `WWW-Authenticate`. `--dev` stays open.

### ORM

- **Column-mode `encrypts` and STI.** A `table "…"` model can encrypt text
  columns (AES-256-GCM, same key and format as the document path) and share
  that table across subclasses with a string `type` discriminator. Subclass
  queries add `type IN (class, descendants)` on the real column; `find` /
  `find_by` / `first_by` on a subclass refuse a row of another type. Boot
  fails if an encrypted field is missing or not text, or if an STI subclass's
  table has no `type` column. `create`/`save` decrypt adopted rows so the
  in-memory instance matches a subsequent `find`. Composite primary keys are
  still refused.

### Performance

- **Hash get/set.** Overwriting an existing key (`h[k] = v` / `h.set(k, v)`)
  no longer clones the key on a hit. The VM folds `h[keys[i]]` and
  `total = total + h[keys[i]]` into one opcode and no longer calls the cold
  span helper on a successful `[]` / `[]=`. Controller reads
  `req["params"]["id"]` compile to one `HashGetLocalConst2` instead of two
  hash gets. Request plumbing: `req["all"]`/`req["cookies"]` probes use a
  borrowed key (no `String` per lookup), `req["all"]` reuses the params hash
  when there is only one source, request-key construction is process-static
  (no TLS), helper `req` rebinding is skipped when no view helpers are
  loaded, and middleware no longer calls `Instant::now` unless metrics or
  the dev middleware log is on. No-middleware requests skip republishing
  `params`/`cookies` (already set) and skip stashing `*_url` host when no
  named routes exist.

### Security

- **`h2` bumped to 0.4.18 — RUSTSEC-2026-0258 (unbounded empty DATA frames).**
  A peer could stream empty HTTP/2 DATA frames without bound, so the server
  side of our own hyper 1.x / reqwest stack was a DoS target. The bump clears
  it; no Soli-facing behavior changes. The second flagged copy — `h2` 0.3.27,
  pinned by hyper 0.14 under `rusoto_core` — has no patched 0.3 release and is
  waived in `.cargo/audit.toml`: it is client-only, reached solely by the S3
  builtins talking to AWS, and goes away with the rusoto → aws-sdk-s3
  migration.
- **The jobs dashboard was exempt from CSRF.** The whole reserved `/__soli/`
  namespace skipped both barriers, and `POST /__soli/jobs/<id>/retry` is
  production-reachable behind Basic auth — which a browser attaches
  automatically, so any other site could forge it against a logged-in operator.
  `/__soli/jobs` keeps both barriers now; its own forms are same-origin.
- **`SOLI_FORCE_SECURE_COOKIES` was bypassed by the two-argument `set_cookie`.**
  That form hardcoded its attribute string and skipped the builder that applies
  the flag, so `set_cookie("remember_me", token)` shipped a credential with no
  `Secure` on an app declared TLS-only.
- **OpenTelemetry span names carried credentials off-box.** An outbound HTTP span
  was named from the raw URL while the query panel beside it was scrubbed. Span
  names go through the same scrubber now — which itself had two bugs: its
  userinfo check required `<` where the two offsets it compared are always
  equal, so `user:password@` was never stripped; and it returned early, so a URL
  with both userinfo and an `?api_key=` kept the key.
- **`create_many` stored `encrypts` fields as plaintext on SQL.** The bulk-insert
  branch bypassed the write layer that applies the transform, so a model with
  `encrypts("ssn")` persisted raw values, and reads still looked correct because
  the loader leaves non-ciphertext untouched. The bulk path now encrypts each row
  (and picks the bulk path per the model's own `connection`, not the ambient one).
- **`has_*_attached` accepted and re-served `text/html`.** An absent
  `content_types` meant "anything", and the blob route echoed the client-declared
  type with no `nosniff` — a bare `has_one_attached("avatar")` was stored XSS on
  the app's origin. They now default to a curated allow-list that excludes every
  script-executing type (no `text/html`, no `image/svg+xml`, no XML), and the blob
  route sends `X-Content-Type-Options: nosniff` plus `Content-Disposition:
  attachment` for anything that is not an inline-safe image. Since those
  disposition headers are the primary defence, the list stays as wide as it
  safely can — images including BMP and TIFF, PDF, plain text, Markdown, CSV,
  JSON, Zip (both MIME spellings), the Office formats, common audio/video.
  **Upgrading:** this is still a change from "anything", so an app storing a
  type outside the list must name it in `content_types`.
- **A locked account could never lock again.** In the generated auth stack,
  `register_failed_attempt` returned early whenever `locked_at` was set, and only
  `locked?()` cleared an expired stamp — which ran after a *successful* login. The
  attempt counter froze permanently, so once the first window elapsed an attacker
  guessed at full rate forever. The guard asks `locked?()` now, which re-arms the
  lockout without sliding the window under sustained attack.
- **Generated OAuth clients could not authenticate, and their PKCE was `plain`.**
  The services passed `"headers"` to `HTTP.get`/`HTTP.post`, which read that hash
  for `timeout` only and return the body as a String — so the `Authorization`
  header was dropped and `response["body"]` was a type error. They use
  `HTTP.request(method, url, headers, body)` now and check the status. PKCE sends
  a real `S256` challenge instead of the raw verifier.
- **`live_component` interpolated its child id into HTML unescaped.** The id is
  application data (`live_component("row", {"id": row.slug})`) and reached the DOM
  through `morph` → `innerHTML`, so a quote could add a live event handler. Ids
  are attribute-escaped, and one containing a control character (which would split
  the line the patch engine splices on) is refused at the call.
- **A panic inside an adapter pinned the worker to the wrong database.**
  `with_connection` restored the previous target only on the normal path, so a
  caught panic left the worker reading *and writing* the secondary database for
  the rest of its life. The restore is a `Drop` guard.
- **`soli db:drop` could silently drop the default database.** Its flag loop
  ignored anything unrecognised, so `soli db:drop --connection` with the value
  forgotten dropped the default without a word. `db:create`, `db:drop`,
  `db:schema:dump` and `db:schema:load` exit 64 on an unknown or value-less flag.
- **Chunked LiveView uploads had no cap.** `put_chunk` pruned only by TTL,
  so unlike the completed-file store it enforced neither a global nor a
  per-session limit. `POST /live/upload` is reachable without a session
  (a first-time visitor may not have one yet), so one unauthenticated
  client could mint a fresh `X-Soli-Upload-Id` per request, send a single
  chunk of a declared 512, and park up to 8 MiB per id until memory ran
  out. Now: 4 in-progress uploads per session, 32 and 64 MiB globally, and
  a 2-minute *idle* deadline refreshed by each accepted chunk (so a slow
  transfer is never cut off). Over the limit returns `413`. A resent chunk
  replaces its slot rather than being counted twice.

- **The CSRF exemption covered application routes.** Both barriers — the
  Origin/Referer gate and per-form token verification — skipped any path
  starting with `/_`. That namespace is one applications use, so a
  `POST /_internal/wipe` or `/_admin/users` lost both checks with nothing
  asked for. The exemption is now the endpoints the framework serves:
  `/_health`, `/_ready`, `/_metrics`, `/__coverage__`, and the reserved
  `/__soli/`, `/__solidev/`, `/__dev/`, `/__livereload` prefixes.
  `skip_csrf("/path")` remains the way an application route opts out.

- **`SOLI_FORCE_SECURE_COOKIES` only covered `session_id`.** Every cookie
  set from application code — the scaffolded remember-me token above all,
  a 30-day bearer credential — was emitted without `Secure` even on a
  deployment that had declared itself TLS-only. The switch now applies to
  the whole jar. `set_cookie(..., {"same_site": "None"})` also adds
  `Secure` on its own: browsers drop such a cookie without it, so the old
  behaviour produced a cookie that silently never arrived.

- **`soli generate auth` sign-in was a user-enumeration oracle.** It
  returned early when no account matched, so a miss answered in
  microseconds against ~100 ms of Argon2id for a real attempt, and it
  showed a distinct "account locked" message *before* checking the
  password. The miss path now spends the same hashing work
  (`User.burn_password_work`), both failures share one message, and the
  lockout message is held until the password verifies.

- **No rate limiting on the credential endpoints.** Account lockout is
  blind to credential stuffing spread across many accounts, and was itself
  a denial of service: ten guesses locked any known address, and each
  further guess re-stamped `locked_at`, extending the lockout
  indefinitely. Sign-in, sign-up, and password-reset are now throttled per
  source address (`429` over the limit), and an existing lockout is no
  longer re-stamped. The three share one budget —
  `AUTH_ATTEMPTS_PER_IP` per `AUTH_IP_WINDOW_SECONDS` (default 15 per
  5 min) is the total across all credential traffic from that address, not
  15 each. `rate_limiter_from_ip` keys on the peer address unless
  `enable_trust_proxy()` is on, so a rotating `X-Forwarded-For` cannot
  bypass it.

- **`rate_limiter_from_ip` rejected its own documented third argument.**
  It was registered with an exact arity of 2 while the body read
  `args.get(2)` for the window, so the documented
  `rate_limiter_from_ip(req, limit, window_seconds)` failed with "Wrong
  number of arguments: expected 2, got 3" before running. It is variadic
  now and checks the count itself; a non-integer window raises instead of
  silently falling back to 60 seconds.

- **The generated User model had no password policy.** A one-character
  password was accepted at sign-up while the reset flow enforced an
  unrelated 8-character minimum. `User#password_error` now owns the rule
  (`AUTH_MIN_PASSWORD_LENGTH`, default 12) and both flows call it.
  `finish_password_reset` also clears `remember_token_digest` — a reset is
  what someone does when they suspect compromise, and a remember-me cookie
  outliving it by 30 days means the reset evicted nobody.

- **`auth_base_url` reads `APP_BASE_URL`.** It was a hardcoded
  `http://localhost:5011` behind a `TODO`, so a deployment that missed the
  comment mailed reset and confirmation links pointing at localhost — over
  plaintext HTTP if the host resolved at all. The literal remains as a dev
  fallback.

- **`cargo audit --deny warnings` runs in CI.** An advisory against a
  transitive dependency is invisible to code review and leaves the suite
  green. RUSTSEC-2026-0253 (`lru`, unsound) is ignored with a comment: it
  is patched in 0.18.2, which our direct dependency already resolves to,
  and the two flagged copies are pinned by ratatui 0.26 and
  azul-layout / mysql 28.

### Added

- **First-class attachments.** `has_one_attached("avatar")` /
  `has_many_attached("photos")` default to disk (`./storage/attachments`,
  `SOLI_ATTACHMENTS_PATH`) or `service: "s3"` / `"solidb"`. Same
  `attach_` / `detach_` / `_url` methods as `uploader(...)`. Destroy
  purges blobs. LiveView `soli-upload` hashes attach directly.

- **LiveView chunked uploads and `send_update`.** Files over 256 KiB
  POST to `/live/upload` in chunks (`X-Soli-Upload-Id` / chunk index /
  count). `send_update(component, assigns)` stores child state under
  `_components` and, when the child has `router_live`, runs that handler
  with `event == "update"` (Soli's `update/2`). A bare hash still merges
  onto the parent.

- **LiveView rooms share one instance across tabs and visitors.**
  `data-live-room="name"` on the mount sends `?room=name`; the server keys
  `room:name:component` instead of `session:component`. A public demo no
  longer looks like a different session when the WebSocket upgrade has no
  cookie (each socket used to mint a unique `sess-<uuid>`). The Field Desk
  blog widget uses `field-desk`.

### Fixed

- **Docs described an assertion API that does not exist.** The testing pages
  documented `assert_equal` / `assert_true` / `assert_false` / `assert_nil` /
  `assert_not_nil`, each taking a trailing `message`, and claimed assertions
  return a `{passed, message, expected, actual}` hash. None of that is real: the
  vocabulary is `assert`, `assert_not`, `assert_eq`, `assert_ne`, `assert_null`,
  `assert_not_null`, `assert_gt`, `assert_lt`, `assert_match`, `assert_contains`,
  `assert_hash_has_key`, `assert_json` — values only, raising on failure and
  returning `1`. Rewritten across `testing-assertions.md`, the Testing and
  Testing-Functions pages, and the scaffold page.

- **`soli generate scaffold` wrote `describe("UsersControllerController")`.** The
  caller appended `Controller` to a name the template already suffixes.

- **Docs told you to run scripts with `soli run file.sl`**, which fails with
  "Only one script file can be specified" — the invocation is `soli file.sl`.
  Fixed in ten places, including the live-reload page (which also documented a
  `SOLI_ENV` variable nothing reads, and a `./dev.sh` the template never shipped)
  and the error-pages page (which documented a `--no-dev` flag that does not
  exist; production is simply the absence of `--dev`).

- **Docs documented `.add_weeks()` / `.add_months()` / `.add_years()`** on
  DateTime, none of which exist, and omitted `.add_minutes()`, which does. The
  page now says why month and year steps are absent and shows what to do instead.

- **Docs documented a `Math` namespace and bare math functions.** There is no
  `Math.floor` / `Math.random` / `Math.pi` (nor trigonometry, logarithms, or
  exponentials anywhere), and `abs(n)` / `min(a, b)` / `sqrt(n)` are not free
  functions — they are methods: `(-5).abs()`, `[3, 7].min()`, `(16).sqrt()`.

- **Five docs links served a JSON blob instead of a page** (`/docs/jobs`,
  `/docs/authorization`, `/docs/core-concepts/models`, `/docs/language/linting`,
  `/docs/core-concepts/testing`), each falling through to the catch-all route.
  Every internal `/docs` link in the site now resolves.

- **Docs search returned entries for API that does not exist** — twelve `Math.*`
  functions, plus two pointing at a removed `/docs/sdbql-graphs` page.

- **The SQL adapter env vars were missing from the Configuration page.**
  `SOLI_DB_ADAPTER`, `DATABASE_URL`, and `SOLI_DB_POOL_SIZE` were documented in
  `configuration.md` but not on the page that mirrors it, whose Database section
  listed only the SoliDB variables.

- **The hash-filter operator symbols were undocumented.** `>`, `>=`, `<`, `<=`,
  `==`, `=`, `!=`, `<>` are accepted alongside the names; they now appear in the
  Models / Query-builder operator reference and in the unknown-operator error.

- **LiveView instances are released when their last socket closes.** Nothing
  ever unregistered one, so every `session:component` pair kept its state, its
  full last render, and its live-query subscriptions for the process lifetime —
  and writes kept waking views whose browser was gone (a handler run, a render
  and a diff per wake). Subscriptions are dropped at close, state is held for
  two minutes so a refresh or blip reclaims it, then reaped by a sweep.

- **LiveView frames no longer lose each other's state.** A tick and a client
  event each cloned the instance, mutated the clone and wrote it back
  (last-writer-wins), and the loser's stale render became the next diff base —
  so the client's shadow was diffed against markup it never received. Frames of
  one instance are serialized, and a frame finishing after its socket closed no
  longer re-creates the instance.

- **LiveView events are scoped to the sending socket.** Dispatch used the
  `liveview_id` in the client's message, so a client could drive another
  component of its own session — or, with a known session id, another user's
  view, which then received the patch.

- **A raising LiveView handler no longer runs the built-in demo state machine.**
  A handler error or unexpected return fell through to the counter/metrics
  fallback, so an app bug looked like "the counter incremented". It now pushes
  an error to the client (message only under `--dev`) and leaves state intact;
  returning nothing means "no state change".

- **LiveView uploads are bound to the uploading session**, with 8 pending slots
  per session. A stored id was redeemable by anyone holding it, and one client
  could fill the 64-slot × 8 MiB global store (≈512 MiB) and lock everyone else
  out of uploads.

- **A reconnected LiveView keeps ticking.** The tick task was aborted at
  disconnect while the instance still remembered its interval, so the handler's
  request looked unchanged and was skipped — a ticking view stopped for good
  after any blip.

- **LiveView render errors no longer leak server paths and are escaped.** A
  missing template sent the four absolute paths it tried to the browser, and the
  error markup was interpolated, so a component name could carry markup into the
  page.

- **The LiveView heartbeat ack is actually sent**, and one type owns its wire
  shape (the hand-written JSON had drifted from it). The ack and the event
  enqueue no longer block the socket's read loop — the enqueue used a blocking
  send that parked a runtime thread when the worker pool was saturated.

### Added

- **LiveView patches every tab of the same session.** A second tab
  attaches another sender to `session:component` instead of replacing
  the first. A click in one tab patches the others; closing one tab
  does not detach the rest.

- **Field Desk LiveView tutorial.** A blog post at `/docs/blog/liveview-desk`
  with the widget on the page: nested `live_component` assigns, `soli-upload`,
  in-socket tabs, debounce, click-away, hooks, and JS commands — plus the hash
  `.where` / jobs snippets you would ship next to it.

- **LiveView debounce/throttle, JS commands, and navigation.**
  `soli-debounce` / `soli-throttle` (ms) on any event element; window-level
  `soli-window-keydown` / `soli-window-keyup`; `soli-href` for a full-page
  leave. A handler may return `redirect: "/path"` (client navigates, no
  further patch) or `js: [{ op, to, ... }]` — eval-free commands
  (`add_class` / `remove_class` / `toggle_class`, `set_attr` /
  `remove_attr`, `focus`, `dispatch`, `navigate`, `patch`). Uploads and
  nested live components remain out of scope.

- **LiveView hooks, loading states, and click-away.** `soli-hook="Name"`
  binds a client hook (`mounted` / `updated` / `destroyed` /
  `disconnected` / `reconnected`, plus `this.pushEvent`). Register on
  `SoliLiveView.hooks` before connect, or pass `{ hooks }` to `live()`.
  In-flight events add `soli-loading` / `soli-<event>-loading`;
  `soli-disable-with` swaps the label and disables the control until the
  next patch. `soli-click-away` fires when a click lands outside the
  element.

- **LiveView in-socket navigation and nested child sockets.**
  `soli-patch="/path?q=1"` updates the address bar and sends
  `event == "patch"` with `href` / `path` / `query` / `hash` (browser
  back/forward too). A handler may return `patch: "/path"` or
  `patch: { "url": "/path", "replace": true }`. Nested
  `[data-liveview-url]` mounts inside a parent become their own sockets
  after each render (implicit `soli-ignore` so a parent patch does not
  wipe them). Phoenix-style `live_component` assigns remain out of scope.

- **LiveView file uploads.** `soli-upload="handler"` on a file input
  POSTs each file to `/live/upload` (multipart + CSRF; 8 MiB default,
  override with `soli-upload-max`) and then sends the handler
  `params["file"]` / `params["files"]` in the same shape as
  `find_uploaded_file` (`data` is base64). Progress is
  `soli-upload-loading` + `data-soli-progress`. Not chunked or
  resumable.

- **LiveView nested components share parent assigns.**
  `live_component("score", { "score": true })` renders the child template
  with parent-owned keys and wraps it in `soli-component`. A child event
  may send `soli-assign-*`; the runtime merges `_assigns` onto the parent
  (typed) before the handler runs, then the next render fans values back
  down. Independent `[data-liveview-url]` sockets stay isolated.

- **LiveView root swap and reconnect restore.** `soli-live="/live/socket/name"`
  (or handler `{ "live": "/live/socket/name" }`) disconnects this socket
  and connects another component on the same root, optionally updating
  the URL via `href` / `soli-patch`. A dropped socket reconnects with
  the previous instance state (same `session:component` id) so the
  connect handler sees in-flight values.

- **Standalone job worker and queue dashboard.** `soli jobs` (alias
  `soli worker`) loads the app, claims `_jobs`, and runs them with no
  HTTP listener — pair with `SOLI_JOB_WORKERS=0` on `soli serve` to scale
  workers separately. `soli jobs list [--queue] [--state]`, `retry <id>`,
  and `cancel <id>` inspect the queue. `--dev` adds `/__soli/jobs` (dev-bar
  tools panel) for the same cancel/retry. `Job.retry(id)` re-queues a
  `failed` or `dead` row and keeps `attempts` / `last_error`.

- **Dedicated Postgres, MySQL, and SQLite docs pages.** Adapter-specific notes
  (URL, create/drop, JSON storage, indexes, jobs, schema dump, column-mode
  types, honest limits) now live at `/docs/database/postgres`,
  `/docs/database/mysql`, and `/docs/database/sqlite` — markdown and the docs
  site, linked from Multiple Databases and the sidebar.

- **Portable hash `.where` comparisons, `IN`, `LIKE`, and `OR`.** The hash form
  used to be equality-only (`{ "status": "open" }`); anything richer had to be
  raw SDBQL, which SQL adapters refuse — and a `{ "gt": 10 }` silently became
  `==`. The hash now compiles through a structured IR on SoliDB, SQL document
  tables, and column-aware models:

  - comparisons: `{ "total": { "gt": 100, "lte": 999 } }` (`gt`/`gte`/`lt`/`lte`/`eq`/`ne`, plus `>`/`>=`/…)
  - `IN`: `{ "id": [1, 2, 3] }` or `{ "id": { "in": [1, 2, 3] } }` (an empty list matches nothing)
  - `LIKE` / `ILIKE`: `{ "email": { "like": "%@x.com" } }`
  - grouping: `{ "or": [{ "state": "draft" }, { "state": "open" }] }`

  The string form remains for expressions the vocabulary cannot express.

- **Filtered `.includes` on SQL.** `.includes("comments", { "visible": true })`
  and `.includes("comments", { "where": { "n": { "gt": 1 } } })` apply the same
  hash vocabulary to the related rows — on document tables (after the batched
  fetch) and on column-aware models (pushed into the `IN` query). A raw string
  filter on `.includes` stays SoliDB-only and is refused with the hash shape
  named.

- **`soli db:schema:dump` / `soli db:schema:load`.** Dump writes `db/schema.sql`
  — dialect SQL for every user table and index, plus the applied migration
  versions in a header — so a fresh database can be built without replaying
  every file. Load runs that SQL and records the versions in `_migrations`.
  Both accept `--connection NAME`. SQLite dumps `sqlite_master`; MySQL uses
  `SHOW CREATE TABLE`; Postgres reconstructs `CREATE TABLE` from introspection
  plus `pg_indexes`.

- **Column-mode `through:` / HABTM includes, `.join`, and `.having`.** A
  `table "…"` model can now eager-load `has_and_belongs_to_many` and
  `has_many through:` (the join / intermediate table must also be
  column-aware), filter parents with `.join("comments")` (correlated
  `EXISTS`), and filter groups with `.having("n > 5")`.

- **`through:` includes, `.join`, and `.having` now run on the SQL adapters.**

  - **`.includes` on a `has_many through:`** batches into three queries whatever
    the parent count: the intermediate rows for these parents, the targets those
    rows point at, then grouping in Rust. It turns out `through:` eager loading
    was refused at the *builder*, so it had never worked on SoliDB either; the
    check moved to execution, where the AQL shape still declines it and the SQL
    path serves it.
  - **`.join("comments")`** compiles to a correlated `EXISTS` subquery rather than
    a real join, so parents are not duplicated when a child matches twice and the
    `SELECT doc` shape is untouched. A child-side filter rides inside the
    subquery, in the portable hash-equality shape.
  - **`.having("n > 5")`** compiles to a `HAVING` clause. The supported shape is
    one comparison of a group key or aggregate alias against a number; the
    aggregate expression is repeated rather than its alias, because Postgres does
    not accept an alias there. An unknown alias is refused listing the ones the
    query emits, and anything richer is refused naming the supported shape rather
    than being passed through as SQL.

- **`Model.find_by_sql(sql, binds?)`** — the escape hatch for a query the
  portable surface cannot express. `BackendCaps.raw_sql` had been set to true on
  the SQL adapters since they shipped while nothing exposed raw SQL at all; the
  flag finally means something. Binds are positional and always bound, never
  interpolated. A single `doc` column hydrates as documents, so instances come
  back as usual; any other shape becomes a hash per row. Raises on SoliDB,
  pointing at `Model.query` with SDBQL.

- **`create_many` is one statement per chunk on SQL**, instead of one statement
  (and on SQLite one transaction) per row. Chunked at 500 rows — each row takes
  two binds and Postgres allows 65535 per statement — and wrapped in a single
  transaction so a partial failure cannot leave half a batch behind. Re-running
  upserts, matching single-row `insert`. The per-item `attr_accessible` filter
  still runs on every row: bulk insert would otherwise be a perfect mass-
  assignment bypass.

- **`pluck` / `select` push their projection into the `SELECT` list** on
  column-aware models, so a projection on a wide table reads two columns instead
  of fifty. The primary key is always included so the row stays identifiable, and
  a field that is not a real column falls back to the client-side projection.

- **A column added by `ALTER TABLE` no longer needs a restart.** When a query
  names a column the cached schema does not have, the column path re-introspects
  that one table and retries once; a genuine typo still reports the original
  "unknown field" error with the real column list.

- **Column-aware models reached association parity.** A model bound to an
  existing table with `table "…"` could only do single-row and scalar work; the
  interesting half of the ORM refused to run. Now supported there:

  - **Batched eager loading** for `belongs_to`, `has_many`, `has_one`, and
    `includes_count` — one query per association whatever the parent count, using
    `column IN (…)` over the real foreign-key columns. Measured in a live app:
    3 parents with `includes("books")` costs **2 queries**, not 4. A parent with
    no children gets `[]`, never null. Both sides must be column-aware; joining a
    real column to a JSON field is refused with a message naming both models
    instead of quietly returning nothing.
  - **`group_by`** with sum/avg/min/max/count over real columns (a non-numeric
    aggregate is refused by name), and **`delete_all` / `update_all`**, which
    stamp `updated_at` when the table has it and never rewrite the primary key.
  - **`soft_delete`**, when the table actually has a `deleted_at` column — the
    scope becomes an ordinary `IS NULL` / `IS NOT NULL` filter. Declaring it on a
    table without that column now fails at boot naming the missing column,
    instead of the declaration being rejected outright.
  - **Counter caches**, which follow from the atomic column increment above.

  Still out at the time: composite primary keys, `encrypts`, and STI
  (`encrypts` and STI now land in column mode; see below).

- **The dev bar, `dev_queries()`, and N+1 detection now work on the SQL
  adapters.** Only the SoliDB path wrote to the per-request query log, so on
  Postgres/MySQL/SQLite the DB panel was empty, timings were missing, and the
  N+1 badge, `assert_no_n_plus_one`, and `soli test --fail-on-n1` could never
  fire — the framework's own N+1 guard was blind on three of four backends.
  Every statement now records its SQL, its binds (numbered as `$1` / `?` appear
  in the statement) and its duration, plus a `Db` span so the flamegraph shows
  database time. Verified in a live `--dev` app: the badge reads `5q`, the panel
  lists the real SQL, and a deliberate per-row lookup reports
  "N+1 DETECTED · 2 TEMPLATES". Bind values over 200 characters are truncated
  with their real length noted. The Prometheus DB-time counter is fed on this
  path too, so production gains SQL timings even with the dev log off.

- **`soli db:create` / `soli db:drop`.** SoliDB creates its database on first
  use; a SQL server does not, so pointing `DATABASE_URL` at a database nobody
  created failed at boot with a driver error. `db:create` runs `CREATE DATABASE`
  (through the `postgres` maintenance database on Postgres, a db-less connection
  on MySQL) or creates the SQLite file and its parent directory; `db:drop`
  removes it, including the `-wal`/`-shm` sidecars on SQLite, which would
  otherwise resurrect committed data into the next file of the same name. Both
  take `--connection NAME`.

- **`index` declarations now work on the SQL adapters, and the planner uses
  them.** A document table had no indexes at all: `index_sync` reconciled
  declarations through SoliDB's HTTP index API, which *refuses* on a SQL
  connection, and nothing else issued index DDL. Every `.where({ status: … })`
  was a sequential scan plus a per-row JSON extract.

  - `index "status"` now creates an expression index on the JSON field —
    `((doc->>'status'))` on Postgres, `((doc ->> '$.status'))` on SQLite, and on
    MySQL a generated `STORED` column plus an index on that, since MySQL cannot
    index a JSON extract directly. Multi-field and `unique:` declarations work
    the same way; reconciliation stays idempotent by name.
  - **String equality now compiles to the same expression the index holds**, so
    the index is actually used. Numbers and booleans keep exact JSON comparison
    (`10` still matches a stored `10.0`), which no expression index covers — the
    trade-off is documented rather than silently chosen.
  - Migrations can create one directly with a `doc.` prefix:
    `db.add_index("posts", ["doc.status"], { "unique": true })`.
  - **The job queue indexes itself** on first enqueue (`state`, `run_at`,
    `priority`, plus `next_run_at`/`enabled` for cron). The claim query runs
    every poll tick and previously scanned every job ever enqueued — failed and
    dead rows are kept deliberately, so that table only grows.
  - `fulltext`/`bloom`/`cuckoo` index types and `vector_index`/`geo_index` are
    SoliDB engine features; on SQL they are now reported as skipped instead of
    failing with a confusing adapter error.

- **Migrations can build real column tables, portably.** Column-aware models
  could read and write an existing relational schema, but nothing in Soli could
  *create* one — migrations only produced `_key` + `doc` document tables, so a
  greenfield column schema had to be built with psql, the `sqlite3` CLI, or
  another framework's tooling. Migrations now speak columns:

  ```soli
  def up(db)
    db.create_table("orders", {
      "id":         "pk",
      "code":       { "type": "string", "limit": 32, "null": false },
      "amount":     "decimal(10,2)",
      "paid":       { "type": "boolean", "default": false },
      "meta":       "json",
      "user_id":    { "type": "bigint", "references": "users" },
      "timestamps": true
    })
    db.add_index("orders", ["code"], { "unique": true })
  end
  ```

  - **One migration, three backends.** The types are Soli's (`pk`, `uuid_pk`,
    `string(n)`, `text`, `integer`, `bigint`, `float`, `decimal(p,s)`,
    `boolean`, `date`, `datetime`, `json`, `uuid`, `binary`) and each adapter
    renders its own SQL. The rendered names are chosen so introspection reads
    the table back as the same Soli types — a table created this way is always
    one a column-aware model can map, which a test pins for all three dialects.
  - New helpers: `add_column`, `drop_column`, `rename_column`, `rename_table`,
    `add_index`, `drop_index`, the SoliDB-shaped `create_index`, and
    `execute(sql)` as the engine-specific escape hatch. `create_table(name)`
    with no column hash still means a document table, unchanged.
  - **Portability details handled rather than papered over**: MySQL parses an
    inline `REFERENCES` and then ignores it, so foreign keys are emitted at
    table level there; MySQL takes no `IF NOT EXISTS` on `CREATE INDEX` and
    needs the table on `DROP INDEX`; SQLite cannot add a `UNIQUE`/primary-key
    column or a `NOT NULL` column without a default to an existing table, and
    says so with the way around it. A composite primary key is refused at
    parse time, because column mode could not map the result.
  - `db:migrate generate` now shows the column DSL in its template.

- **A migration can declare which database it belongs to.** Put
  `connection "analytics"` at the top of the file and `soli db:migrate up`
  places it correctly — no `--connection` flag, no chance of running it against
  the wrong schema.

  - **Each connection tracks its own versions**, so a migration applied to one
    database is not marked applied on another.
  - `--connection NAME` becomes a *filter*: migrations that declare a different
    database are held back and reported as skipped.
  - `db:migrate status` grows a Connection column as soon as one migration
    declares one; `down` rolls back the newest applied migration on its own
    connection.
  - The declaration is metadata, not a statement — it is neutralized before the
    file executes (at top level it would otherwise call the model DSL's
    `connection` builtin with the wrong arity). It must be the first
    non-comment statement; a line inside a string or after `def` is ignored,
    and a second declaration is an error.

- **SQLite adapter.** `SOLI_DB_ADAPTER=sqlite` with
  `DATABASE_URL=sqlite://db/app.sqlite3` (or `adapter = "sqlite"` on a named
  connection) runs the **same Model surface as Postgres and MySQL**: document
  CRUD, hash `.where`, order/limit/offset, count/exists, aggregates, `group_by`,
  bulk writes, soft-delete, batched `.includes` (HABTM included),
  `Model.transaction`, SQL migrations, and background jobs + cron. A connection
  is a path — no server to install, start, or credential — and the client is
  compiled in, so the host needs no `libsqlite3`.

  - **Column mode works too**: `table "orders"` maps a model onto an existing
    SQLite schema, introspected with `PRAGMA table_info`. SQLite enforces no
    types, so Soli reads the declared type to decide how to convert and reads
    each value by what it actually is — a `DATETIME` holding a unix timestamp
    (seconds or milliseconds) reads as a date, like stored text does. An
    `INTEGER PRIMARY KEY` is recognised as database-generated (it aliases the
    rowid, with or without `AUTOINCREMENT`).
  - **Job claiming** has no `SKIP LOCKED` to lean on, so it takes the database
    write lock (`BEGIN IMMEDIATE`) for the length of the select-then-update —
    exclusive by construction. Leases, retries, backoff, and single-winner cron
    firing are unchanged.
  - **Defaults**: WAL, a 10s busy timeout, foreign keys on, `BEGIN IMMEDIATE`
    transactions (a deferred one can fail to upgrade a read into a write),
    `sqlite::memory:` pinned to one pooled connection, and the file's parent
    directory created if missing.
  - **Caveats are SQLite's own**: one writer at a time (a write-heavy
    multi-process app belongs on Postgres), and no exact numeric type —
    a `DECIMAL` column has NUMERIC *affinity*, so SQLite stores the value as a
    `REAL` and `19.90` reads back as `19.9`. Postgres `numeric` and MySQL
    `decimal` keep the scale; declare the column `TEXT` if the exact text
    matters.
  - New Cargo feature `sqlite` (on by default, included in the `sql` alias).
    Tests need no server, so the adapter is covered on every CI run.

- **Batched HABTM `.includes` on Postgres and MySQL.** A
  `has_and_belongs_to_many` eager load was SoliDB-only; it now runs on the SQL
  document adapters in **two queries regardless of parent count**: one for the
  join-table rows whose owner key matches the parents, one for the distinct
  targets those rows point at. Targets attach in the order the join rows came
  back, a parent with no links gets `[]` rather than null, and a dangling join
  row (a link to a deleted target) contributes nothing instead of a null hole.
  `includes_count` follows the same path, so it counts join rows for HABTM.
  `through:` includes, `.having`, and `.join` now run on SQL as well — see the
  entry above.

- **Column-aware models — use Soli against an existing relational database.**
  Until now the SQL adapters were a document store *on top of* SQL: every table
  was `_key` + `doc JSONB`, so pointing a connection at a real schema failed on
  the first query ("column doc does not exist"). A model can now bind to an
  existing table and read/write its **real columns**:

  ```soli
  class Order < Model
    connection "legacy"    # a postgres or mysql connection
    table "orders"         # bind to an existing table -> column mode
  end
  ```

  - **Schema by introspection.** At boot Soli reads `information_schema` once
    per table and caches it: columns, types, nullability, and the **primary key**
    — including whether the database generates it (`BIGSERIAL` / `IDENTITY` /
    `AUTO_INCREMENT`), so inserts leave it alone. `created_at`/`updated_at` are
    stamped only when those columns actually exist.
  - **Full CRUD**: `find` (Int or String key), `find_by`/`first_by`, hash
    `.where` (including `{field: null}` → `IS NULL`), `order`/`limit`/`offset`,
    `count`/`exists`, `sum`/`avg`/`min`/`max`, `create`/`save`/`update`/`delete`,
    `pluck`/`select`, and `Model.transaction` (which works because the column
    path reuses the same held connection as the document path).
  - **Never issues DDL.** Column mode maps to a schema Soli does not own, so
    there is no auto-create, no migration, and no index sync against it.
  - **Fails loudly, never silently.** A missing table, composite primary key,
    keyless table, or `solidb` connection fails at boot naming the connection and
    table. Writing an unknown field, filtering an unsupported column type, or
    summing a text column errors with the column and its type. Unsupported
    features (associations/`.includes`, `group_by`, `delete_all`/`update_all`,
    soft-delete, `encrypts`, STI) each raise a message naming the feature and
    listing what does work.
  - Doc-store models on the same connection are untouched: column mode is
    per-model, and the document compiler was left byte-for-byte alone (the column
    path is a separate IR, `src/db/sql_columns_compile.rs`).

- **Soli-side background job engine** — jobs now run *inside* the Soli process
  instead of being delegated to SolidB's queue and delivered back through a
  signed webhook. Jobs are rows in a `_jobs` collection on the default
  connection, so the engine works identically on **SolidB, PostgreSQL, MySQL,
  and SQLite** (apps on the SQL adapters could not run jobs at all before).
  - A poller thread claims due work **atomically** — Postgres
    `UPDATE … FOR UPDATE SKIP LOCKED`, MySQL a single-statement token claim,
    SolidB an `If-Match` compare-and-swap — so several `soli serve` processes can
    share one queue without ever double-running a job.
  - Execution happens on the worker pool (`SOLI_JOB_WORKERS`, default 1), never
    on a web worker, so a slow handler can no longer delay request serving.
  - Soli owns **retries**: exponential backoff from 5s, doubling, capped at 1h
    with per-job jitter; `attempts` increments at claim time; a `running` row
    whose lease expires is reclaimed, which is how a killed process's work
    recovers. Completed rows are pruned after `SOLI_JOBS_RETENTION_SECS`.
  - **Cron runs in Soli too**: `Cron.schedule/list/update/delete`, the
    `Cron.every/hourly/daily_at/weekly_at` builders, and `static cron` are
    evaluated with the `cron` crate and fired via a compare-and-swap on the
    schedule's `next_run_at`, so exactly one process fires each occurrence.
    Invalid expressions are now rejected at declaration time (with a message
    naming the six-field shape) instead of silently never firing.
  - `Webhook.enqueue/enqueue_in/enqueue_at` is a built-in job type delivered by
    the engine, with the same `X-Webhook-Signature` / `X-Webhook-Event` /
    `X-Webhook-Delivery` headers receivers already verify — and now with retries,
    on every adapter.
  - **`XJob.perform_now(args)`** runs a handler inline with no queue, worker, or
    database — the documented-but-missing method that makes jobs unit-testable.
  - New env: `SOLI_JOBS_POLL_MS` (1000), `SOLI_JOBS_LEASE_SECS` (60),
    `SOLI_JOBS_MAX_RETRIES` (3), `SOLI_JOBS_RETENTION_SECS` (604800).
  - `soli new` now creates `app/jobs/`, so a generated app can add a job without
    also having to create the directory the engine looks for.

### Fixed

- **`validates uniqueness:` raised on every SQL adapter.** Its pre-check ran a
  raw SDBQL query, so declaring it made `create`/`save` fail with "Raw SDBQL
  queries are SoliDB-only" before any row was written. It now uses the portable
  hash-filter path (and the column path for a column-aware model).

- **Constraint violations arrived as driver text instead of field errors on SQL.**
  Detection matched only SoliDB's `HTTP 409`, so a duplicate on Postgres/MySQL/
  SQLite surfaced as e.g. `sqlite column insert row: UNIQUE constraint failed:
  orders.code` in `_errors`. Each adapter now classifies its own driver error —
  Postgres by SQLSTATE, MySQL by error number, SQLite by extended result code —
  and the model layer turns it into `{ field, message }`: "has already been
  taken", "must reference an existing record" (foreign key), "can't be blank"
  (NOT NULL), "is invalid" (CHECK). The field comes from whatever the database
  names, falling back to `_base`. Anything that is not a constraint violation is
  still reported as-is, so a connection failure cannot masquerade as a validation
  error.

- **Postgres errors said "db error" and nothing else.** `postgres::Error`'s
  `Display` is that literal string; every message users saw was our context plus
  those two words. Errors now carry the driver's real message and `DETAIL`, which
  is also where the offending column comes from.

- **Concurrent `increment` / `decrement` and counter caches lost counts on the
  SQL adapters.** `cas_field_delta` fell back to a read-modify-write there — its
  own comment admitted it was "not multi-writer atomic" — so two requests both
  read 5 and both wrote 6. Measured with 8 threads x 25 bumps on SQLite: **53 of
  200 increments survived**. The arithmetic now happens inside one statement
  (`jsonb_set` / `JSON_SET` / `json_set`, and `SET col = COALESCE(col,0) + ?` for
  a column-aware model), so the row's own lock serializes the bumps: 200 of 200.
  Counter caches ride the same path, so parent counts stop drifting. A missing
  field or `NULL` column still counts as 0, and a non-numeric column is refused
  by name rather than by a driver error. Verified concurrently on both SQLite and
  live Postgres.

- **SQL migration DDL is no longer a global builtin.** `__soli_sql_execute`
  (and the column-table helpers) were registered on every interpreter, so a
  controller or template could run arbitrary SQL and leave `SET` / `ATTACH` /
  `PRAGMA` on a pooled connection. They are registered only on the migration
  interpreter now. `db.execute` opens a dedicated connection (and resets the
  session afterwards on `sqlite::memory:`).
- **MySQL `DEFAULT` strings escaped for MySQL.** Quote-doubling alone let a
  default of `x\', extra INT --` close the literal and become a second column.
  Backslash, quote, NUL, newline, CR, and SUB are now escaped the way
  `mysql_real_escape_string` does.
- **Reserved table names refused.** A migration cannot create, drop, or rename
  `_migrations`, `_jobs`, or `_cron_jobs`.

- **Column-mode timestamps read as 1970, and saving a record rewrote them.**
  `Value::DateTime` is nanoseconds throughout the runtime, but the column-mode
  reader wrapped the *seconds* value `datetime_parse` returns. A `datetime`
  column therefore hydrated as 1970-01-01 plus a fraction, and writing the
  record back stored that wrong date. Both directions are correct now, with a
  test that pins the unit by round-tripping through the runtime's own formatter.

- **`create` and `save` on a column-aware model ignored the stored row.** The
  SQL write returns the row as the database holds it, but only the key was
  taken from it — so a column `DEFAULT`, a database-side trigger, and the
  stamped `created_at`/`updated_at` were invisible until the record was read
  back. Both now adopt the returned row (and convert its temporal columns the
  same way the read path does).

- **`connection "name"` never parsed in a class body.** The multi-database
  binding shipped documented but unusable: the parser recognizes class-body DSL
  calls by name and `connection` was missing from that list, so a model
  declaring it failed to load with "expected ':' and type annotation for field
  declaration". Both `connection` and the new `table` are registered now, with
  parser tests covering the bare and parenthesised forms — and confirming a
  field genuinely named `table`/`connection` still needs its annotation.

### Changed

- **BREAKING (ops, not code): the job callback endpoint is gone.** `POST
  /_jobs/run/:name`, `SOLI_JOBS_CALLBACK_URL`, `SOLI_JOBS_SECRET`, and
  `SOLI_JOBS_DATABASE` are no longer used, and the database no longer needs
  network access back to the app. `SOLI_WEBHOOK_SECRET` now signs only
  *outgoing* `Webhook.*` deliveries. Every public API — job classes, `Job.*`,
  `Webhook.*`, `Cron.*`, `perform_later`, `static cron` — is unchanged. Let
  SolidB's internal queue drain before upgrading; jobs sitting in it are
  invisible to the new engine.
- `static background: Bool = true` is accepted but has **no effect**: it opted a
  job out of running on a web worker, which is now the default for every job.
  Jobs no longer ack before running, so they are retried on failure — the old
  fire-and-forget caveat is gone.
- Fixed docs that had drifted: the cron helper tables showed five-field strings
  while the builders emit six-field ones (a five-field expression was rejected
  by the scheduler), and `SOLI_JOB_WORKERS` was documented as defaulting to 2.


- **SQL adapters (Postgres + MySQL, Phase 2–3)** — `SOLI_DB_ADAPTER=postgres|mysql`
  + `DATABASE_URL` runs Model CRUD against JSON document tables, hash `.where`,
  order/limit/count/exists, partial merges, `sum`/`avg`/`min`/`max`,
  `delete_all`/`update_all`, soft-delete scope, client-side `pluck`/`select`,
  and `Model.all`/`count`/`delete_all` on SQL (no raw SDBQL). **Phase 3:**
  eager `.includes` batching (`belongs_to` / `has_many` / `has_one`),
  multi-row `group_by` + multi-aggregate, and `soli db:import [collections…]`
  (SoliDB → SQL document tables). HABTM/through includes, `.having`, `.join`,
  graph, pgvector, and transactions stay SoliDB-only. Multi-DB Soli bench:
  `bench/frameworks/soli/bench-multi-db.sh`. Design: `docs/sql-adapter-design.md`.

- **Multi-database connections (M0–M1)** — optional `config/database.toml` names
  connections (`solidb` / `postgres` / `mysql`) with `${ENV}` expansion; without
  the file, env still defines a single `primary`. Class-body
  `connection "name"` binds a model (and its collection) to a named pool;
  QueryBuilder / CRUD route through the active connection. Cross-connection
  `.includes` errors clearly. Follow-ups: multi-SoliDB hosts, migrate
  `--connection`, request-scoped roles.

- **Production observability** — structured JSON logs and OpenTelemetry traces:
  - `SOLI_LOG_FORMAT=json` emits one NDJSON object per request (and per
    production error on stderr). Detail channels (`query`/`http`/`kv`/`timing`)
    become nested arrays; secret-bearing binds stay redacted.
  - `SOLI_OTEL=1` or `OTEL_EXPORTER_OTLP_ENDPOINT` enables distributed tracing:
    W3C `traceparent` in/out, OTLP/HTTP JSON export of the same span tree the
    dev-bar flamegraph builds (middleware, action, views, DB, HTTP). Export is
    async on a background thread (full queue drops rather than stalling
    workers). Also `OTEL_SERVICE_NAME`, `OTEL_RESOURCE_ATTRIBUTES`,
    `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT`, `OTEL_SDK_DISABLED`.
  - JSON access lines carry `trace_id` / `span_id` when tracing is on for
    log↔trace joins.
  - Docs: `/docs/development-tools/observability` (and `www/docs/observability.md`).

### Added

- **SQL `Model.transaction`** — Postgres and MySQL document adapters hold one
  pool connection for the duration of `Model.transaction { … }` (BEGIN/COMMIT/
  ROLLBACK). Nested blocks join the outer transaction. Keep blocks short so the
  pool is not starved. SoliDB path unchanged.
- **`soli db:migrate --connection NAME`** (`-c`) — run `up` / `down` / `status`
  against a named connection from `config/database.toml` (SQL secondaries).
- **`soli generate oauth <github|google>`** — OAuth *client* scaffold (requires
  `generate auth`): `OauthIdentity`, provider services, `/auth/:provider`
  routes, CSRF `state`, find-or-create user + session login. Docs:
  `/docs/security/oauth-client`.

### Changed

- **Cargo feature gates for slim builds** — `paseto`, `postgres`, and `mysql`
  are optional (on by default with `embedding` / `llm` / `codegraph`). Drop
  unused clients at compile time to shrink the binary and mapped code pages:

  ```bash
  cargo install --path . --locked --no-default-features \
    --features embedding,llm,codegraph
  ```

  Using a disabled SQL adapter at runtime fails with a rebuild hint
  (`DbError::FeatureNotCompiled`); the `Paseto` class is not registered when
  `paseto` is off. Meta-features: `sql` (= both SQL backends), `full` (=
  defaults + `solidb-driver`). Docs: Configuration → Slim binary / Keeping
  memory low.

- **Production HTTP worker default is 2** when `APP_ENV=production` (or `prod`)
  and neither `SOLI_WORKERS` nor `--workers` is set — caps baseline RSS on
  many-core boxes. Explicit env/CLI still wins; non-production still defaults
  to CPU cores.

## [1.29.0] - 2026-08-09

### Added

- **`soli update docs [folder]`** — rewrite the agent guides (`CLAUDE.md`,
  nested `CLAUDE.md`, `AGENTS.md`, `.claude/`) and the bundled language
  reference under `docs/` from the templates embedded in the installed `soli`
  binary. Use after upgrading soli so existing apps pick up current guidance.
  Overwrites those paths; keep project-specific notes elsewhere.
  `.claude/settings.local.json` is left alone.

### Fixed

- Scaffold agent markdown and related docs no longer recommend the non-existent
  generators `soli generate controller`, `soli generate model`, or
  `soli generate migration`. Recipes and `/soli-resource` use
  `soli generate scaffold` and `soli db:migrate generate`.
- Documented migration runs as `soli db:migrate up` (a bare `soli db:migrate`
  requires an action and fails).
- Scaffold docs and the success message match real output: controller E2E at
  `tests/controllers/*_controller_spec.sl`, no model test file, singular
  resource names.

### Changed

- JSON / collection hot paths continued after 1.28.0 (parse/stringify, hash
  get, array join, model/template value conversion). See the performance
  commits since `v1.28.0`.

### Added

- **`soli db:migrate` auto-loads `app/models` and `app/services`.** Data
  migrations can call `User.create(...)`, iterate `User.all()`, and use other
  Model APIs without an explicit `import` — same recursive preamble as
  `db:seed` and `soli serve`. Engine migrations load models from the engine
  root the same way.

### Fixed

- **`DateTime` component accessors are consistent about timezone.** `hour` /
  `minute` used to return **UTC** while `year` / `month` / `day` / `second` /
  `format` / `to_string` used the **local** zone, so composing parts of one
  value (e.g. `"#{t.day()} at #{t.hour()}:#{t.minute()}"`) could print a
  wall time that never existed. Every component accessor now uses the same
  view: **local by default**. Call `t.utc()` for a same-instant value whose
  components are UTC (`t.utc().hour()`), or `t.local()` to switch back.
  Equality and ordering still compare by instant only (`t == t.utc()`).
  Static `DateTime.utc()` returns “now” with the UTC view; other constructors
  keep the local view. **Breaking** for code that relied on bare `.hour()` /
  `.minute()` being UTC outside a UTC timezone.

### Clarified

- **`Duration.humanize` is magnitude-only.** It never appends `" ago"` (even
  for negative intervals from `Duration.between`); use `time_ago` for relative
  past phrasing. Documented and covered by tests.

## [1.28.0] - 2026-08-05

### Added

- **PASETO v4 tokens** — a `Paseto` class covering both purposes: `encrypt` /
  `decrypt` for local (symmetric) tokens and `sign` / `verify` for public
  (asymmetric) ones, plus `generate_local_key`, `generate_key_pair`,
  `public_key`, `key_id` and `decode_unsafe`. Keys and tokens are PASERK strings
  (`k4.local.`, `k4.secret.`, `k4.public.`, `k4.lid.`, `k4.pid.`) so they are
  self-describing and cannot be handed to the wrong purpose. Unlike JWT there is
  no `alg` header to confuse and no `none` algorithm.

  Every function raises on failure rather than returning an error hash, so a
  tampered, wrong-key or expired token cannot be mistaken for a valid result —
  use postfix `rescue` for the "or nil" shape. `decode_unsafe` nests the claims
  under `claims` and sets `unverified: true`, so reaching for `peek["sub"]`
  yields nil rather than a trusted-looking value (same reasoning as
  `jwt_decode_unsafe`).

- **Optional native SoliDB driver transport** (`--features solidb-driver`,
  `SOLI_DB_DRIVER=1`) — MessagePack on a pooled TCP socket instead of HTTP for
  model CRUD and queries. Auth prefers `SOLIDB_API_KEY`, then username/password.
  `Model.find` / document get go over the driver too. `SOLI_DB_NO_QUERY_CACHE=1`
  is honoured on the driver path as well as the cursor path.

### Changed

- **`render_json` serializes with sonic-rs** (same path as `JSON.stringify`),
  and future-resolution no longer rebuilds arrays of row hashes when no Future
  is present. Pure JSON throughput rises ~16% on the framework bench.

- **Benchmarks page** updated for the native driver session and the JSON
  optimization (synthetic prose). Harness unsets `NO_COLOR` so `oha` 1.12
  works; `start.sh` enables `SOLI_DB_DRIVER=1` when measuring Soli.


## [1.27.2] - 2026-08-03

### Fixed

- The Windows build failed to compile, which stopped the whole v1.27.1 release:
  the binary matrix is fail-fast, so `soli-windows-amd64` failing cancelled the
  arm64 builds and skipped both the release publish and the Docker image.

  `soli cloud` and `soli env down` read their target server from `deploy.toml`,
  but `module::deploy` is `#[cfg(unix)]` because it is built on ssh2 — so
  `load_deploy_config` did not resolve on Windows (E0432/E0433) even though
  neither command needs SSH to parse a file. The config half — `DeployMode`,
  `ServerConfig`, `DeployConfig`, `load_deploy_config`, `parse_deploy_toml` — is
  now an ungated `module::deploy_config`; the SSH machinery and `DeployResult`
  stay behind the unix gate and import the types from their neighbour. Behavior
  on Unix is unchanged, and `soli deploy` is still Unix-only.

## [1.27.1] - 2026-08-03

### Added

- `soli serve --assets DIR` — an extra read-only static root for file mode,
  repeatable and consulted in order, used only when the served folder has no
  match (including its nice-URL extension probe), so a mounted root can never
  shadow a real page. Fixes the case that motivated it: `soli serve www/docs`
  rendered every page and 404'd every picture, because the pages embed
  `/images/…` while those files live in `www/public/images/` — in file mode the
  served folder is the whole static root, with no `public/` sub-root.

  An assets root is data, not a second site. Only an exact file answers, with
  the same MIME type, `ETag`, `304` and `Range` handling as any other file: a
  folder gets no generated index and is absent from the sidebar tree, Markdown
  is not rendered, there is no extension probe, and a `.slv`/`.erb` is neither
  executed nor dumped as source. Pointing the flag at a folder you did not
  audit therefore widens what is *readable* by one directory tree and never
  what is *executable*. Each root is canonicalized and jailed separately, so
  dotfiles and symlinks escaping it stay `404`.

  Paths resolve to absolute at startup — before the server may daemonize and
  change directory — and a non-directory fails fast. An MVC app already serves
  `public/`, so the flag warns that it is ignored there rather than doing
  nothing silently. Under `--dev` each root is watched like the served folder,
  so editing a picture there reloads the page embedding it.

- `soli cloud` — immutable releases with a mutable alias. `deploy` builds an
  artifact, lands it in `releases/<app>/<id>/`, repoints `sites/<app>`, asks the
  proxy to deploy, gates on the health check and only then moves the alias;
  `rollback` repoints the symlink; `releases` lists what is on the host;
  `--dry-run` prints the same plan a real deploy executes. Servers come from the
  existing `deploy.toml`, the proxy key from `SOLI_PROXY_API_KEY`.


### Added

* **`soli env` — a running environment per branch.** `soli env up --branch feat/cart` creates a git worktree, writes it an `.env`, creates and migrates its own SoliDB database, seeds it, and links it into Soli Proxy's sites directory so it comes up on its own subdomain. `down` reverses all four; `list` and `url` round it out. Configured by a `[preview]` section in `deploy.toml` (`domain_base`, `local_domain_base`, `sites_dir`, `worktrees_dir`, `env_template`, `build_command`, `seed`); `--server <name>` targets a `[[servers]]` entry's proxy instead of the local one.

  Nothing new runs the app: the proxy already treats any site directory as a supervised app, so an environment is a symlink plus a database. Migrations run with `SOLI_PROTECT_ENV=SOLIDB_DATABASE`, the same guard the parallel test runner uses, so the child's own `.env` reload cannot redirect them.

  Teardown order is load-bearing. The proxy is asked to stop the app *before* the symlink is removed, because `discover_apps_inner` only drops vanished apps from its map — unlinking first leaves an orphan process holding its allocated ports. The database name and host are read back from the worktree's generated `.env` rather than re-derived, so a teardown long after creation still targets the right database. Failures are collected and all reported, since a partial teardown is exactly when you need to know what survived.

* **Preview domains are flat: `<branch>--<app>.<base>`.** DNS wildcards and the proxy's SNI resolver both match exactly one label deep, so a flat name lets a single `*.<base>` record and a single wildcard certificate cover every app and every branch; a nested `<branch>.<app>.<base>` would need a record and a certificate per app. Branch names are sanitised into a DNS label — lowercased, `[^a-z0-9-]` replaced, runs collapsed — and anything over 30 characters keeps a 24-character prefix plus 6 hex of the full name's SHA-256, so two long `task/…` branches sharing a prefix cannot collapse onto one domain. If `<slug>--<app>` would exceed a 63-character label the slug is shortened, never the app name, so the domain still says what it belongs to.

* **Build pipeline (`module::builder`).** Source → artifact in one call: clone a git ref (branch, tag or raw SHA), detect the project kind, `npm ci` + `npm run build` when there is a `package.json`, then `soli build` into a `.soli` bundle. Reports a per-stage log and a cache key derived from the manifest, lockfiles and compiler version, so an identical build can be skipped. This is the stage a PaaS runs on every push; `soli build` alone only does the bundling.

  Builds execute untrusted code — `npm ci` runs arbitrary `postinstall` scripts — so the environment handed to every stage is an **allowlist** (`PATH`, `HOME`, `LANG`, `LC_ALL`, `TZ`, plus `CI=true`), never the caller's. A database password or proxy admin key exported in the operator's shell is not visible to a build script, and a newly-invented secret is excluded without anyone updating a denylist. Each stage has a deadline and a hung one is killed rather than occupying a worker.

  The bundle stage shells out to a configurable `soli` (`with_soli_binary`) rather than assuming `current_exe()`, which is only correct when the builder runs inside the soli binary — a build service using this as a library would otherwise invoke itself. The artifact's existence is asserted after the stage rather than inferred from the exit code, since the path is handed straight to an artifact store.

* **`soli serve --strict-port`.** A taken port normally makes the server scan upward for a free one. That is right interactively and wrong under a supervisor: Soli Proxy health-checks the port it assigned, so an app that quietly moved to `port + 1` reads as unhealthy and is quarantined after three such failures — a port race presenting as a broken deployment. `--strict-port` exits instead.

### Security

* **`SOLI_HTTP_ALLOW_HOSTS` — reach a trusted sidecar without disabling SSRF protection.** `HTTP.*` blocks loopback and private addresses, so an app that legitimately needs to call something local had only `SOLI_DEV_ALLOW_SSRF=1`, which switches the guard off for *every* request the app makes — including ones built from user-supplied URLs. An app that also handles webhooks becomes an SSRF pivot. The new variable takes a comma-separated list of `host` or `host:port` entries and exempts only those; naming the port means allowlisting a proxy admin API on `127.0.0.1:9090` does not also expose a database on `127.0.0.1:6745`. Matching is on the literal host in the URL, never a resolved address, so a DNS answer cannot decide what is reachable.

* **A preview cannot inherit production database credentials.** `env_template` is copied into the worktree and then overlaid with the generated `APP_ENV`, `SOLIDB_DATABASE`, `SOLI_SESSION_DRIVER` and URL values, so a `SOLIDB_DATABASE` in the template can never survive into a preview — each overridden key is bound exactly once. A missing template is a hard error naming the file; it never falls back to the app's own `.env`, since a preview that migrates and seeds into production is the one failure here that cannot be undone.

  The default template is `.env.preview.example`, not `.env.preview`, because the generated file sets `APP_ENV=preview` and `load_env_files` layers `.env.{APP_ENV}` *over* `.env` with override — a template with that name would be checked out by git into every worktree and silently win. `soli env up` refuses to start when it finds a `.env.preview` in the worktree, and explains why.

  Preview sessions are pointed at the SoliDB driver so they land in the branch's own database. SoliKV has no namespaces and its session keys carry a fixed global prefix, so previews sharing one SoliKV would share sessions. Cache keys needed no change: they are already scoped by `SOLIDB_DATABASE`.

### Fixed

* **`HTTP.*` failed intermittently against HTTP/2 APIs — every other page load, for the same URL.** A parallel `HTTP.get_all_json` to an h2 host returned `{"error": "Request failed: error sending request for url (…)"}` for some of its URLs while the others succeeded, so a controller that read `responses[1]` 500'd about half the time. The user-facing `reqwest::Client` is process-wide with an 8-connection-per-host pool, but every request was driven by a `new_current_thread` runtime built for that call and dropped on return. Over HTTP/1.1 that is invisible — the socket dies with its runtime, so the pool just opens a new one — but over **HTTP/2** the pool keeps one multiplexed connection per host and hands it to concurrent requests, so a connection whose runtime is gone is still handed out and sending on it fails at once with `dispatch task is gone -> runtime dropped the dispatch task`.

  User HTTP now runs on a single long-lived two-worker runtime that owns the pool. The future itself still runs on the calling thread via `block_on`, so anything it reads from thread-locals (the dev query log, the current request) is unchanged; the runtime only supplies a reactor that outlives the connections. A call made from inside an async context uses `block_in_place` on a multi-thread runtime, or hands the future to the shared runtime's workers on a current-thread one, rather than building a runtime of its own. Guarded by a test that counts accepted TCP connections: three sequential requests must share one, which is exactly what the old code could not do (it opened three).

  Connection reuse across calls is the side benefit — a second request to an API you just called now skips the TCP and TLS handshake.

* **A failed HTTP request said what went wrong instead of only that it went wrong.** `Display` for `reqwest::Error` stops at the top level, so a transport failure read `error sending request for url (…)` and nothing more — the sentence that names the cause (`dns error`, `connection closed before message completed`, `runtime dropped the dispatch task`) is one or more `source()` hops down and was being discarded. Every `HTTP.*` error, and the dev HTTP log, now carry the full chain.

* **A browser that cannot run no longer fails the whole browser suite, and says why it could not.** Discovery stopped at the first browser on `PATH` and the driver then retried that one binary five times — so a machine with a snap-packaged Chromium *and* a working Chrome or Edge failed every browser spec, because `snap run` refuses a session whose user cgroup it does not recognise (no `systemd --user`, no D-Bus) and exits before Chromium starts. `Browser::launch` now walks every candidate it found, in preference order, and moves on when one will not start; `google-chrome` and `google-chrome-stable` resolving to the same file through a symlink count once. `SOLI_CHROME_PATH` still pins one browser rather than merely preferring it — falling through from the browser you asked for to a different one would be worse than failing.

  The browser's stderr was `Stdio::null()`, so the failure read `the browser exited during startup (exit status: 1)` and nothing else, discarding the one line that named the cause. It is piped and drained now, and the last lines are quoted under each candidate in the error. Drained on a thread, because the pipe has to be read or it fills and stops the browser dead — Edge logs a D-Bus error every few frames on a host without a session bus — and detached rather than joined, since Chromium's forked children inherit the pipe and EOF can lag well behind the process we killed.

  Retries are also bounded per binary now (25s, just over the 20s readiness timeout): a browser that exits immediately is cheap and still gets all five attempts, while one that never publishes a DevTools endpoint gets a single 20-second shot instead of five, which used to mean 100 seconds before the next candidate would have been tried.

## [1.27.0] - 2026-07-31

### Added

* **`soli serve` works on any folder, not just Soli apps.** Pointed at a directory with no `app/controllers/` and no `config/routes.sl`, it used to exit 70 with `Invalid MVC structure: …/app/controllers does not exist`. It now serves that directory as a website. Resolution order: a dotfile segment → `404`; a path escaping the root → `403`; a folder with an index document → that document, by its own rule; a folder → generated index with its `README.md` rendered above the listing; `*.md`/`*.markdown` → rendered page; `*.html.slv`/`*.slv`/`*.html.erb`/`*.erb` → executed by the template engine; an extension-less path → `.md`, `.html`, `.html.slv`, `.slv`, `.html.erb`, `.erb` in that order; anything else → raw bytes with MIME, `ETag` and `Range`. `GET` and `HEAD` are answered, everything else `405`. A folder requested without a trailing slash `301`s to the slash form so relative links in a README resolve correctly.

  Detection is automatic (`app/controllers/` or `config/routes.sl` ⇒ app; a `.soli` bundle is always an app), with `--static` and `--app` to pin it. `--app` keeps the previous behaviour and its error verbatim, so nothing that worked before changes.

  Markdown renders through the same converter and the same SEC-022 URL policy as `.md` views (`markdown_to_html_safe_urls`), with tables/strikethrough/task lists on. Fenced `soli` and `sl` blocks are highlighted server-side by re-lexing with the language's own lexer (`coverage::reporter::html_highlight_soli`); other languages render as plain monospace with the fence's info string as a label, rather than being guessed at by a heuristic or a CDN highlighter that would break the offline promise. Templates render with the served folder as the views root, locals `path` and `params`, and no layout; a render failure returns a `500` naming the file and the error. `--dev` live-reloads on edit through the existing SSE channel.

  Generated pages use a `tree(1)` sidebar drawn with real box-drawing glyphs whose vertical rail lights along the ancestor chain of the current page, a book-style folder index (name → leader dots → size · age), night/day solar palettes driven by `prefers-color-scheme` with a `data-theme` toggle that overrides it in both directions, and a `/`-focused filter with arrow-key navigation. The sidebar is server-rendered and every row is a real link, so it works with JavaScript off; the script only narrows what is there. CSS and JS are `include_str!`'d into the binary and served from `/__soli/files.css` and `/__soli/files.js`, so a page makes no network request at all. The sidebar tree is capped at 1000 entries and says so when it truncates; folder index pages read their directory directly and are never truncated.

* **Media, source and text files open inside the site.** Clicking a `.jpg` in a listing handed the browser raw bytes: you left the site and lost the tree. Images, video, audio and PDFs now render in the shell with breadcrumb, sidebar, size and type; text and source files render in the page (`.sl`/`.slv` highlighted by the Soli lexer) up to 512 KB, past which they stay a download; anything unshowable offers a download link instead of dumping bytes. Raw bytes remain reachable — an `<img>` embedded in a Markdown page needs the picture, not a page about it — via `Sec-Fetch-Dest` (`document` = viewer, subresource = bytes), with `Accept: text/html` as the fallback for older browsers. Tools sending neither header (`curl`, `wget`) get the file, and `?raw` forces it for anything; the viewer's own tags point at `?raw`, so it cannot recurse.

* **Markdown pages gained an "On this page" rail.** Every `##`/`###` gets a unique slug anchor and a right-hand entry, scroll-spied via `IntersectionObserver`. Hidden below 1180px; omitted for documents with fewer than two headings.

* **Index documents replace a folder's listing.** `index.html`, `index.htm`, `index.md`, `index.html.slv`, `index.slv`, `index.html.erb`, `index.erb`, in that order, each served by its own rule. Previously only `index.html` was honoured, and `index.md` was mis-classified as a README. `README.md`/`readme.md` still render *above* the listing — a README describes a folder, an index replaces it.

* **The sidebar scrolls the current file into view** on load, so a deep tree no longer lights its rail below the fold.

* MIME table gained `pdf`, `csv`, `yaml`/`yml` and `toml` — the file-mode viewer classifies by MIME type, so without them a PDF read as an unnamed binary blob.

### Security

* **File mode binds `127.0.0.1` by default** — serving a directory the operator happened to `cd` into must not publish it to the LAN without an explicit choice. `SOLI_HOST=0.0.0.0` still opts in; MVC apps keep their `0.0.0.0` default.
* **Dotfiles are invisible in file mode.** Any path segment starting with `.` returns `404` rather than `403` (a `403` would confirm the file exists), checked on the *canonicalized* path so a symlink named `notes` pointing at `.env` is hidden too, and they never appear in a listing or the sidebar. The mode never loads `.env`, never calls `init_db_config`, and never executes a `.sl` file; `File.*` and `Image.*` are jailed to the served folder.

### Fixed

* **Static file extensions are matched case-insensitively.** `get_mime_type` compared `Path::extension` against a lowercase-only table, so `logo.PNG` was served as `application/octet-stream` and the browser downloaded it instead of rendering it — which presents as a broken image rather than as a naming rule. `.md` and `.markdown` were also added to the table as `text/markdown; charset=utf-8`.


## [1.26.3] - 2026-07-30

### Fixed

* **`soli fmt` rewrote controller hook assignments into calls, silently deregistering every filtered hook.** `this.before_action(:index) = fn(req) { … }` came back as `this.before_action(:index, fn(req) { … })`. That still parses and still runs — it just registers nothing, because the hook registry (`extract_all_action_specific_function_sources`) scans raw source for the literal `") = "` followed by `fn`. The failure is invisible at the point of damage and surfaces later as a missing instance variable (`Cannot access property 'project'`). A whole-project `soli fmt` run on a real app killed **54 filtered hooks across 16 controllers** — auth, plan and guest gates on todos, docs, messages, schedule, hill, campfire, drawings, pings, archives, invitations, search, project, widgets, gather and checkins.

  Root cause: the parser desugars `foo(args) = value` into `foo(args…, value)` (`parser/expressions.rs`), so both spellings arrive at the printer as one indistinguishable `Call` node. The source still distinguishes them — the text between the last written argument (or the callee, when the parens are empty) and the trailing value reads `") = "` where a plain call has `", "` — so the printer now recovers the original spelling from that gap and prints the assignment form back. The check requires a closing paren and a trailing `=` that is not part of `==`, `!=`, `<=`, `>=` or `=>`, so comparisons inside an argument list (`eq_check(a == b, c)`, `cmp(a, b >= c)`) are not mistaken for it. Verified across every controller in the affected app: 54 hooks in, 54 hooks out, where the previous build produced 0.

* **`soli fmt` emitted a postfix `if` after a multi-line `@sdbql{ … }` block or `[[ … ]]` raw string, producing a file that would not parse.** The guard-clause rewrite collapses `if cond { stmt }` to `stmt if cond`, which puts the keyword *after* the value — fine for a one-line value, fatal for one printed across lines, since the `if` lands on the line following the closing delimiter. This broke `soli serve` boot and made `soli test` report "Test server failed to start", with the parse error pointing at a token well past the real cause.

  `@sdbql{ … }` blocks and raw string literals are copied verbatim out of the source (that is the point — escaping a multi-line query would collapse it to one 200-plus character line), so their newlines survive any layout choice. The rewrite now refuses when the inner statement or the condition carries one. The predicate is deliberately separate from `expr_likely_breaks` so call-argument width estimation is unaffected, and it reads the AST rather than counting source lines — a raw line count would answer differently on the second pass and break idempotency. A single-line `@sdbql{ … }` still collapses to postfix, which parses fine. Verified by reformatting all 423 `.sl` files of the affected app: 0 parse failures, against 1 before.

### Changed

* **`soli lint` no longer walks into locale files.** Apps that keep their translations in Soli rather than YAML (`app/helpers/locale_fr.sl` returning a hash of full sentences, one key per line) got hundreds of `style/line-length` hits per language — in one real project, 9 locale files buried every genuine finding under noise nobody would ever act on. A directory walk now skips a file when it sits under a directory named `locales/` (the `config/locales/` convention) or when its stem is `locale_<tag>` / `<tag>_locale` and the tag has the shape of a locale code: a 2–3 letter language subtag plus at most one 2–4 alphanumeric script/region subtag (`fr`, `fil`, `pt_BR`, `zh-Hans`, `es-419`). The shape check is what keeps the skip honest — `locale_helper.sl`, `locale_switcher.sl` and `locale_en_us_backup.sl` are code, not data, and stay linted. Double extensions are stripped first so `locale_fr.html.slv` matches on `locale_fr`. Skips are never silent: the summary line reports `(N locale files skipped)`, and naming a file explicitly (`soli lint app/helpers/locale_fr.sl`) lints it regardless, which doubles as the escape hatch. `soli check` is unaffected — type-checking a translation table is cheap and produces no noise.


## [1.26.2] - 2026-07-30

### Fixed

* **`soli fmt` deleted blank lines inside an `if` body.** Reported as "fmt removes the blank line before a `return`", but the `return` was incidental — any blank line between two statements in an `if` body was dropped (`a()` / blank / `b()` came back with the blank gone), while the identical body under `for` / `while` / a method kept it. `print_block_or_stmt` records the block's opening line so a body-leading comment measures its gap from the opener rather than from the statement above the keyword (the v1.25.4 `catch`-comment fix). It took that line from the block's span — but the parser gives an `if` body a span whose line is its *last* statement, where `for` / `while` use the first. Recording that pushed `last_emitted_line` past the whole body, so the paragraph check saw a phantom comment above every statement and suppressed the blank. The opening line is now clamped to the first statement's line, which a block's opener can never follow. Idempotency and the `catch`-comment behaviour both verified; three regression tests cover `if`, `else`, and the reported blank-before-`return` shape.


## [1.26.1] - 2026-07-30

### Fixed

* **`for x in …` over a value read inside `grouped(fn() { ... })` raised `cannot iterate over array`.** A grouped read returns a `Value::Deferred` placeholder, and `batch::end()` resolves the cell it points at without replacing the binding — so every consumer has to unwrap it, and `for`-in was not one of the sites that did. The error text was self-contradictory because `Value::Deferred::type_name()` *forces* the deferred and reports the resolved type, so an unresolved placeholder holding an array printed "cannot iterate over array". Fixed in both for-in implementations, which are separate code paths: `executor/statements.rs` for Soli code (normalized alongside the existing `QueryBuilder` case) and `template/renderer.rs` for `<% for post in posts %>` in a view, guarded there so the non-deferred case does no extra work. Covered end to end through `parse_template` + `render_nodes`, including a test that a genuine non-iterable still errors.

  Also corrected the docs, which claimed values are "ordinary values" after the block. They behave that way, but the binding remains a placeholder that resolves on use — which is precisely why a new way of consuming one can need teaching to unwrap it. Note a `Deferred` used as a *method receiver* (`@posts.map(...)`) is still not covered by any resolution site.


## [1.26.0] - 2026-07-30

### Added

* **`soli test` now exercises the production coalescing path, and a new `assert_no_ungrouped_reads`.** The test server runs with `--dev` so the AQL query log is populated — but `--dev` also *disables* `grouped(fn() { ... })` coalescing, so no spec ever ran the combined `LET … RETURN […]` path, and `assert_query_count` measured dev's un-coalesced number instead of the round-trips production makes. The decision now lives in `batch::should_coalesce(dev_mode, test_runner)`: interactive `--dev` still keeps reads separate for a readable query log, while test-runner children (which already carry the runner's token via `SOLI_INTERNAL_TEST_RUNNER`) coalesce like production. A grouped action reports one query in a spec, not one per read.

  Paired with a detector for the case N+1 scanning is structurally blind to. `detect_n_plus_one` fingerprints by query template, so it only fires on a *repeated* one; three unrelated reads are three distinct templates with a count of one each — invisible to it, yet exactly what `grouped` is for. `detect_ungrouped_reads` reports distinct read templates that each ran once outside any `grouped` block, surfaced as an amber `N READS · N ROUND-TRIPS` advisory in the dev bar's query panel and as `assert_no_ungrouped_reads(response)` / `response["ungrouped_reads"]` in specs. Writes and repeated templates are excluded, and reads already inside a `grouped` block are excluded via a new `LoggedQuery.grouped` flag — necessary because interactive `--dev` does not coalesce, so without it correctly-grouped code would be reported as unfixed. Advisory rather than an error: dependent reads (`find` a record, then query by its key) cannot share a round-trip and nothing in the log distinguishes them, so both the panel and the failure message state that precondition.

### Changed

* **The dev bar's N+1 warning now names Soli's own remedies instead of raw AQL.** On detecting a query template fired in a loop, the hint read "batch with `FILTER doc.field IN @ids`" — sending the user to hand-written AQL while `includes(...)` (association preloading) and `grouped(fn() { ... })` (read coalescing) went unmentioned. A codebase audit found `grouped()` had zero uses outside its own spec, and this was the likely reason: the one place a developer notices an N+1 never pointed at the fix. The hint now reads "preload with `includes(...)`, or coalesce unrelated reads in `grouped(fn() { ... })`". `assert_no_n_plus_one` shares the same detector, so spec failures benefit too. Also documented `grouped()` in the repo-root `CLAUDE.md`, which had never mentioned it.


## [1.25.4] - 2026-07-30

### Changed

* **`soli new` and every `soli generate` now emit formatted Soli.** A freshly generated app was 6 files away from `soli fmt --check` clean, so the first thing an agent did — run `fmt`, as the generated `CLAUDE.md` instructs — produced a diff of files the user never touched. The scaffold's 30 `.sl` templates are formatted at rest, and `write_file` (the single choke point all generators share, including ones added later) runs `.sl` content through the formatter on the way out; content the parser rejects is written through unchanged rather than failing the generator. Appended `config/routes.sl` blocks and the seed generator, which used `fs::write` directly, go through it too. Verified across `new` + `auth` + `oidc_provider` + `offline` + `devices` + `app_links` + `scaffold` + `mailer` + `component` + `db:seed generate`: `soli fmt --check` reports 0 files and `soli lint` reports no issues, and the generated app serves `/`, `/posts/new`, `/login` and `/signup` with 200.

### Fixed

* **fix(fmt): a blank line under a file header comment was deleted.** The paragraph-preservation check measures the gap from the previous statement, so with no previous statement it never ran — `# Migration: create_users` + blank + `def up(db)` lost its blank, while an identical gap two statements down kept it. Same for a comment leading a block body. The blank is now preserved when a comment block was just emitted and the source had a gap, which also leaves `# soli-lint-disable-next-line` attached to its target.

* **fix(fmt): a comment that was the whole body of a `catch` escaped the block.** With no statement to flush before, `rescue` / `# already exists` / `end` printed the comment *after* the `end`, where it read as documenting whatever followed. Block bodies now flush through their closing line. The `catch` keyword's own line is recorded too — without that, a body-leading comment measured its gap from the statement above the keyword and gained a blank line on the second pass, breaking idempotency.

* **fix(scaffold): `soli generate offline` produced a controller that could not be parsed.** `sync_controller.sl` assigned an `if`/`else` as an expression (`let events = if since.present?`), which the parser rejects with `Unexpected token 'if', expected expression` — so the sync routes were dead on arrival. Hoisted to a declaration assigned in each branch, per the `let` guidance in `CLAUDE.md`.

* **fix(scaffold): the `offline` and `devices` migrations tripped 18 lint warnings in the app that generated them.** Their deliberately-idempotent `begin`/`rescue` steps had empty (or comment-only) rescue bodies, which `smell/empty-catch` and `style/empty-block` both flag — leaving a user who ran the documented `soli lint` step with 18 warnings they hadn't written. Each rescue now prints which step it skipped, which is what an operator running a migration wants to see anyway.

* **fix(scaffold): the generated form partial had a 186-char line.** The Tailwind class string in `f.text_field(…)` broke `style/line-length` inside a `<%- %>` block. Split across two concatenated lines; the rendered `class` attribute is byte-identical (verified through the template engine, not just the linter — `soli lint` does not parse the Soli inside `.slv`).


## [1.25.3] - 2026-07-30

### Changed

* **`soli fmt` now puts a blank line after an early `return`.** A guard clause and the body it guards were run together as one block of lines. A `return` is followed by a blank line, with two exceptions where the blank would only add noise: the next statement is another `return` (a run of guards stays one paragraph) or the block's `end` follows (nothing below to separate it from). The rule covers postfix `return x if cond` — in practice the only `return` with code after it, the rest being unreachable — and mirrors the blank already emitted after a block-form guard. `fmt` remains idempotent, verified over `www/app` and `tests/`.

* **The `CLAUDE.md` files `soli new` generates tell the agent to run `soli fmt`, not just `soli lint`.** `fmt` is step 1 of the verification loop in the root file and in `tests/`, `app/controllers/`, and the `/soli-verify` command; `app/models/`, `app/middleware/`, `app/views/`, and `db/migrations/` gained a "Before you're done" block with the two commands scoped to that directory. `fmt` goes first because it rewrites layout in place, so several `style/*` rules stop firing once it has run and lint's remaining output is the part needing judgement. Each file also notes what `fmt` does *not* touch: `.html.slv` templates are left hand-indented, and a `""" … """` query is rewritten while `[[ … ]]` is preserved. The blank-line-after-`return` convention the root file already documented is now enforced by `fmt` rather than left to the agent.

### Fixed

* **fix(fmt): `soli fmt` re-escaped raw strings, collapsing multi-line SDBQL onto one over-long line.** A `[[ … ]]` literal was printed through the escaping path, so a 6-line migration query came back as a single 226-char `"\n    FOR post IN posts\n …"` — semantics intact, but the query unreadable and `style/line-length` now failing on code the formatter itself produced. `r"…"` had the same fate (`r"C:\x"` → `"C:\\x"`). Both forms are now re-emitted from their source bytes. The two differ in what their span covers (`[[` is included, `r"` is not), so the closing delimiter is located from the source and the enclosed bytes are compared against the lexed value before being trusted — a mismatch falls back to escaping rather than emitting a literal that means something else. `""" … """` is unchanged (still escaped); `[[ … ]]` is the form to use for multi-line queries.

* **fix(scaffold): a freshly generated app failed the `soli lint` step its own `CLAUDE.md` documents.** Two issues, both shipped in the scaffold: `application_helper.sl`'s `link_to_class` was a 136-char line (`style/line-length`), and `auth.sl` compared an API key with `== ""` (`idiom/prefer-blank`). `soli new` now produces a lint-clean app. `soli fmt --check` still reports 6 template files — the scaffold is written in brace style while the docs prescribe Ruby style — which is a separate cleanup, not a patch-release sweep.

* **fix(scaffold): the generated `CLAUDE.md` listed three generators that don't exist, 600 lines after warning against them.** The "Common commands" block still advertised `soli generate controller|model|migration` while the section above it explained that reaching for those is how an agent wastes its first five minutes. Replaced with the real commands, and `db/migrations/CLAUDE.md` no longer tells agents to name files with `soli generate migration` either. — and in one case code that meant something different.** Four defects, all reachable by running `soli fmt` then `soli lint` on a real app. **(1)** A block `unless a || b` desugars to `Not(Or(a, b))`, and the unary printer only parenthesised operands that were `Binary`/`Assign`/`CompoundAssign` — so it emitted `!a || b`, which is `(!a) || b`. That is a **silent behaviour change**: `unless false || true` skipped its body, the reformatted `if !false || true` entered it. The paren list now covers every operator that binds looser than unary (`&&`, `||`, `??`, pipelines, `rescue`, ternaries, `match`, `throw`, spreads). **(2)** Wrapping a long `&&`/`||` chain put the operator at the *start* of the continuation line, but a Soli statement ends at its line break — the output failed to parse with `Unexpected token '&&', expected expression`. The operator now trails, and the continuation is indented two levels so it stays distinct from the block body. **(3)** The per-argument width estimate was clamped at 60 chars and array elements at 40, so a call or array holding one long string was judged to fit and printed past the limit, tripping `style/line-length`. Both clamps are gone. **(4)** A string literal's span excludes its closing quote, so every span-derived width was one byte short per string; widths for literals, arrays, hashes and groupings are now computed exactly from the AST instead. Arrays also break on width alone — a 2- or 3-element array of long strings previously matched no count-based rule and overflowed. `fmt` remains idempotent, and formatting a 105-file app now leaves `soli lint` clean where it previously reported a parse error and 8 length violations.


## [1.25.2] - 2026-07-29

### Added

* **feat(dev): a mail inbox at `/__soli/inbox` — every email the app sends, viewable with no local SMTP server.** `/__soli/mailers` previews mailer templates with fake data; this shows what the app *actually sent*, with the real data that was rendered into it. Each message opens on a detail page with its headers, its attachment metadata, and three tabs: the HTML body (served into a sandboxed iframe, so a mail's own scripts never run against the dev origin), the text part, and the raw RFC 5322 source — downloadable as `.eml` to open in a real mail client. The listing is **searchable and paginated**: `?q=` matches the subject, any address (`to`/`cc`/`bcc`/`from`/`reply-to`), both bodies and attachment filenames; `?per=` and `?page=` page through it; a filtered view is a plain link. New arrivals show a badge rather than reloading the page under the reader, and **Clear inbox** empties it. Captured in a process-wide ring buffer (last 100, cleared on restart) written by every worker thread — Soli's workers are threads in one process, so the inbox is complete regardless of which one delivered. Dev-only: nothing is captured and the routes do not exist without `--dev`. Rails needs the `letter_opener` gem or a separate MailCatcher process for the same thing.

* **feat(dev): a `tools` menu in the dev bar, so the dev-only galleries aren't URLs you have to know.** `/__soli/inbox`, `/__soli/mailers` and `/__soli/components` were reachable only by typing them; they are now one click from every page, each opening in a new tab so the page under inspection isn't navigated away from. The button carries the inbox's message count, so mail sent while you were clicking around announces itself. The panel renders *outside* `#__solidev_panels` — clicking a request row replaces that container's innerHTML to retarget the per-request panels, so a tools panel inside it would vanish on the first drill-down; a test pins the ordering.

* **feat(dev): every `/__soli/*` page carries a link back to the app.** The galleries and the inbox were one-way trips — you arrived by URL or from the dev bar and the only way out was the browser's back button. The two standalone preview pages (`/__soli/components/<name>`, `/__soli/mailers/<mailer>/<action>`) get it too, but only when opened directly: the catalogs append `?framed=1` to their iframe srcs, so a back link doesn't repeat inside every gallery card.

* **feat(mailer): under `--dev`, a mailer with no configured SMTP host captures instead of failing.** A dev box rarely runs a mail server, and a signup flow that dies on `deliver_now` is a poor way to discover that. An unconfigured host now parks the message in the dev inbox and lets the request carry on, the way `letter_opener` does. Every delivery is tagged with what happened to it — `sent` (an SMTP server accepted it), `captured` (never left the process: no host, or a `test`/`logger` delivery method), or `failed` — and **a failure is captured too, carrying its error**, including one that fails MIME validation before any connection is attempted: a rejected recipient or a refused connection is exactly what you opened the inbox to look at. Outside `--dev` an unconfigured host is still a hard error and nothing is captured.

### Removed

* **remove(dev): the database browser at `/__soli/db`.** The collection index, the paginated row tables (`/__soli/db/<collection>`), the JSON document view (`/__soli/db/<collection>/<_key>`) and the read-only SDBQL query box are gone, along with `src/serve/db_browser.rs` and the routes that mounted them. Use the app-aware TUI REPL (`soli` in an app directory loads your models and DB connection) or SolidB's own tooling instead. Dev-only to begin with — the routes never existed in production, so nothing there changes.

### Fixed

* **fix(bench): the published Laravel Octane memory row measured the supervisor, not the server — wrong by ~5x, in the flattering direction.** `pgrep -f frankenphp` matched `php artisan octane:start --server=frankenphp`, the supervising PHP CLI, while the actual `/usr/local/bin/frankenphp run` process was skipped because its `smaps_rollup` is root-owned and unreadable, contributing a silent zero. Both containerised rows are corrected from cgroup usage: Laravel php-fpm 84 → 104 MB idle (111 → 131 loaded), Laravel Octane 43 → 200 MB idle (43 → 214 loaded). That inverts the claim the page made — it said Octane used *less* memory than php-fpm; the truth is the intuitive one, Octane roughly doubles both throughput and memory. The five native stacks were audited and have zero unreadable processes, so their PSS figures stand. `memory.sh` now measures containers from their cgroup, labels which method each row used, and warns loudly when a PSS sum would skip a process rather than undercounting in silence.

* **fix(mailer): email sent from a `--dev` server carried the dev bar's instrumentation into recipients' inboxes.** The hover overlay wraps each rendered template in `<!--solidev:view:start id=… name=…-->` / `<!--solidev:view:end-->` comments, and mailer bodies go through the same renderer — so every HTML *and text* part sent from a development server shipped with those markers embedded, invisible in an HTML client but plain stray text in any client showing the plain-text alternative. Mailer renders now suppress marker emission (`template::without_dev_markers`, restored via a drop guard so an error can't leave it stuck on); `view_log` still records the render, so the dev bar's per-template timings are unchanged.


## [1.25.1] - 2026-07-28

### Fixed

* **fix(controllers): `render_json(expr)` evaluated its argument twice, so a query builder passed inline issued the database query twice per request.** `render_json` has an interceptor that implements the `as_json` override: it evaluated the first argument to test whether it was an instance whose class defines `as_json`, and if it was not — which is every hash, array and builder — it *discarded the value and returned `None`*, sending `evaluate_call` down the normal dispatch path, which evaluated the same expression again. Harmless for a literal; for `render_json(Post.pluck(:id, :title, :views).all)` — the idiomatic one-line JSON action, and the shape the docs recommend — it meant two identical round-trips to the database on every request. Confirmed by query log (6 `FOR doc IN posts` for 3 requests, now 3) and by SoliDB's CPU halving. The interceptor now evaluates each argument exactly once and always returns a result, so there is no second evaluation to fall through to; it bails out *before* evaluating anything when the argument list is a shape it does not handle. Measured on a 50-row JSON route at 16 workers: **22,527 → 37,425 req/s (+66%)**, p99 10.45 → 6.69 ms. Binding the builder to a local first was the workaround and is no longer needed. Guarded by an end-to-end test that counts how many times a handler's argument is evaluated.

### Added

* **feat(model): Ruby-style symbols are accepted wherever a field name is expected.** `Post.pluck(:id, :title, :views).all`, `User.order(:created_at, :desc)`, `User.select(:name, :email)`, `User.sum(:balance)`, `find_by(:email, ...)`, `group_by(:country, ...)`, `increment(:views)` — every Model static and every chained QueryBuilder method now takes `:name` alongside `"name"`. The language always had symbol literals (`:name`, `%i[...]`) and `Value::Symbol`; the field-name argument matchers just refused them. Chained builder methods normalize symbols once at dispatch, the Model statics accept both in their matchers, and the error messages now say "expects a field name (string or symbol)". Strings remain fully supported — this is sugar, and it makes a Soli controller line read almost byte-for-byte like its Rails equivalent.

* **bench:** **the framework comparison runs six stacks over seven workloads, and every app now lives in the repository.** `bench/frameworks/` holds the Soli, Rails, Express, AdonisJS, Laravel and Django apps plus the harness — previously they sat in a scratchpad, so the published numbers were not reproducible by anyone. Added this cycle: **Laravel** (php-fpm 16 workers + nginx, Eloquent + Blade), **Django** (gunicorn, Django ORM + templates), **AdonisJS 6** (Node cluster, Lucid + Edge), three **write rows** (one create, update and delete per request against an isolated 800,000-row table reset before every cell), **WebSocket** echo and fan-out rows across Soli, Express and Rails ActionCable, and a per-row tab strip that shows the handler in each stack beside the result. Every figure was re-measured in one sweep when AdonisJS landed rather than splicing new rows beside older numbers, so all stacks read 5–15% below the five-stack run — internally consistent is what a comparison needs. Fairness corrections, all stated on the page, which moved results more than any framework difference did: Express was moved off the raw `pg` driver onto Sequelize (measuring one stack's hand-written SQL against four ORMs flattered it by 34%); PostgreSQL runs `synchronous_commit=off` for the write rows, since SoliDB acks before `fsync` and comparing buffered writes to durable ones is not a comparison; Laravel and Django needed persistent connections or they were measuring ~8ms of connection setup per request; and **Laravel Octane** is published as a labelled reference row rather than as "Laravel", because presenting the faster runtime under the framework's plain name would flatter it the same way the raw driver flattered Express.

## [1.25.0] - 2026-07-27

### Fixed

* **fix(model): multi-field `Model.pluck(...)` rows are plain hashes, not half-hydrated model instances.** Projected rows were run through the same instance hydration as full documents, producing `Post` instances that carried only the plucked fields — accessors read fine, but they *looked* like models while `save`/`update` on one would have operated on a partial document with no `_key`. Rows from `pluck` now bypass instance hydration on every path that can produce them (`.all`, `.first`, batch-coalesced reads, `.similar()` chains) via one `hydration_class()` accessor; the output shape is unchanged (`[{a: .., b: ..}, ...]`, single-field pluck stays a flat array of values). Note the deliberate asymmetry, now documented: builder `pluck` returns self-describing hashes, while `Array#pluck` on an in-memory array returns arrays of values. Performance guidance from the same investigation: prefer builder-side `Model.pluck(...).all` over `Model.all.pluck(...)` — the former projects **in the database**: the benchmark's 50-row route went from 13.3k to 21.2k req/s at 16 workers, with the wire transfer dropping from full ~15 KB documents to 2.3 KB of named fields per request.

* **perf(db): DB queries run on a per-worker reactor with a per-worker connection — +12% throughput at 16 workers, +23% at 32, and the TCP churn toward SoliDB is gone.** Every Model query used to `block_on` the server's shared tokio runtime, funnelling all DB I/O readiness through that runtime's single driver thread, which then had to cross-thread-wake the parked worker for every completion — a per-query tax (~190µs at 16 workers) that *grows* with worker count, which is why adding workers plateaued instead of scaling. Separately, the shared client kept only 8 idle connections per host, so under 200 concurrent requests the rest were discarded after each query: ~1,300 TCP connects/sec to SoliDB, 12,900 TIME-WAIT sockets in a 10-second window, sustained pressure on the ~28k ephemeral ports. Each worker thread now drives its DB futures on its own current-thread runtime (the same `FALLBACK_RT` the REPL and scripts always used) with its own HTTP client, so each worker holds one hot connection reused for every query, and readiness is polled by the very thread that waits on it. Measured on an idle 16-core box against a loopback SoliDB, full request through a single-doc `find` route: **25,905 → 28,955 req/s at 16 workers, 31,679 at 32** (the old path was flat-to-negative with worker count); a 50-row scan route 12,452 → 14,067; TIME-WAIT growth over a 20s run +4,086 → **0**; p50 7.68 → 6.86ms. `SOLI_DB_SHARED_REACTOR=1` restores the previous behavior unchanged, and `SOLI_DB_POOL_MAX_IDLE` (default 8) sizes the shared client's idle pool for the paths that still use it (async contexts, keep-warm). Two consequences of a per-thread pool, both handled. **A pooled connection may only ever be driven by the runtime that created it** — hyper spawns its I/O task on whichever runtime is current when the connection is established — so every DB path (the model layer, `SoliDBClient`, sessions, jobs, uploads) funnels through one `block_on_db` entry point owning a single runtime per thread. Two runtimes over one pool is not a slow path but a stall: whichever path picks up the other's connection writes its request onto a reactor nobody is driving and waits out the full client timeout, 10s per alternation, silently. And a reactor that only runs during a query cannot notice the peer closing an idle connection, while the keep-warm ping — one thread, one pool — can only refresh its own; worker-local pools therefore retire connections after 25s (`SOLI_DB_POOL_IDLE_SECS`), inside SoliDB's 30s idle close, so a worker idle longer than that pays one reconnect on its next query instead of failing on a dead socket.

* **fix(serve): `--workers 2` served requests on one thread, so it performed exactly like `--workers 1`.** The realtime split reserves a worker for WebSocket/LiveView events so a burst of them can't starve HTTP — but it defaulted to reserving one at *every* pool size, and on a two-worker pool that is half the capacity. `--workers 2` and `--workers 1` both ended up with a single HTTP worker and measured identically: a DB-backed route sat at ~11.2k req/s in both, where two HTTP workers reach ~20.7k. The cost was worst exactly where it was least affordable, and silent unless you read the `Worker pool:` startup line — while `configuration.md` was recommending `SOLI_WORKERS=2` as the primary lever for cutting memory. A realtime worker is now reserved by default only once the pool has **4 or more** workers, where it costs 25% instead of 50%; below that every worker drains both channels, which is what a one-worker pool always did, so realtime keeps working and simply shares the pool. An explicit `SOLI_WS_WORKERS` is still honored at any size (and is now documented — it never was), including `0` to disable the split on a large pool, and the allocation is always clamped so at least one HTTP worker survives. Measured on an idle 16-core box against a loopback SoliDB, `Model.find` through a full request: **workers=2 11,191 → 20,651 req/s (+85%)**, workers=3 20,343 → 26,014 (+28%), workers=4 unchanged at the threshold. The startup line now also reports the collapsed case rather than printing nothing.

* **fix(datetime): a `DateTime` serialised to JSON as `{}`.** Every timestamp in an API response, and every DateTime written through a model, came out an empty object. The cause: a DateTime was an `Instance` whose only field was the private `_ts`, and the serialiser deliberately drops `_`-prefixed framework internals — so there was nothing left to emit. It now serialises as an RFC 3339 string, matching `to_iso()`, on all three JSON paths (`.to_json()`, the executor's renderer, and `serde`). For the same reason `str(dt)` printed `<DateTime _ts: 1794744000000000000>`, leaking the internal field; it now prints the same local wall clock `to_string()` returns. **Both are behaviour changes**, but neither previous output was usable.
* **perf(datetime): a `DateTime` is a native value rather than an allocated object — the category is 1.22x faster.** It was an `Instance`, so every operation paid an `Rc<RefCell<..>>` allocation plus a hash-map insert, and every accessor paid a string-keyed lookup to read `_ts` back out. It is now `Value::DateTime(i64)`, following the `Decimal` precedent and fitting in the existing 24-byte `Value` since an instant is just an `i64`. Operations that *return* a DateTime gained most — `from_unix` **-26.1%**, `subtract_days` **-25.2%**, `parse` **-24.9%**, `now` **-24.3%**, `end_of_month` **-23.3%**, `add_hours` **-22.2%** — while those returning an `Int` gained the hash lookup only (`year` -5.9%, `to_unix` -5.1%). Measured against Ruby 4.0.6, the DateTime category improves from **2.43x to 1.88x**. Dispatch has no class to go through, so both engines route to one shared method table and cannot drift. Verified by diffing every DateTime method across 16 timezones and 12 DST-straddling timestamps against the previous binary — 16/16 byte-identical, and zero interpreter-vs-VM disagreements.

* **fix(datetime): the month and year boundary methods crashed on daylight-saving dates.** `beginning_of_month`, `end_of_month`, `beginning_of_year` and `end_of_year` built a local wall-clock time and then called `LocalResult::unwrap()`, which panics when that time either does not exist (the hour the clocks skip forward) or is ambiguous (the hour they repeat). Under `TZ=America/Havana`, `DateTime.parse("2026-11-15 12:00:00").beginning_of_month()` panicked with *"Ambiguous local time, ranging from 2026-11-01T00:00:00-05:00 to 2026-11-01T00:00:00-04:00"* — a 500 on that request, contained by the per-request panic guard. Scanning all 597 zones over 2015..=2035 found 17 such dates across Africa/Cairo, America/Asuncion, America/Havana, Asia/Amman, Asia/Almaty, Cuba and Egypt, several of them in the future. Separately, `beginning_of_hour` and its neighbours returned a plain `Failed to compute …` runtime error on the fall-back hour in Europe/Paris, Europe/London and Asia/Beirut. Both resolve to a value now: an ambiguous time takes the **earliest** of the two instants (the first time the wall clock reads it, which is what "beginning of" means, matching ActiveSupport), and a nonexistent one moves forward to the moment the gap closes. The conversion is total, so these methods have no failure case left.
* **perf(datetime): local-time conversion is cached instead of re-resolved on every call.** `chrono`'s `Local` re-resolves the system zone each time it is used; every DateTime accessor did so once and the boundary methods twice. A `chrono_tz::Tz` resolved once per process does the same work in 20.2ns instead of 47.2ns, and `from_local_datetime` in 21.6ns instead of 234.7ns — which is why the boundary methods gained the most. `end_of_month` is **48.5% faster**, `year` 12.5%, `format` 6.5%, `now` 6.4%; the DateTime category's geometric mean improves **9.3%**. `$TZ` is still honoured first, exactly as `Local` does — resolving through the system zone alone would have silently ignored the `ENV TZ=UTC` that most containers set, so a `$TZ` holding a POSIX spec rather than an IANA name keeps using `Local` and stays correct at the old cost. Verified by diffing every DateTime method against the previous binary across 16 zones and 12 DST-straddling timestamps: 13 zones byte-identical, 3 where the old binary failed outright.

* **fix(testing): `mock_http_server_start()` reset the connection on a request with a large body.** The mock read only as far as the end of the request headers and then answered, leaving the body unread in the socket — and closing a socket with unread data makes the kernel send RST rather than FIN, so the client lost the response and the call surfaced as `ConnectionReset` instead of the `{"ok":true}` the mock answers with. A small body is buffered by the kernel and was unaffected, which is why this went unnoticed; anything past the socket buffer failed every time. The handler now drains the body through `Content-Length` first — which the code's own comment always said it did.
* **fix(check): 25 general-purpose builtins were rejected by `soli check` despite working at run time.** `sha256`, `sha512`, `md5`, `hmac`, `secure_compare`, `password_hash`, `password_verify`, `html_escape`, `html_unescape`, `strip_html`, `sanitize_html`, `file_exists`, `mkdir_p`, `time_ago`, `setenv`, `unsetenv` `sleep`, the X25519/Ed25519 key primitives (`x25519`, `x25519_keypair`, `x25519_public_key`, `x25519_shared_secret`, `ed25519_keypair`), `datetime_now`, `file_write_bytes` and `rerank` — the one AI primitive missing beside `embed`, `embed_batch` and `llm_generate` — are all registered at run time and were unknown to the type checker, so a script that hashed a password or escaped some HTML failed `soli check` while running perfectly. Found by listing every registered builtin and comparing "resolves at run time" against "accepted by the checker" — the same audit that found the method-registry drift. The comparison is deliberately like-for-like: a builtin that is also unavailable at run time in a plain script (a test helper, a DSL keyword, a controller helper) is correctly rejected and is not counted.
* **perf(vm): `break` and `next` compile everywhere they can appear.** Both were refused inside a `try` — jumping out would have left that `try`'s exception handler registered with its loop abandoned — and refused inside a lambda, where there is no enclosing loop at all. Both now compile. Leaving a `try` drops its handlers and runs its `finally` first, innermost outward, in that order so an exception raised by the `finally` reaches the handler *outside* the block being left. Inside a lambda they are absorbed at the function boundary, matching the interpreter: `[1,2,3].each(fn(x) { break })` stops the callback without touching the loop running outside it. With this the repository has **no file left whose top level the compiler refuses for a control-flow reason** — what remains is `debug` (which needs the interpreter's captured environment), two block forms that are interpreter-semantics by design, and one `const` redeclaration shape left deliberately.
* **perf(vm): bare type-test patterns (`v: Type`) compile.** The last match shape the parser can actually produce. It binds nothing — the name before the colon is discarded, as in the interpreter — so it is a pure type test. **Every match pattern Soli can parse now compiles.** The `Type { field, … }` form that would carry fields is unreachable: its parser branch sits behind a guard requiring the identifier to be followed by `:`, so the `{` it then looks for can never be there — filed alongside the equally unreachable `And`/`Or` patterns.
* **perf(vm): hash rest patterns compile.** `match v { {name: n, ...rest} => … }` binds the leftover keys and runs compiled. Built from the `except` the language already exposes rather than a new opcode, so the two engines agree on what "leftover" means by construction instead of by two implementations happening to match. With this, every match pattern the parser can produce compiles except `Type { field }` destructuring — the known-divergent list holds no `match_*` entry at all now.
* **perf(vm): nested match sub-patterns compile — no match pattern in this repository is refused any more.** `match data { {user: {name: n}} => … }`, `[1, x]`, `[{k: v}]` and the rest all run compiled now. Nesting was the one structural gap left: every other pattern kind runs all its tests *before* pushing anything, so a single "how much to clean up" number covered every exit, but a nested sub-pattern is tested only after its container has extracted and bound the value it lives in — an inner failure unwinds with outer bindings already on the stack. Each failure jump now carries its own live count, and pattern compilation recurses by parking each extracted value in a slot of its own. Only `{a: x, ...rest}` (which must *build* the leftover hash) and `Type { field }` destructuring are left, plus `And`/`Or`, which no parser path can produce.
* **perf(vm): array rest patterns compile.** `match v { [first, ...rest] => … }` no longer sends the handler to the slower engine. With `...rest` the named elements are a prefix and the length test becomes `>=` rather than `==`, matching the interpreter; the tail is bound with the same `slice` the language already exposes. Hash rest (`{name: n, ...rest}`) still falls back — it has to build the leftover hash — as does a *nested* sub-pattern such as `{user: {name: n}}`, which needs the pattern compiler to recurse.
* **perf(vm): enum-variant patterns compile.** `match s { Status.Active => …, Status.Pending(r) => … }` no longer sends the handler to the slower engine — the shape most enum code is written in. The class name and the `__variant` tag are both checked before any payload is bound, so the arm has the same provable shape as the array and hash forms. Payload binding needed a new opcode rather than a compile-time property read: an enum variant's payload field *names* live in the class's `__enum_variants` metadata, so the mapping from position to field name is only known once the instance is in hand. A nested sub-pattern inside a payload still falls back. The repository's own refusals are now down from 10 to 6, and no match-pattern file is left except one using `...rest`.
* **perf(vm): hash patterns with named fields compile.** `match v { {name: n, age: a} => … }` no longer sends the handler to the slower engine. Same bounded shape as the array form and for the same reason: every key test runs *before* any binding is pushed, so a failing arm never unwinds a half-built set of bindings. A missing key falls through to the next arm and extra keys are ignored, matching the interpreter. `{name: n, ...rest}` still falls back — building the leftover hash is not implemented. With this, the repository's own refusals are down from 10 to 7; the two files still using a deferred pattern use `...rest` and enum-variant patterns.
* **perf(vm): fixed-length array patterns compile.** `match v { [a, b] => …, [_, b] => … }` no longer sends the handler to the slower engine. The bounded shape is deliberate: a fixed length whose parts only bind or ignore has a provable stack effect, because every test runs *before* any binding is pushed, so a failing arm never has to unwind a half-built set of bindings. `[a, ...rest]` needs a slice, and a nested or literal sub-pattern needs recursion the compiler does not have yet — both still fall back. Hash and destructuring patterns are unchanged, which is why the repository's own remaining refusals are unmoved: those three files use exactly the shapes still deferred.
* **fix(vm)/perf(vm): typed match patterns compile, and a non-exhaustive match raises instead of yielding null.** `match v { Int: n => …, String: s => … }` used to send the handler to the slower engine along with every other binding pattern; it compiles now, guards included. Separately and more importantly: a match that fell through *every* arm raised `no pattern matched the value` in the tree-walking interpreter and silently evaluated to `null` in the compiled one — so a match missing a case failed loudly under `soli test` and produced a null under `soli serve`, which is the exact shape this branch has spent its time removing. Both engines raise now. Code cannot have depended on the null: it would already have been failing in the interpreter.
* **perf(vm): binding match patterns compile.** `match n { 0 => "zero", x if x < 0 => "negative", x => "positive:#{x}" }` — the ordinary shape of a match — used to send the whole handler to the slower engine, because only wildcard and literal arms compiled. Binding needs somewhere for the bound value to live, so the subject now sits in a real local slot for the duration of the match and the binding is another local directly above it; each arm ends by collapsing `[subject, binding, result]` down to `[result]` with `SetLocal` (whose stack effect is 0 — it writes the slot and leaves the value on top). A match used **mid-expression** (`out.push(match x { … })`) has temporaries below the top and therefore no meaningful slot, so a *binding* there still falls back — literal and wildcard arms compile in both positions, as before. Guards see the binding, which is what makes `x if x < 0` work. Repo-wide, the files whose top level the compiler refuses are down from 10 to 8 across this branch.
* **security(logging): `SOLI_LOG=http` no longer prints credentials in outgoing URLs.** The channel logs the full URL of every `HTTP.*` call, and a query string is an ordinary place to carry one — `?api_key=…`, `?access_token=…`. Only the *values* of credential-looking parameters are replaced now, so the endpoint and its other parameters stay readable. Completes the sweep of the production log surfaces: the error log's environment dump, its request snapshot, the query channel's bind variables and now the http channel's URLs all share one definition of what a secret is. The access line was checked and needs nothing — it logs `uri().path()`, without the query string.
* **security(logging): `SOLI_LOG=query` no longer prints credentials.** That channel exists to give production dev-grade query diagnostics, and it rendered each query's bind variables verbatim — but bind variables are exactly where a query's *values* live, so a login (`FILTER u.email == @email AND u.password_digest == @password`) wrote the submitted password into the production log. Bind values whose name looks like a credential now render as `[REDACTED]`, using the same rule as the error log's request snapshot and environment dump; everything else still prints, so the channel stays useful for debugging.
* **security(logging): a handler's local variables no longer leak secrets into the production error log.** When a request raises, the error log's `env:` line carries the failing handler's locals **by value** — so a `let api_key = "ak_live_…"` was written to disk verbatim every time that handler failed, and logs get shipped, retained and shared. Request params and headers were already redacted; locals were not, so the same secret could be hidden as a param and printed in full three lines below it. Both now use one rule (`password`, `passwd`, `secret`, `token`, `api_key`, `private_key`, `authorization`, `auth`, `session_id`, `csrf`, matched case-insensitively as substrings), which lives in one module rather than being copied per call site. Model serialisation deliberately keeps its own, narrower rule: over-hiding a model field silently changes an API's shape, while over-redacting a log costs only a little debugging context — so a field called `author` is serialised normally and still redacted in a log, and that asymmetry is documented where both rules live.
* **perf(vm): re-declaring a local in the same scope no longer demotes the handler.** `let x = 1` followed by `let x = 2` in the same function scope runs fine in the tree-walking interpreter — the second `let` writes the existing binding — but the compiler rejected it as a redeclaration, so a handler containing one ran entirely on the slower engine. It is compiled as the assignment it is. Anything involving `const` deliberately still falls back: the interpreter's behaviour there is not self-consistent (`const x = 1; let x = 2` keeps 1, while `const x = 1; const x = 2` gives 2), so the engine that defines those semantics keeps defining them. Found by compiling every `.sl` file in the repository and grouping what the compiler still refuses — this was the only entry in that list that was a plain rejection of working code rather than a documented punt.
* **perf(vm): safe navigation (`&.`) compiles natively.** It was refused, so any handler using `user&.address&.city` — the idiomatic way to walk a chain that might be absent — ran entirely on the slower engine. Earlier still it was an `unimplemented!()`, which panicked; with `panic = "abort"` in release that took the whole server down the moment a handler used it. Both forms compile now, the property read and the method call, and a null receiver **does not evaluate the arguments** (`nil&.foo(bar())` never runs `bar()`), matching the interpreter and Ruby. A handler using `&.` records zero demotions where it previously recorded two. Adding it surfaced a missing entry in `Chunk::patch_jump`'s branch list — that list is the one of the three that panics on an unlisted opcode rather than keeping a wrong offset, so it announced itself immediately instead of mis-jumping at run time.
* **perf(vm): `next` compiles natively.** It was refused, so any loop using it — the ordinary way to skip an element — ran entirely on the slower engine. It now compiles to the jump it means, in both spellings (`next` and `next()`), for `for`, `while` and range loops. Unlike `break` the iterator stays, since the same loop is about to take its next element, but the body's own locals still come off at the jump site or each skipped iteration would leave its locals behind and the stack would grow for the life of the loop. `next` inside a `try` the loop does not enclose is still handled by the other engine, the same one shape `break` defers. A `next`-heavy handler is **3.5x faster** than the engine it used to fall back to (1844us vs 6449us, median of 60 requests, production mode). A program that declares its own `next` still gets an ordinary variable.
* **perf(vm): `break` compiles natively.** It was refused outright, so *any* handler containing a `break` — the ordinary way to stop scanning once you have found what you were looking for — ran entirely on the slower engine. The refusal existed because a `break` has to unwind, at the jump site, everything the loop body pushed: body locals (closing the upvalue when a closure captured one, so bindings from different iterations stay distinct) and, for a `for` loop, the iterator, which `ForIter` only discards when the sequence runs out. A new `PopIter` handles the last of those; leaving it out would have leaked the iterator to the next loop, the same bug `return` and `throw` had. A `break` **inside a `try`** is still refused — jumping out would leave that handler live with its loop abandoned — but only that one shape, decided by comparing the open-handler depth at the `break` with the depth at the loop, so it cannot misjudge. A `break`-heavy handler is **4.8x faster** than the engine it used to fall back to (1301us vs 6245us, median of 60 requests, production mode), and a handler that scans a list and breaks now records zero demotions.
* **perf/fix(vm):** **`finally` is compiled properly now, so a handler that uses it no longer falls back to the slower engine.** The previous release shipped a deliberate stopgap: the compiler *refused* `try`/`finally` because the compiled version ran the block only when control fell off the end of the `try`, which skipped cleanup on `return` and discarded a pending exception when no `catch` clause matched. Refusing was correct but demoted the whole handler. The block is now inlined on every edge that leaves the `try` — before each `return`, and on an exception path that runs it and rethrows — using a second handler wrapped around the ordinary try/catch so a throw from *inside* a `catch` clause reaches it too. A `return` inside the `finally` still wins over a pending exception, matching Ruby's `ensure`. A `finally`-heavy loop is **5.3x faster** than the interpreter it used to demote to, and a server handler using `finally` now records **zero demotions** where the same app recorded three. The eight parity cases that tracked this are off the known-divergent list, which is what locks the behaviour in.
* **fix(engines):** **An uncaught `throw` could return `200 null` instead of failing, and a top-level `throw` did nothing at all.** Two separate leaks of the same shape. (1) A `return` from inside a `try` skips the `TryEnd` that pops the exception handler, so the compiled engine kept a handler whose catch target pointed into a frame that had already returned. The next `throw` with no newer handler above it matched that dead handler and jumped the *current* frame to an offset belonging to a different function — in a server, a handler that called any function using `try { return ... }` and then raised answered **HTTP 200 with a `null` body** instead of erroring, so a client saw success for a failed operation. It now returns 500. This is the third stack with this exact bug (values, iterators, and now handlers): `return` is the one exit that skips the instruction which would have cleaned up, so every piece of per-frame state has to be unwound there. (2) The tree-walking interpreter's top-level loop discarded the control-flow result of each statement, so a `throw` with no enclosing `try` evaporated and **the next statement ran** — `print("a"); throw "boom"; print("b")` printed both and exited 0 under `soli run` and `soli test`, while the compiled engine reported it. Both engines now stop with the same message and exit code.
* **fix(arrays):** **`order`, `all` and `includes` on a materialized array worked under `soli test` and raised in production.** `has_many`/`has_one` accessors hand back a plain array, so a controller written with Rails habits does `org.contacts.order("name").all()` — the tree-walking interpreter has always accepted that (its own comment says so), and the compiled engine answered `Cannot access property 'order' on Array`. That is the shape that passes the test suite and 500s under load. All three now work in both engines, sharing one `order_by` in `array_ops` so the two cannot order differently — the interpreter previously had its own private copy of the field lookup and the comparator. `soli check` accepts the chain too; it used to reject `order`/`all` on an array, making a working chain unreachable from type-checked code, which is the default. Sorting on a field that is **missing** from some rows is now deterministic (absent sorts first) instead of leaving those rows wherever the sort happened to put them — normal for schemaless documents, and it also makes `sort_by`/`min_by`/`max_by` deterministic on null keys. Found by diffing the type checker's method tables against the runtime in both directions, the same audit that found `String#chr`.
* **fix(strings)/fix(tooling):** **`String#chr` was advertised everywhere but implemented nowhere, and `Int` claimed two array predicates.** The method registry, the type checker and the member whitelist all listed `chr` on String, so `soli check` accepted `"abc".chr()` and the runtime answered `Cannot access property 'chr' on String` — only the two dispatch arms were missing. It is now implemented in both engines with Ruby's semantics: the first *character* (never half a multi-byte one), and `""` rather than an error for an empty string. Separately, the registry listed `none?` and `one?` on `Int`; they are array predicates and were never Int methods, so they were offered by tab completion and could not be called. Found by walking the registry and calling every one of its 358 entries — the registry describes itself as the single source of truth but nothing dispatches through it, so drift was invisible. That walk is now a test, and it fails with either drift reintroduced.
* **fix(vm):** **Closed the bug class behind the `try`-block corruption instead of just the instance.** Three separate places have to know which opcodes advance the instruction pointer by one of their operands, and each kept its own hand-written copy of that list until they drifted. `Chunk::patch_jump` panics on an unlisted opcode, so drift there is loud; the peephole's `is_jump_target` and `compact_nops` both fail *silently*, which is how `TryBegin` stayed broken. Auditing every opcode whose handler moves `ip` turned up a second unremapped one, `NullishJump` (the `??` operator), and six missing from the peephole's jump-target guard — where the cost is the optimizer fusing the instruction a branch lands on, so the branch arrives in the middle of a fused pair. All are now listed, and a single canonical `FORWARD_BRANCH_OPS` in `opcode.rs` is walked by a test that puts a removable placeholder between each branch and its target, so a variant missing from the rewrite fails there rather than in somebody's `catch` block. The test found `NullishJump` on its own before it was fixed. No measurable cost: a hot loop is within noise (+0.9%) of the previous binary.
* **fix(vm):** **The peephole optimizer corrupted every `try` block containing an optimizable instruction.** `compact_nops` rewrites jump offsets after the peephole fuses instructions, and its own doc comment states the invariant: *every opcode that advances `ip` by one of its operands must be listed*. `Op::TryBegin` carries **two** such targets — the catch and finally handlers — and neither has "jump" in its name, so it was missed exactly as `ForIter`/`ForIterRange`/`RescueJump`/`CatchMatch` had been, but silently: no test put a fusable instruction inside a `try`. Fusing one shifted every later offset, and the unremapped catch target landed one instruction *into* the catch body. What that cost depended on which instruction fused, and all three were observed: a skipped initialiser, so the catch block ran with **garbage locals** and no error (`let marker = "GOOD"` then reading `marker` gave an unrelated string); a skipped handler, so **`try`/`catch` did not catch** and the exception escaped as if no handler existed; and an offset past the end of the chunk, so the VM **panicked** with `index out of bounds`. Six of ten realistic shapes were broken — `h.n`, `i = i + 1`, `a * 2`, `h.a.b`, and any `#{...}` containing a method call — while a `try` with no optimizable instruction was fine, which is why this survived. A handler in the ordinary `try { let x = a * 2 ... } catch e { ... }` shape returned a 500 instead of running its error path. Both `TryBegin` targets are now remapped, with a unit test that checks the remapping directly and four differential cases covering the distinct failure modes.
* **fix(engines):** **A `throw` from inside `map`/`filter`/`each`/`reduce`/`sort_by` lost its value, and `sort_by` swallowed it entirely.** These methods drive the callback from Rust, so a throw has to cross that boundary to reach the caller's `catch` — and both engines destroyed it there, differently: the interpreter replaced it with a generic `Exception in array method` (losing the payload *and* the author's own message), the VM with the rendering of the value. So `rows.map(fn(r) { throw {"code": 422, "field": "email"} })` could not be caught as a hash from either engine. Worse, `sort_by` did not propagate the throw at all: it gave that element a null key and **returned the list unsorted with the exception silently gone**, while the VM raised on the same code. Fourteen callback sites now carry the value out, and `sort_by` propagates instead of swallowing — verified across 16 callback methods, all preserving the value in both engines where none did before. A thrown class instance stays an instance, so `catch e { e.message }` works through a callback.
* **fix(engines):** **`debug()` did nothing in compiled mode, and internal routing sentinels leaked into caught errors.** `debug()` returns a sentinel `Value` that the tree-walking interpreter recognises as "stop here, open the REPL"; the compiled engine evaluated the call and popped the result as an ordinary expression statement, so the breakpoint never fired — the same shape as the `next` bug, and these two are the only sentinel values that exist, so the class is now closed. Both are refused at compile time so the handler falls back to the engine that implements them, which for `debug()` also supplies the captured environment the REPL needs. The refusal now checks the compiler's known-globals set first, so a program with its own `let debug = 42` or `let next = "step"` compiles normally instead of demoting — which also narrows the earlier `next` fix. Separately, `Model.find` and `forbidden()` mark their errors with an internal sentinel so the request layer can turn them into a 404 or a 403; catching one bound that raw text, so rendering it printed `__Forbidden__:nope at 2:3` on the page. Both engines now build the caught value through one shared helper that strips the sentinel, so they cannot drift, and the 404/403 routing is untouched.
* **fix(vm, interpreter):** **`finally` did not run when a `try` was left early, and could swallow an exception outright.** The compiled engine laid `finally` out as straight-line code after the catch clauses, so it ran only when control *fell off the end* of the try — the one case where it matters least. A `return` inside the try emitted the return and left the frame without reaching it, so `try { return x } finally { conn.close() }` **leaked the connection** in production while releasing it under `soli test`; and with no catch clause the pending exception was popped and discarded, so `try { risky() } finally { cleanup() }` returned normally when `risky()` threw and the **error vanished**. Both verified against a running server, before and after. Compiling `finally` correctly means emitting it on every exit edge, which is a real piece of work and filed as one; until then the compiler refuses `try`/`finally` and the handler falls back to the interpreter, which runs it on every path — the same trade already made for `break` and `next`, and a demotion is cheaper than a leaked handle or a lost error. Separately, the interpreter ran `finally` on early exits but *discarded what it did*, so a `return` or `throw` inside a `finally` was ignored whenever the try was already unwinding; it now takes over from the exit in progress, matching Ruby's `ensure`. `soli run` and standalone builds are unaffected; a direct `soli run --vm` on such a script now reports a compile error.
* **fix(interpreter):** **A thrown value lost its type when it crossed a function call.** `throw {"code": 404}` caught in the same function gave a hash; caught one call up it gave the *string* `Unhandled exception: {code => 404} at 0:0`, so `e["code"]` failed with "cannot index string with string". Structured errors — the idiomatic way to carry a code and a message — only worked when thrown and caught in the same function body. A throw travels as a control-flow value inside one body, but crossing a call boundary means moving into Rust's `Result`, and there was nowhere for the value to ride: it was rendered to text and the `catch` re-wrapped that text as a String. It now rides in a `RuntimeError::Thrown` variant that `catch` unwraps, so the value is the value it was thrown as — hash, array, int, or instance — at any call depth. The VM has always got this right, so this is the interpreter matching production. As a side effect, a throw crossing a call is **31x faster** (4.886s -> 0.159s over 200k iterations, with an untouched control flat at -0.8%): the error path was JSON-serializing every local and building a stack trace for the dev error page, on every throw, and a value in flight to a `catch` no longer pays for that.
* **fix(vm):** **`next` was silently ignored in a loop.** `for i in [1,2,3,4] { if i == 2 { next } ... }` kept every element under the VM and skipped correctly under the interpreter — so a filtered loop quietly processed the rows it was told to skip, in production. `next` is a zero-argument builtin returning `Value::Continue`, which the interpreter recognises as "skip to the next iteration"; the VM evaluated it, discarded the value as an ordinary expression statement, and carried on. It is now refused at compile time, exactly as `break` already is, so the handler falls back to the interpreter, which implements it — the same route `break` takes. A server handler using `next` now returns the **correct** result and records one demotion; before, it returned the wrong one silently. A shadowed `next` costs a demotion and nothing else.
* **fix(vm):** **a `return` from inside a `for` loop corrupted the caller's loop.** The VM keeps iterators on a shared `iter_stack`, and only `ForIter` pops one — so returning early skipped that pop and left the callee's iterator on top. The next iteration of whatever loop was running in the *caller* then consumed the callee's leftovers: `for info in rows` printed `{k => one}` and then the inner function's string values, running five times over a three-element array. Any function that returns from inside a loop, called from inside another loop, was affected — which is an ordinary shape, not an exotic one. `CallFrame` now records an `iter_base` alongside `stack_base`, and `Op::Return` truncates the iterator stack to it, exactly as it already truncated the value stack. Found by chasing the filed `blog#index` divergence — `Cannot access property 'get' on string`, where a hash-typed loop variable had been overwritten with a string. `/docs/blog` now runs entirely on the VM: same 186,907-byte page, **demotions 1 → 0**.
* **fix(fmt):** **the formatter could emit `!n > 10`, which does not compile.** Printing a `!` whose operand binds looser than the unary operator produced `(!n) > 10` on reparse — a type error, not just a reshaping. Source written with parens carried a grouping node and printed correctly, so this only surfaced for an AST built without one, such as a desugared `unless`. The operand is now parenthesised when it binds looser.
* **fix(parser):** **a condition that begins with a parenthesis no longer ends at the closing paren.** `if (row["url"] ?? "") == ""` reported `Unexpected token '=='` — the parser consumed the leading `(` as *"parentheses around the condition"*, stopped at its match, and then choked on the operator that followed. The same expression parsed fine on the right of an assignment, so the guard had to be split into two statements. It affected `if`, `elsif`, `while` and the postfix `if`/`unless`/`return`/`throw` forms. A condition is now parsed as one ordinary expression, with `(...)` as a plain primary that operators can follow; the optional `if (cond)` form still works. **Conditions now also end with their line**, which they always needed to: `if x < 0` followed by a body line beginning `-x` would otherwise continue as `if (x < 0 - x)`. The formatter used to paper over exactly that by wrapping such conditions in parens — and that workaround was itself the reason `soli fmt` was not idempotent on one repo file, since the added parens parsed back as a grouping node, printed as parens, and got wrapped again on the next pass. With the parser fixed the workaround is gone, along with the two helpers that supported it.
* **fix(vm):** **`max` and `min` returned the first element for arrays of strings.** `["a", "b", "c"].max()` answered `"a"` and `["c", "b", "a"].min()` answered `"c"` — the VM's comparison had arms for every numeric pairing and none for strings, so a string never displaced the running candidate and the result was simply whatever came first. The interpreter compares through the shared sort comparator and always answered correctly, so this is one more case of the same code passing tests and returning nonsense in production. Found by a fifth generated axis: chaining calls so each method's return value becomes the next receiver, which is how `{"a": 1, "b": 2}.keys().max()` came to be tried at all.
* **fix(vm):** **`reduce` without an initial value did not work in the production engine.** `[1, 2, 3].reduce(fn(a, b) a + b)` — the Ruby-idiomatic form, where the first element seeds the accumulator — returned `6` in the interpreter and raised `Wrong number of arguments: expected 2, got 1` in the VM. So the shorter form ran under `soli test` and failed under `soli serve`. The VM now accepts one or two arguments, seeding from the first element when the initial value is omitted, and still reports `reduce on empty array requires initial value` for an empty receiver. Alongside it, **every closure-taking method now names itself when handed a non-function**: `[1,2,3].all?(null)` reported the bare `Cannot call non-function value` from the VM — discovered only when it tried to invoke, naming neither the call nor what it wanted, which is useless mid-chain — and now says `all? expects a function argument`, as the interpreter always did. That covers 18 methods across arrays and hashes. Two shared arms also reported the wrong name: `select(...)` blamed `filter`, and `fold(...)` blamed `reduce`. Found by a fourth generated axis — every method receiving each **wrong argument type** — which started at 210 divergences out of 826.
* **fix(strings):** **a user-supplied padding width could abort the process.** `"x".ljust(9223372036854775807)` asked the allocator for nine exabytes and took the whole process down with `SIGABRT` — in **both** engines, and reachable from any request that passes a number through `to_i()` into `ljust`, `rjust`, `center`, `lpad` or `rpad`. One crafted parameter killed a worker. All five now reject a width above 1,048,576 characters with an ordinary error; a million characters is already far past any real column alignment. `truncate` is deliberately **not** capped — its argument shortens rather than allocates, so `"abcdef".truncate(9223372036854775807)` correctly returns `"abcdef"`; capping it was a regression in the first attempt, caught by the parity sweep. Found by a third generated axis: same methods, same receivers, hostile *argument values* — negatives, zero, and the `i64` limits.
* **fix(vm):** **`substring` crashed the process on non-ASCII text, and `sum`/`min`/`max` were wrong for floats.** `"é".substring(0, 1)` **panicked** — the VM sliced the *byte* range, and `é` is two bytes, so the slice split a UTF-8 character. It now counts characters, sharing one implementation with the interpreter (with an ASCII fast path, where bytes and characters coincide). Separately, the VM's zero-argument fast-path table computed `sum`, `min` and `max` over `Int` values only and **silently skipped everything else**, shadowing the correct implementation behind it: **`[1.5, 2.5].sum()` was `0`**, `[1, 2.5].sum()` was `1`, `[1.5].min()` was `null`, and `[1, "a"].sum()` was `1` where the interpreter raised. Summing an array of floats — money, measurements — returned zero in the engine that serves production while the engine that runs the tests answered correctly. Those fast paths now bail out on the first non-integer and let the full implementation take over. Also: `fetch` answered `null` for a missing key instead of raising, sharing an arm with `get` when Ruby distinguishes exactly there (`get` → null, `fetch` → raise, `fetch(k, default)` → default); and `key not found: String("a")` leaked Rust's `Debug` formatting into a message a Soli user reads.
* **fix(interpreter/vm):** **`sort_by` silently returned unsorted data when the key expression failed, and five smaller divergences.** The interpreter matched on the whole `Result` from the key lambda and mapped every failure to `null`, so `rows.sort_by(fn(r) r.typo())` gave every element the same key and returned the list **in its original order with no error** — while the VM raised on the same code. Its four siblings (`reject`, `none?`, `one?`, `count`) already propagated correctly; only `sort_by` was missing the `?`. Alongside it: `hash.size()` worked in the VM and raised in the interpreter (`len`/`length` worked in both); `arr.push(x)` returned the array in the VM and `null` in the interpreter, so `a.push(1).push(2)` chained in production and broke in tests — it now returns the array, as Ruby does; `arr.get(oob)` raised in the interpreter where the VM, `hash.get`, `first`, `last`, `pop` and `shift` all answer `null`; the VM reported **comparison operands backwards** (`[1] > 1` said "Cannot compare int and array", because `a > b` is evaluated as `b < a` to reuse one comparator); and arithmetic type errors differed only in capitalisation between engines. All found by generating the parity corpus from the engines' own dispatch tables — 215 expressions covering every method — rather than hand-picking, which took the corpus from 12 divergences to 0.
* **fix(tests):** **the standalone-build tests leaked 1.5 GB of `/tmp` on every run.** The shared fixture — two 762 MB standalone executables, built once and reused — was held in a `tempfile::TempDir` parked inside a `static OnceLock`. Rust does not run destructors for statics at process exit, so the `TempDir`'s cleanup never happened and each `cargo test` left another directory behind. Twenty-five accumulated runs took the filesystem to 99%, and the symptom was six standalone tests failing with **nothing in the output mentioning disk space** — deleting the leaked directories made the identical suite pass 2525/2525 with no code change. The fixture now lives at a fixed path under `target/`, cleared on the way in: the one-time build is preserved, the cost is bounded to a single directory every run reuses, and `cargo clean` reclaims it. Verified flat at 1.5 GB across three consecutive runs, and zero `/tmp` residue after a full suite.
* **fix(interpreter/vm):** **the two engines now word an arity error identically.** Passing an argument to `class`, `nil?`, `blank?`, `present?` or `inspect` reported *"cannot call non-function value"* from the interpreter — which resolves the member to its answer and then tries to call that answer, so the message described the symptom rather than the mistake — and *"cannot access property 'nil?' on int"* from the VM's primitive path, where the lookup missed *because of* the argument and so blamed the method. Both now say `Wrong number of arguments: expected 0, got 1`, for every receiver type: String, Array, Hash, Int, Float, Bool and Null. `to_s`/`to_string` are deliberately excluded, because on an `Int` they take an optional radix — `255.to_s(16)` is `"ff"` — and a first attempt that treated them as universally zero-argument broke exactly that, which the existing `test_vm_int_to_s_radix` caught on the first full run.
* **fix(vm):** **zero-argument universal methods accepted and discarded extra arguments.** `"abc".nil?("junk")` returned `false` under the VM and raised in the tree-walking interpreter — so, once again, a nonsense call errored under `soli test` and answered quietly under `soli serve`. `class`, `nil?`, `blank?`, `present?`, `inspect`, `to_s`/`to_string`, and (on strings) `join`, `to_f`/`to_float` now reject extra arguments in both engines, guarded in one place per receiver type rather than arm by arm. `to_i` is deliberately excluded — it takes an optional radix. Legitimate calls are untouched: `[1,2,3].join(",")` still joins, `"abc".join()` still returns the string, `"ff".to_i(16)` still parses.
* **fix(types/arrays):** **eight more methods were unreachable through `soli check`, and `[1, 2].to_s()` worked in production while raising in tests.** `uppercase`/`lowercase` (runtime aliases of `upcase`/`downcase`) were unknown to the checker entirely; `delete_prefix`/`delete_suffix` were grouped with the zero-argument methods so passing the affix they strip was rejected; `casecmp?`, `ascii_only?`, `assoc` and `rassoc` were missing outright. Separately, `to_s` on an **array** was answered by the VM and not by the interpreter — the same split hashes had — so `[1, 2].to_s()` ran under `soli serve` and raised under `soli test`; it is now an alias of `to_string` in both.
* **fix(types):** **`soli check` rejected six more working calls, all optional arguments the checker did not know about.** `config.get("port", 8080)` and `fetch(key, default)` return the default when the key is missing — implemented in both engines, declared to the checker with one parameter, so a correct call failed to type-check. Same for the optional pad string on `center`, `ljust`, `rjust`, `lpad`/`rpad`, and the omission marker on `truncate`: `name.ljust(20, ".")` ran fine and would not check. Together with `count`/`index_of`/`scan`/`partition`/`rpartition` earlier this cycle, that is **eleven** methods whose declared arity did not match their implementation. A checker that refuses working code costs as much as one that misses broken code, so these are pinned by specs now.
* **fix(arrays):** **`[].pop()` raised in one engine and returned `null` in the other** — and the VM disagreed with *itself*, carrying two implementations of `pop` where the method-table one (which wins) returned `null` and the array-methods one raised. Every sibling already returns `null` on an empty collection in both engines — `shift`, `first`, `last`, `min`, `max`, `avg` — and so does Ruby, which makes `pop` the sole outlier. All three implementations now return `null`. Because `soli test` runs the tree-walking interpreter and `soli serve` runs the VM, the same `arr.pop()` raised in the test suite and quietly returned `null` in production.
* **fix(vm/interpreter):** **`inspect` dropped the quotes on strings nested in arrays and hashes, and `hash.to_s()` behaved differently in each engine.** The VM rendered `[1, "a"]` as `[1, a]` and `{"k": "v"}` as `{k: v}` — using the *display* form for what is meant to be an unambiguous debug rendering, and inconsistently, since a bare `"s".inspect()` did quote. The VM now shares the interpreter's recursive renderer, which also gives it the existing pretty-printer for long values. Separately, `to_s` on a hash worked in the VM and raised in the interpreter; it is now an alias for `to_string` in both. Aliasing it exposed a latent bug in the interpreter's fallback renderer, which formatted a hash as `[a => 1]` with **array brackets** — invisible until now because `to_string` is answered by a faster path. Two parity cases pin all of it.
* **fix(interpreter):** **a typo in a hash method name silently returned `null` instead of raising** — so `record.lenght()` passed the test suite and then failed in production. `soli test` runs the tree-walking interpreter and `soli serve` runs the VM; the VM already raised `Cannot access property`, but the interpreter's zero-argument member fallback (the rule that lets `n.abs()` work) handed back the `null` that a missing hash key resolves to. Both engines now raise the same error. The check runs *after* member resolution rather than before, so anything the member access can still resolve keeps working — `shift`, methods added by `define_method`, and universal members like `nil?` / `class` / `to_json`. The first attempt checked before resolution and broke exactly those, which the spec suite caught. Three parity cases pin it.
* **fix(vm):** **a function stored in a hash could not be called under the VM — so dispatch tables passed every test and failed in production.** `handlers.on_create(record)` worked in the tree-walking interpreter, which looks the key up and invokes it, but the VM's hash dispatch raised `Cannot access property 'on_create' on Hash` without ever checking whether the key held a callable. Since the VM is what `soli serve` runs and the tree-walker is what `soli -e` runs, the failure mode was the worst kind: green locally, broken once deployed. The VM now looks the name up when it is not a built-in hash method and invokes it if it is callable, matching the interpreter for zero, one and many arguments. Built-in method names still win over entries of the same name — `h.delete(k)` removes a key rather than calling `h["delete"]` — in both engines, as before. Five cases are pinned in `tests/differential_engines_test.rs`; three of them fail against the previous build.
* **fix(cli):** **`soli --vm -e '…'` silently ignored `--vm`** and ran the tree-walking interpreter, so the one command you would reach for to check VM behaviour was the one command that could not. `run_eval` now branches on the flag exactly as `run_script` does. This is how the hash-dispatch divergence above stayed invisible: every quick check through `-e` was measuring the wrong engine.
* **fix(types):** **`soli check` rejected five working String methods.** `count`, `index_of`, `scan`, `partition` and `rpartition` were declared to the type checker as taking *no* arguments. A zero-argument member gets auto-invoked, so `s.count("a")` resolved to `Int` and the checker then reported `Cannot call non-function type 'Int'` — a valid, runnable call rejected at check time, with an error message that pointed nowhere near the cause. All five now declare the argument they actually take.
* **fix(datetime):** **every accessor raised `Invalid timestamp` on a pre-1970 instant carrying sub-second precision.** All 21 conversion sites split the stored epoch nanoseconds by hand — `t / 1_000_000_000` for the seconds and `(t % 1_000_000_000) as u32` for the remainder — but the remainder is *negative* before 1970, and casting it to an unsigned 32-bit value wrapped it to roughly 4.29 billion nanoseconds, which is not a valid sub-second offset. `chrono` rejected it and the accessor raised. `DateTime.parse("1969-07-20T20:17:00.500Z")` succeeded and then `.year()` failed. It hid for so long because a whole-second instant has a zero remainder and slips through unharmed — only sub-second precision exposes it. All 21 sites now use the infallible `chrono::DateTime::from_timestamp_nanos`, which handles negative instants correctly and removes the `Option` handling that was papering over it. Performance is unchanged (median of 6 runs: +1.1%, distributions overlapping). Five specs pin it, and they fail against the previous build.
* **fix(arrays):** **`pluck` and `pick` silently returned `null` for every row from the ORM.** Their field accessor handled hashes and array rows but not instances — and instances are exactly what `Model.all()`, `where(...)` and friends give you, so `User.all().pluck("email")` came back as a list of nulls rather than raising. The accessor now reads instance attributes, and is shared by `pluck`, `pick` and all twelve field-keyed methods in both engines, so a row is read the same way everywhere. (`grouped {}` deferreds resolve to the row they stand for, too.)
* **fix(serve):** **a panic in one worker no longer aborts the whole server.** `[profile.release]` set `panic = "abort"`, under which `std::panic::catch_unwind` never catches — the process aborts at the panic site. That silently made dead code of both worker supervisors (the web-worker restart loop in `serve/mod.rs` and the background-job pool loop in `serve/background_jobs.rs`), each of which was written specifically to contain a panic to one worker. In every released binary through v1.24.1, one panic in any worker aborted the process and took every other worker with it — a real exposure given ~1,850 `unwrap()` calls in `src/`, much of it reachable from user Soli code through the builtins. Release builds now unwind. On top of that, a **per-request `catch_unwind`** (`serve::run_caught`) turns a panicking handler into an immediate `500` and leaves the worker serving; previously the handler never replied, so the caller waited the full 40s `RESPONSE_WAIT_TIMEOUT_SECS` for a `504` from a worker that was already dead. Panics are counted in `soli_handler_panics_total` on `/_metrics`. Because a setting this quiet should not be able to come back unnoticed, a `#[cfg(panic = "abort")] compile_error!` in `serve::shutdown` now **fails the build** if it does.

### Added

* **feat(language):** **block-form `unless ... end`.** `unless` existed only as a postfix modifier, so the multi-line guard shown in the project's own instructions — `unless ["up", "late", "overdue"].includes?(status)` over several lines — did not parse. It now does, desugaring to `if !cond` exactly as the postfix form already did, and accepts an `else` branch. `elsif` after `unless` is deliberately not accepted: "unless A, else if B" reads as a puzzle rather than a guard, which is why Ruby rejects it too.
* **dev:** **a hostile-input scan** at `bench/engine-parity/panicscan.sh` — the third sweep axis, after engine parity and checker accuracy. It pushes deliberately awful arguments (negative widths, out-of-range indices, zero chunk sizes, `i64::MIN`/`MAX`) through both engines and reports anything reaching a Rust panic rather than a Soli error. **Zero panics across 47 expressions in both engines**, and the two agree on every one. It did surface something neither other axis can see, because it is neither a divergence nor a checker gap: **integer arithmetic wraps silently on overflow** — `9223372036854775807 + 1` is negative, `2.pow(64)` is `0`. That is a language-level decision rather than a bug, and it matters here because Soli's own documentation teaches storing money as integer cents, so it is filed at `tasks/todo/integer-overflow-wraps-silently.md` with the four options costed rather than changed unilaterally.
* **dev:** **an engine-parity sweep** at `bench/engine-parity/`, which runs one expression at a time through both engines and reports any whose output differs — the wide net for finding *new* divergences, where `differential_engines_test.rs` is the regression gate for known ones. Currently **0 divergences across 3,941 expressions** spanning String, Array, Hash, Numeric, DateTime, Duration, closures, indexing and error paths. It only became possible once `soli -e` started honouring `--vm`; before that, both sides of every comparison ran the same engine, which is how four divergences fixed this cycle survived.
* **feat(arrays):** **`sum_by`, `group_by`, `index_by`, `count_by` and `tally` — field-keyed aggregates that beat Ruby.** Each names the field as *data* rather than taking a callback, so the whole traversal stays in Rust and never re-enters the interpreter. That is the entire performance story: `rows.sum_by("amount")` and `rows.reduce(fn(a, r) { return a + r["amount"] }, 0)` compute the same thing, but the closure form calls back into Soli once per element and measures **~235x slower** on 20k rows. Against Ruby 4.0.6 ZJIT, whose equivalents take a block and cannot escape that cost: **`sum_by` 3.8x faster, `tally` 6.2x, `count_by` 3.0x, `index_by` 2.9x, `group_by` 1.9x — 5/5, geometric mean 3.3x faster.** `sum_by` keeps integers integral so money-as-cents does not silently become a float, promoting only once a float is seen; missing and non-numeric fields are skipped rather than raising, matching `pluck`. `group_by` preserves first-seen key order and within-group order; `index_by` is last-write-wins like Rails; a record missing the field groups under `null` so counts still total the input length.
* **feat(arrays):** **seven more field-keyed methods — `avg`/`avg_by`, `filter_by`, `find_by`, `uniq_by`, `max_by` and `min_by`.** The same bargain as the aggregates above: naming the field as data keeps the traversal in Rust, and Ruby's block-taking equivalents cannot. With these the family is **11 of 11 against Ruby 4.0.6 at its best of interpreter/YJIT/ZJIT, geometric mean 2.5x faster** — `min_by` 3.3x, `filter_by` 3.2x, `avg_by` 3.1x, `find_by` 3.0x, `max_by` 2.4x, `uniq_by` 1.6x. `filter_by`/`find_by` deliberately reuse the `Model` method names so an in-memory filter and a database one read identically. Semantics: `avg` is always a `Float`, because an average is a ratio and integer division would report `[2, 3].avg()` as `2`; averaging nothing yields `null` rather than a `0` indistinguishable from a real zero mean. `max_by`/`min_by` return the *record*, not the value, and skip records missing the field instead of letting a null win — so a partly-populated list still answers usefully. `uniq_by` keeps the first of each group, like Ruby. **Passing a closure where a field name belongs now raises** and names the block-taking alternative (`max_by(fn(x) ...)` &rarr; `sort_by(fn(x) ...).last()`); previously it matched no field and returned an empty or zero result silently — a wrong answer with no error. The four earlier aggregates gained the same guard. `sort_by`'s value comparator, which had been duplicated per engine and already drifted cosmetically, is now shared with `max_by`/`min_by` so the three cannot disagree.
* **docs:** **a published Soli-vs-Ruby benchmark page** at `/docs/getting-started/benchmarks`, generated from real measurements rather than hand-written numbers. 67 operations across Aggregate, String, Array, DateTime, Hash, Duration, Numeric and control flow, with Soli's production VM against **Ruby 4.0.6 in all three modes** (interpreter, YJIT, ZJIT) — Ruby credited with its *best* result, on **byte-identical inputs** generated by the same LCG in both languages. Soli leads on 33/67 with an overall geometric mean of 1.05x, and the split is structural rather than random: Soli wins where work happens inside Rust (Aggregate 2.5x overall and 11/11; String 2.8x; `DateTime.parse` 33x, `capitalize` 14x, `flatten` 4.8x), Ruby wins where work happens per-element in interpreted code (Numeric 4.1x, calls 4.8x, Hash 2.2x). The page states the cause plainly — 24-byte `Value` vs Ruby's 8-byte tagged `VALUE`, ~62ns/iteration for a bare loop against ZJIT's ~13ns — flags that `Duration` is *not* a like-for-like comparison (Ruby has no Duration type), and says outright it measures operations rather than request throughput. It also carries a **Corrections** section documenting that the first draft's data flattered Soli (`sort` on pre-sorted input, set ops on identical arrays, `flatten` on a flat array) and what the honest numbers became. Matched scripts ship in `bench/cross-language/`.
* **feat(serve):** **graceful shutdown — rolling deploys no longer truncate in-flight requests.** `SIGTERM`/`SIGINT` went straight to `std::process::exit(0)`, so every rolling deploy, container restart or `systemctl restart` cut off whatever was being served. (A partial mechanism existed but was never wired: a `shutdown_flag` was allocated and read on every request, and nothing ever set it.) The server now drains: `/_ready` starts answering `503`, so the load balancer takes the instance out of rotation; new requests get `503 Server shutting down` with `Connection: close`; **requests already in flight run to completion** and return their real response; then the process exits `0` — once the last connection finishes, or after `SOLI_SHUTDOWN_GRACE_SECS` (default `25`, chosen to sit under Kubernetes' 30s default `terminationGracePeriodSeconds`). A second signal skips the wait. The accept loop deliberately keeps running during the drain: breaking it would return from `block_on` and drop the tokio runtime, killing the very connections being drained, and a closed listener would give the load balancer a TCP refusal instead of a readable `503`. Exit is via `exit`, never `abort`, so `atexit` handlers — including the `cargo llvm-cov` profile flush — still run.
* **feat(serve):** **`/_health` and `/_ready` endpoints.** `GET|HEAD /_health` is liveness: `200 ok` for as long as the process serves, **including mid-drain** — a shutting-down process is healthy, and failing here would make an orchestrator restart a container that is already exiting cleanly. `GET|HEAD /_ready` is readiness: `503 starting` before the worker pool is up, `200 ready` while serving, `503 draining` for the whole shutdown. Both are plain text, unauthenticated, always on, and exempt from the drain's blanket `503` — a readiness probe swallowed by the drain check cannot do its one job. See [Configuration](/docs/getting-started/configuration#health-checks).

### Performance

* **perf(arrays/values):** **`join` on numbers −65%, `avg` −80%** — two allocation bugs found by benchmarking every array method rather than only the ones on the comparison page. `Value::display_len` computed an integer's width with `n.to_string().len()`, heap-allocating a `String` purely to read its length and then dropping it, and `write_to_string` then allocated a *second* one to render the same value — so `join` on a 1000-element integer array made 2000 throwaway allocations. Width is now counted arithmetically and rendering goes through `itoa`'s stack buffer (`itoa` and `ryu` were already dependencies). Integers have a single decimal form, so output is byte-identical; floats deliberately keep `to_string`, because `ryu` renders `2.0` where Rust renders `2` and that would change what every program prints. Two tests pin rendering *and* the reported width against `to_string` at every digit boundary and at `i64::MIN`/`MAX`. Separately, `avg` cloned every element while the neighbouring `sum` read through references — it now borrows, which is where the other 80% went. Measured on 1000 elements: **`join` 5.95 → 2.09 ms**, **`avg` 0.785 → 0.157 ms**. Every `join`, `to_string` and `#{}` interpolation of an integer benefits.
* **perf(strings):** **`reverse` 8x faster, `swapcase` −70%, `chars` −25%** — three methods were decoding and re-encoding UTF-8 where the answer needed neither. `reverse` and `swapcase` now take a byte-wise path when the input `is_ascii()`, which is nearly every string a web app touches: reversing ASCII bytes *is* reversing ASCII characters, and swapping ASCII case is a single pass. The Unicode path stays for everything else and is still the only correct one — `"Straße".swapcase()` is `"sTRASSE"`, where the mapping changes length, so it cannot be done in place. `chars` was allocating a whole `String` per character via `c.to_string()`, then converting it to a `SoliStr` that stores one character *inline* — an allocation, a copy and a free per character, all discarded. It now encodes into a stack buffer. Measured on the benchmark suite: **`reverse` 0.060 → 0.009 ms**, moving from 2.80x slower than Ruby to **0.49x — 2x faster**, and `chars` 0.43x → 0.33x. On a 53-character string, `swapcase` 398 → 119 ns and `chars` 1033 → 779 ns. String is now **12 of 13 against Ruby at its best, geometric mean 3.2x faster**; only `interpolate` still loses. Written once and shared: `reverse` had **six** copies across the two engines, `chars` three and `swapcase` three, and single-character indexing (`s[i]`) shared the same wasted allocation at three more.
* **perf(vm/objects):** **object construction −16% to −31%; per-field cost −37%.** Creating an object is the most common thing an application does that is not a field read, and it was paying for it three times over. Measured first: construction cost **89 ns fixed plus 84 ns per field**, so an 8-field object cost 856 ns. Three causes, all fixed. (1) `Instance.fields` was keyed by `String`, so **every field name was a heap allocation** — the key type is now `SoliStr` (`EcoString`), which stores names up to 15 bytes inline, and since `EcoString: Borrow<str>` every existing `.get(name)` lookup kept working untouched. (2) The `SetProperty` fast path only handled *updates*; a first write fell through to a general path that allocated the name **twice** (once in `read_string_constant_owned`, once in `op_set_property`'s `name.to_string()`) — and every field of every constructor is a first write. It now inserts directly, cloning the name already in the constant pool. (3) `Instance::new` built a default-capacity map, so a constructor filling in fields rehashed partway through; it now sizes from the class's declared field count. Measured A/B against the preceding commit, consistent across every round: **1 field −16%** (214 → 181 ns), **4 fields −29%** (484 → 344 ns), **8 fields −31%** (856 → 587 ns); per-field 92 → 58 ns. Native constructors gain from the same change — `Duration.of_days` −8%, `DateTime.from_unix` −8% — and 90 call sites that wrote `"_ts".to_string()` and the like now pass the literal straight through with no allocation at all. Model instances, controller instances, `DateTime`, `Duration` and every user class go through this path.
* **perf(vm):** **instance field access −65%, class-heavy code −21 to −25%** — two super-instructions that already existed and were already implemented, but that nothing ever emitted. `GetLocalProperty` and `GetLocalIndex` were declared in the opcode enum, executed in `run_dispatch`, carried stack-effect entries and had disassembler support; the peephole optimiser simply never produced them, so `obj.field` stayed `GetLocal + GetProperty` and `arr[i]` stayed `GetLocal + GetLocal + GetIndex`. This is the same shape of bug as the `compact_nops` stub: infrastructure built, never wired. Measured first, so the fix aimed at the right thing — a bare instance field read cost **+27 ns** over a local read while a whole native `String.len()` call cost **+8.5 ns**, and the cost was flat in field count (1 vs 8 fields) and nearly flat in name length (1 vs 35 chars), so neither probing nor hashing was to blame: it was the second dispatch. `GetLocalProperty` also needed the instance-field fast path that plain `GetProperty` already had — without it the fused form would have been *slower*, because the general path allocates the field name as a `String`. Field access now costs **+8 ns** instead of +27. Interleaved A/B, consistent across every round: **200k field reads −25%** (30.0 → 22.6 ms), a **method body with four `this.` reads −21%** (59.2 → 46.9 ms), **`hash[key]` −17%**, `arr[i]` −6%. This does not move the cross-language benchmark page, which does not exercise instance fields — the win is in the OO code real apps are made of, where every model attribute read and every `this.` in a controller goes through this path.
* **perf(arrays):** **`flatten` −33%, `intersection` −14%, `difference` −9%** — three allocation and hashing fixes, no semantic change. `flatten`, `intersection` and `difference` all grew their result from an empty `Vec`, so a 20k-element result paid ~14 reallocations and the doubling memcpy behind them; each now sizes the result up front. `intersection` and `difference` also used two hash sets and hashed every element twice — once for "is it in `b`", once for "have I emitted it" — where one set answers both: seed it with `b`, then *remove* on a hit for intersection (a hit means it was in `b`; the removal stops a repeat matching again) or *insert* for difference (success means neither in `b` nor already emitted). That retired `ValueSet::contains` entirely, since every caller needs to record something alongside the answer. Measured on 20k elements against the same Ruby baseline: `flatten` 0.171 → 0.115 ms, `intersection` 1.164 → 1.001 ms, `difference` 1.593 → 1.450 ms. The Array category moves 1.24x → 1.13x and the suite overall 1.14x → 1.10x. Also documented on the benchmark page: the `difference` row is **not** like-for-like, because Soli deduplicates its result where Ruby's `Array#-` keeps duplicates from the left operand — Soli is doing strictly more work there.
* **perf(arrays):** *not* changed, and now commented so it stays that way: `reverse` clones then reverses in place, which looks like two passes where `iter().rev().cloned().collect()` is one. The single-pass version was tried and measured **12-15% slower** (at 20k and 200k elements) — the clone is a forward vectorized memcpy and the reverse a vectorized two-pointer swap, while collecting from a reversed iterator is a backwards scalar walk that defeats the prefetcher.
* **perf(vm/interpreter):** **native instance-method calls no longer heap-allocate their argument list.** Natives receive the receiver as `args[0]`, so every call had to materialise `[this, ..args]` — a fresh `Vec` on `dt.year()`, `s.upcase()`, `arr.sum()`, every closure invocation. Now that `NativeFn` takes a slice, the common case builds that list in a stack array and lends it out; 560 of the 565 fixed-arity natives take three arguments or fewer, so the heap path is genuinely rare. Measured, interleaved, 6/6 rounds unless noted: **`Array.sum` −29.3%**, **`Control.fn_call` −15.6%**, `closure_call` −12.3%, `String.bytes` −10.6%, `DateTime.to_unix` −10.4%, `Array.reverse` −13.6% (4/6). The Control category moved 4.15x → 3.80x against Ruby ZJIT.
* **perf(builtins):** **native functions take `&[Value]` instead of an owned `Vec<Value>`.** Every one of the ~1000 builtins had its arguments copied into a fresh heap allocation before the call — including the zero-argument calls that dominate (`dt.year()`, `s.upcase()`, `d.total_seconds()`), where the allocation was pure overhead. The callable is now `pub type NativeFn = Rc<dyn Fn(&[Value]) -> Result<Value, String>>`. Measured: **DateTime/Duration median −2.3%**, `String.index_of` −7.8%, `Array.build` −4.7%, `DateTime.year` −4.4%. Modest because the instance path still allocates once more to prepend `this` — removing that needs a calling-convention change (receiver passed separately), which is the natural follow-up. Scope note for anyone attempting similar: the type system did almost all the work — of ~1000 closures only **3** genuinely moved out of their arguments (`json.rs`'s `swap_remove`, now a clone at a parse entry point), and the compiler located every remaining site.
* **perf(vm):** **the compiler was emitting dead instructions into every hot loop.** The peephole optimiser fuses instruction sequences by overwriting the head and blanking the tail with `Op::Nop` — but those NOPs were then shipped in the emitted bytecode and *dispatched at runtime*. Because fusing is most aggressive exactly where code is hottest, they concentrated in tight loops: a counter loop ran **ten instructions per iteration, five of them NOPs**. Half the dispatches did nothing. A `compact_nops` pass existed but was a stub that returned immediately, its comment reasoning that peephole NOPs could not be told apart from real `Pop`s — no longer true, since `NOP` is `Op::Nop`, its own variant. It now genuinely compacts the code and rewrites every jump offset. Measured, interleaved, 5/5 rounds each: **`int_loop` −26.7%**, `Array.build` −13.6%, `Hash.has_key` −13.3%, `Hash.get` −12.2%, `modulo` −11.9%, `float_math` −11.4%, `closure_call` −7.7%. A bare `while` iteration drops from ~62 ns to ~45 ns. **Every opcode that advances `ip` by an operand must be listed in the remap** — the first attempt missed `ForIter`, `ForIterRange`, `RescueJump` and `CatchMatch` because none has "jump" in its name, and `differential_engines_test` caught it immediately with six divergences; the invariant is now documented at the function.
* **perf(vm):** **in-place local accumulation** — `s = s + i` compiled to `AddLocalLocal` + `SetLocalPop`, a push and a pop of a 24-byte `Value` (with drop glue, since five variants hold an `Rc`) for a value that immediately went back into the slot it came from. It now compiles to a single `AddLocalsInPlace` that mutates the slot directly, with no stack traffic: **+3.4% on a tight accumulator loop, 5/5 interleaved rounds**. Deliberately the only in-place arithmetic opcode — `i = i + 1` and `d = d - 1` were already collapsed to `IncrLocal`/`DecrLocal` by earlier peepholes, so const and subtract variants were written, measured as unreachable, and removed rather than shipped as dead code. A test pins that invariant so they don't get re-added.
* **perf(vm):** **constant-key hash reads are up to 21% faster** — the super-instructions that existed for them now actually fire in real code. Three gaps, all measured with both binaries running concurrently and interleaved rounds:
  * **Index syntax was never fused.** `hash.get("k")` compiled to the single `HashGetLocalConst` super-instruction, but `hash["k"]` fell back to `GetLocal` + `HashGetConst` — two dispatches. That made the idiomatic spelling the slower one (79 ns/iter vs 63 ns), and `params["name"]` is the most common hash read in controller code. Index syntax now emits the same fused opcode: **96 → 76 ns/iter (−21%, 6/6 rounds)**.
  * **The global super-instructions never fired inside a function.** `HashGetGlobalConst` and friends were gated on `scope_depth == 0`, a proxy for "cannot be a local" — but it also excluded every function body, which is where all real handler code lives. The gate now uses the compiler's own `resolve_variable`, which reports `Local`/`Upvalue`/`Global` honestly: **103 → 88 ns/iter (−14%)**. An upvalue receiver correctly falls through to the generic path — emitting the global form for a captured binding would have read the wrong variable, which the `scope_depth` check was bluntly (and over-broadly) preventing.
  * Index syntax on a global gets the same treatment: **101 → 90 ns/iter (−11%)**.

  * **Writes had the same gap.** `h.set("k", v)` fused; `h["k"] = v` — the far more common spelling — did not. Now fused too: **86 → 66 ns/iter (−23%)**. Safe because both sequences leave exactly one value on the stack (the generic form pops receiver+value and pushes `Null`; the fused form pops value and pushes `Null`), and only a side-effect-free plain-variable receiver is ever fused.

  Semantics are unchanged — the fused and generic opcodes already produced identical results and the identical `NoSuchProperty` error for non-hash receivers, and shadowing/capture cases are covered by tests on both engines. With this, `has_key` on a local hash is now **1.1× faster than Ruby 3.4.9**.

* **perf(collections):** **`uniq`, `union`, `intersection` and `difference` were quadratic — now linear.** Each scanned its own output with `Vec::contains` to test membership, making them O(n·k) in the number of distinct elements. Measured on an all-unique array, `uniq()` cost 1 ms at n=1000 but **295 ms at n=16000**, quadrupling on every doubling of n — a third of a second inside a single method call. They now use a hash set. `uniq()` at n=16000: **295 ms → 0.90 ms (327× faster)**; `intersection` at n=4000: **35.2 ms → 0.43 ms**; string `uniq` at n=4000: **35.8 ms → 0.20 ms**. Every array and hash method now scales linearly (verified by a 4×-size sweep). The set operations had been implemented **four** times over (the interpreter's borrowed fast path, its owned path, the VM, and the `Array` class) and all four are now the shared helpers in `array_ops.rs` — the borrowed fast path is the one a plain `[...].uniq()` actually reaches, and an inline copy there silently kept the old behaviour until it was found by re-measuring. Semantics are preserved exactly, including the awkward parts: `Value`'s cross-type numeric equality (`[1, 1.0].uniq()` is one element), `-0.0 == 0.0` despite differing bit patterns, `NaN != NaN` (every NaN survives), and structural equality for arrays/hashes — all pinned by tests. Hash methods were measured and were already fine: `get`/`has_key` are flat in collection size (`IndexMap` + ahash), and `merge` is linear.

* **perf(views):** **~10% less CPU per request on pages that cannot use the response cache.** `render()` opened by computing `data_signature(data)` — a full recursive walk that hashes every byte of every string in the render data, so on a list page it is proportional to the whole result set. The dirty flags that decide cacheability are set *during* the render, not before it: `csrf_meta_tag()` calls `csrf_token()`, which marks the response dirty, and the layout renders after the cache lookup but before the store. So such a page looked clean on entry, paid the full hash, missed (the store had been refused last time for the same reason), rendered, and was refused again — every request, forever. Since the default `soli new` layout calls `csrf_meta_tag()`, this was the normal path for real apps rather than an edge case. `render()` now remembers `(template, layout)` pairs whose store was refused and skips the signature entirely for them, turning an O(data) hash per request into a set lookup. Measured on a 300-row list page under the scaffolded layout, idle 16-core box, both binaries served concurrently with interleaved rounds: **372 µs → 330 µs CPU per request (-11.3%)** and **3076 → 3522 req/s (+14.5%)**, faster in 8/8 rounds with non-overlapping ranges. The mark is deliberately sticky and per render site — a page that is dirty only sometimes forfeits caching rather than risking a stale body — and it is cleared by `clear_cache()` so a hot-reloaded view is re-evaluated. Pages that *do* cache are unaffected (verified: still zero re-renders across 100 requests).

### Changed

* **refactor(serve):** extracted `dispatch_http_request`, removing ~74 lines duplicated verbatim between `worker_loop`'s two dispatch arms (the non-blocking batch drain and the `select`-blocked path) and giving the per-request panic guard a single home.


## [1.24.1] - 2026-07-24

### Security

* **fix(deps):** **bump `ammonia` to 4.1.4 — RUSTSEC-2026-0213 (XSS via SVG animation tags).** `ammonia` 4.1.3 let an attacker smuggle script through SVG `animate` / `set` elements, so anything sanitizing untrusted HTML with its default tag set could emit an XSS vector. Soli's own `sanitize_html` was **not** exposed — `sanitize_builder()` replaces ammonia's defaults with an explicit 22-tag allowlist that contains no SVG elements, so `animate` and `set` were already stripped — but the advisory failed `cargo audit`, which gates every push and PR. The bump clears it; no Soli-facing behavior changes.

### Added

* **feat(native):** **CSRF-safe device registration, jsQR path, desktop protocol helpers.** `POST /devices` and `/sync/*` generators call `skip_csrf` (session still required) so shell token POSTs without Origin work; pages use `soli.nativeBridge.registerDevice` + `csrf_meta_tag`. Barcode decoder auto-tries same-origin `/js/jsQR.min.js` then CDN. `soli desktop build` writes `register-protocol.sh` / `.ps1`; `soli desktop register-protocol` re-emits them.
* **feat(native):** **close the native product loop — generate shells, devices, deep links, WebKit scan, offline outbox.** `soli generate devices` scaffolds a `Device` model, `POST /devices` registration, `push_targets_for` / `prune_tokens`, and a `deliver_to_user` helper for `Push.deliver` (migration uses `begin`/`rescue` if the collection already exists). `soli generate client <android|ios|linux|windows>` emits parameterized WebView shells (Android no-Gradle or `--fcm` Gradle+Firebase; iOS with APNs token POST; Linux GTK/WebKitGTK; Windows WebView2). `soli generate app_links` writes well-known proof routes. `soli generate offline` adds `/sync/push` + `/sync/pull` and `public/js/soli_outbox.js`. Desktop artifacts accept `--open` / scheme URLs and redirect after the launch-token gate; outer `__soli_update__` is stashed so `Updater.*` works in-app. Camera `scan=` auto-loads `/__soli/barcode-decoder.js` on WebKit. Bridge helpers `startBackgroundLocation` / `purchase` reject with `NotSupportedError` unless a shell opts in — see [Platform limits](/docs/native/platform-limits). Docs: `/docs/native/devices`, `/clients`, `/offline`.
* **feat(pdf):** **SIREN / SIRET on both parties of a Factur-X invoice.** The typed invoice carried a VAT id and nothing else, so `Invoice::to_cii_xml` emitted no `SpecifiedLegalOrganization` at all — valid EN 16931 (`BR-CO-26` accepts a VAT number alone) but not a French invoice, whose statutory mentions include both parties' SIREN and whose routing keys off the SIRET. `seller.legal_id` / `buyer.legal_id` now fill BT-30 / BT-47. **The ISO 6523 scheme is inferred** from the digit count — 9 digits is a SIREN (`schemeID="0002"`), 14 is a SIRET (`0009`) — and whitespace is stripped, so the readable `"512 345 679 00017"` travels as the 14 bare digits a directory expects; `legal_id_scheme` overrides for any other ICD code. A non-numeric registration with no explicit scheme (a Dutch `"KvK 34567890"`) is emitted as a bare `<ram:ID>` rather than being tagged with a wrong French one. The element lands between `Name` and `DefinedTradeContact`, which the CII sequence fixes — a correct value in the wrong slot still fails schema validation. Both identifiers also reach the template as `company.registration` / `customer.registration` (alongside `company.vat_number` / `customer.vat_number`), so the visual PDF can print what the XML carries. The `invoice_compliant` and `credit_note` samples are now French invoices demonstrating both schemes. See [PDF & Factur-X](/docs/builtins/pdf#section-siren-siret).
* **feat(native):** **motion sensors — gyroscope, accelerometer and device orientation.** `window.soli.sensors.gyroscope(cb)` / `.accelerometer(cb)` / `.orientation(cb)` ride the standard web `DeviceMotionEvent` / `DeviceOrientationEvent`, which fire in mobile Safari, Chrome and both WebView shells — so this is a thin client helper, not a native bridge. Each returns a `Promise<{ stop() }>` and delivers a normalized reading (gyroscope in rad/s, accelerometer in m/s² with gravity removed by default, orientation in degrees). What it adds over a raw `addEventListener` is what hand-written code forgets: the **iOS 13+ permission gesture** (`requestPermission()` must be called from a click, or it rejects), **stopping the listener** on instant navigation (a `soli:visit` body swap would otherwise leave a sensor running against a page that is gone), one **shared listener per event** fanned out to every subscriber, and per-subscription throttling (`{frequency}` / `{interval}`). A page opts in by referencing `soli.sensors` inline, or by calling the `motion_sensors()` view helper when the code lives in an external `.js` file — a page that does neither downloads nothing. Desktops report `supported()` true but never emit, since there is no gyroscope; the honest signal is a callback that never arrives. See [Motion Sensors](/docs/native/motion-sensors).
* **feat(build):** **signed over-the-air auto-update for standalone & desktop artifacts.** A `soli build --standalone` / `soli desktop build` artifact was a frozen binary — shipping a fix meant asking every user to re-download it by hand. Built with `--update-url <base>` (and `--update-key <p256-pubkey>`), an artifact now embeds an update descriptor and understands `--check-update` / `--update`: it fetches `<base>/<channel>/latest.json`, verifies the manifest's P-256 ECDSA signature against the embedded key, downloads the artifact for its own platform, verifies its sha256, and atomically self-replaces (staged then renamed, so a failed or tampered download never touches the installed binary). An auto-updater is an RCE channel, so the manifest **must** be signed — an unsigned update is accepted only when no key was embedded, and then only with a loud warning; downgrades are refused. Two developer commands: `soli update-keygen` (generate a P-256 keypair) and `soli sign-update <latest.json> --key <pem>` (sign a manifest in place); building with `--update-url` also drops a `<output>.update.json` stub pre-filled with this build's version/sha256/size to merge into the manifest. A new `Updater` builtin — `Updater.version()` / `Updater.check()` / `Updater.apply()` — drives the same flow from Soli so a page can offer an in-app "update available, restart to apply" affordance; every method degrades gracefully (`configured: false`) outside a built artifact. Mobile shells are deliberately excluded — they're WebViews onto a remote URL, so content updates on deploy and the store updates the shell. See [Auto-Update (OTA)](/docs/development-tools/auto-update).


## [1.24.0] - 2026-07-23

### Added

* **feat(clients):** **a native iOS shell** (`clients/ios`, UIKit + WKWebView). Like the Android shell it is a WebView onto the remote deployment — iOS forbids a bundled server — and it carries the full native bridge: notifications (+ APNs registration), camera and microphone permission, geolocation (via app-level Core Location, which is what enables `navigator.geolocation` in an iOS WKWebView), haptics, share sheet, an arbitrary icon **badge** and **Core NFC** (two things the macOS shell cannot do), biometrics, keep-awake, print, clipboard, and deep links (custom `bonfire://` scheme plus Universal Links). The bridge is ported almost verbatim from the macOS shell, since WebKit / UserNotifications / LocalAuthentication are shared. Ships as an Xcode-ready project (an XcodeGen `project.yml` or a manual path); custom-scheme deep links and every device capability build with free provisioning, while push, Universal Links and NFC need the three entitlements and so a paid account. The capability table's iOS column now reflects the shell rather than Safari/PWA. **Unverified** — written without a Mac to compile against.
* **feat(native):** **badge counts now work on every shell, and ride the push to a closed app.** iOS, macOS and the web have a first-class arbitrary icon-badge counter; Android does not — a badge there is a byproduct of a notification. `Fcm.send` now maps a `badge` in the payload to the Android notification's `notification_count` (APNs already set `aps.badge`), so `Push.deliver` carries the count to a closed app on either platform without the caller branching. On the open Android app, `badge(n)` posts a silent, minimum-importance carrier notification with `setNumber(n)` — a number on launchers that render one, a dot on stock/Pixel — cleared at `badge(0)`; the honest ceiling of what Android offers, since there is no arbitrary icon counter. The capability row moves from "no OS API" to "via notification". See [Device Capabilities](/docs/native/device#badges).
* **feat(crypto):** **`X509.spki_pin(cert)` — the public-key pin for TLS certificate pinning.** Returns `base64(SHA-256(SubjectPublicKeyInfo))` as `"sha256/<base64>"`, the form an Android Network Security Config `<pin-set>` or any HPKP-style pinner expects. It pins the key rather than the certificate, so the pin survives a renewal that reuses the key — pinning the certificate itself breaks a client on every ~90-day rotation. Tested against the canonical `openssl` pipeline and proven stable across a same-key renewal. The docs are blunt that pinning is a footgun (a wrong pin bricks the installed app; ship a backup; browsers removed HPKP for the same reason), and soli deliberately does not wire pinning into the shells by default.
* **feat(native):** **deep links open your app's URLs in the app.** Two halves that must agree: the shell declares which URLs it handles, and the host serves a file proving the app is allowed to — get either wrong and the link silently falls back to the browser. `AppLinks.android(package, fingerprints)` and `AppLinks.apple(app_id, paths)` generate the two proof files (`assetlinks.json`, `apple-app-site-association`), which is the part that is error-prone by hand: fingerprints are normalized to the colon-separated upper-case form Google matches (a wrong-length one is rejected rather than never matching), and the Apple document carries both the modern `components` and legacy `paths` so one file serves every OS version. The Android shell now routes an incoming link into the web view on both a cold launch and a warm `onNewIntent` — previously the intent filter existed but a deep link only landed the user on the home page — and adds a `bonfire://` custom scheme alongside the verified https link. The macOS shell registers a URL scheme and handles the Apple event, queuing a link that arrives before the web view is ready. Universal (https) links on Apple still need the associated-domains entitlement and so a paid account; the custom scheme works with ad-hoc signing. See [Deep Links](/docs/native/deep-links).
* **feat(push):** **`Push.deliver` — one call that reaches a user however you can.** There are four transports — the native bridge for an app that is open, and Web Push / APNs / FCM for one that is closed — and which applies depends only on where the user is. `Push.deliver(channel, payload, options)` is the cascade over all four: it tries the bridge first (free, no push service), then falls through to push for whoever it did not reach, routing each target by platform. The framework cannot own the device list, so targets are passed in; what it owns is the ordering, the routing, and dead-token detection. The result's `prune` array lists tokens the service reported gone (`410` / `UNREGISTERED`) so the app can delete them — a wrong-gateway `400 BadDeviceToken` is deliberately excluded, since its token is usually fine. `"always": true` sends every transport regardless, for to-all announcements. The four low-level senders remain available directly. See [Notifications](/docs/native/notifications).
* **feat(native):** **the rest of the native bridge — vibration, share sheet, badge, keep-awake, biometrics, NFC and printing.** Each degrades to the web API where the host has one, so a page calls `soli.nativeBridge.share(...)` and gets `navigator.share` in a browser and `NSSharingServicePicker` / `ACTION_SEND` in a shell. Adding them needed a **request/response protocol**: `notify` was fire-and-forget, but biometrics, NFC and sharing all have to answer, so a call now carries an id and the shell replies through `window.__soliNativeReply`. Every pending call rejects on a timeout — a promise that hangs forever is worse than one a page can handle — and every shell path answers exactly once, including cancellation. Four limits are reported honestly rather than faked: Android has no supported launcher-badge API (OEM extensions, not platform), Macs have no NFC radio, macOS haptics are trackpad-only, and biometrics confirm the person holding the device rather than authenticating anything to a server (that is WebAuthn's job). Android NFC uses reader mode rather than foreground dispatch, so scanning does not bounce through an intent or trigger the system's discovery sound. See [Device Capabilities](/docs/native/device).
* **feat(geo):** **`Geo.*` — the arithmetic behind "find what is near me".** `Geo.distance` (haversine metres), `Geo.bearing`, `Geo.bounding_box`, `Geo.geohash` and `Geo.geohash_decode`. The bounding box is the point of it: distance is a trigonometric function of every row, so a query that filters by it cannot use an index — the usual shape is a cheap indexed box pre-filter followed by exact distances over what survives, and the box's longitude span accounts for cos(latitude), which is the classic bug that makes northern boxes far too narrow. Geohash prefixes mean proximity, so a `LIKE 'u09tv%'` finds a neighbourhood on an ordinary text index; the docs are explicit about the cell-edge caveat. Coordinates off the globe raise rather than wrap, because a wrapped coordinate produces an answer that looks plausible and is wrong. Verified against the canonical geohash reference value and known city pairs. See [Geolocation](/docs/native/geolocation).
* **feat(camera):** **`camera_preview(...)` — a camera in a view, with barcode scanning.** Showing a camera has always been six lines of `getUserMedia`; this exists for what those six lines leave out. Chiefly: **the tracks are stopped when the element leaves the DOM.** Instant navigation swaps the body without a page unload, so a hand-rolled preview keeps its stream and the camera indicator stays lit after the user has navigated on. Also `playsinline muted` (without which iOS goes fullscreen and refuses autoplay), `ideal` rather than `exact` constraints, un-mirrored front-camera snapshots, a `fallback` selector revealed on failure, and `soli:camera-ready` / `soli:camera-error` events carrying the real `NotAllowedError` / `NotFoundError` / `NotReadableError` name. Add `"scan": "qr_code"` and the element emits `soli:scan` with the decoded value: native `BarcodeDetector` where the host has one (Chromium — the Android shell, and Chrome on Windows/Linux), and a page-supplied `soli.camera.decoder` where it does not (WebKit), because bundling a ~200 KB WASM reader into every soli binary to serve the pages that scan would be the wrong trade. The loop is throttled to 100 ms rather than running at frame rate — a code held in frame is still there 100 ms later, and 60 decodes a second only drains the battery. The script is injected only into pages carrying such an element. See [Scanning](/docs/native/scanning).
* **feat(fcm):** **`Fcm.send` — push to a closed Android app.** The counterpart to `Apns.send`, and the only way to reach an Android app that is not running: the OS kills long-lived connections within minutes of the screen going off. Uses FCM's HTTP v1 API, which unlike APNs wants an OAuth2 access token — so it signs a service-account assertion (RS256, via the `jsonwebtoken` already in-tree), exchanges it at Google's token endpoint, and caches the result for 55 minutes so the exchange happens once rather than per message. `title`/`body` become a `notification` (what makes Android display it with the app closed) and everything else becomes `data`, **stringified**, because FCM rejects non-string data values outright and a `{"count": 3}` would otherwise fail the whole send. Messages go at `high` priority, since Doze defers normal ones. Returns `{"status", "reason"}` — `404 UNREGISTERED` is a token to prune, not an exception. The legacy server-key API is deliberately not supported: Google shut it down in 2024. See [Native Bridge](/docs/development-tools/native-bridge#reaching-a-closed-android-app-fcm).
* **feat(apns):** **`Apns.send` — push to a closed Apple app.** Token-based (JWT) APNs: one `.p8` key per team rather than per-app certificates that expire annually. `Apns.send(device_token, payload, options)` wraps `title`/`body`/`badge`/`sound` into the `aps` envelope (or passes an explicit one through), carries custom keys alongside for the app to read on tap, and returns `{"status", "reason"}` rather than raising — a dead token is an outcome to handle, not an exception. Provider tokens are cached and reused for 45 minutes because Apple rate-limits *minting* to once per 20 minutes. The ES256 signing reuses the P-256 machinery VAPID already had; `reqwest`'s `http2` feature is now enabled, since APNs refuses HTTP/1.1 outright. Pairs with the native bridge: `Native.notify` for a client that is looking, `Apns.send` for one that is not. Receiving still requires the `aps-environment` entitlement and therefore a paid Apple Developer account — documented rather than discovered. See [Native Bridge](/docs/development-tools/native-bridge#reaching-a-closed-app-apns).
* **feat(native):** **`Native.*` — a bridge to the shell an app is being viewed in.** An app packaged with `soli desktop build`, or wrapped in a WebView on a phone, renders inside an embedded web view — and neither `WKWebView` nor Android's `WebView` implements the Push API or the Notifications API, so an app shipping web push reaches browsers and installed PWAs and silently reaches nothing at all inside its own shell. `Native.notify(channel, payload)` raises a real OS notification there, `Native.subscribers(channel)` asks whether anyone is looking, and `native_channel(channel)` in a view turns the whole thing on. Delivery rides the existing SSE fan-out, so there is no push service, no certificates and no VAPID keys — and equally, it only reaches a client with the app open. `notify` returns how many it reached, so real push stays one line away (`WebPush... if reached == 0`). Channels travel as HMAC-signed tokens keyed by HKDF from `SOLI_SESSION_SECRET` — subscribing is a browser `GET`, so an unsigned channel would let anyone listen to anyone. Inside a shell the client script also replaces `window.Notification`, so page code that already calls `new Notification(...)` keeps working where that global otherwise does not exist. See [Native Bridge](/docs/development-tools/native-bridge).


## [1.23.4] - 2026-07-21

### Added

* **feat(bundle):** bundles now carry **static assets**, not just code. `BUNDLE_EXTENSIONS` covered `.css` and `.js` but no images, icons, fonts, `.html` or `.webmanifest` — so every `soli build --standalone` and `soli desktop build` artifact shipped an app whose logo, favicon, offline page and web-app manifest were all `404`, silently, while working perfectly from disk in dev. Images (`png/jpg/gif/svg/webp/avif/ico/bmp`), fonts (`woff/woff2/ttf/otf/eot`), media (`mp3/wav/ogg/oga/m4a/mp4/webm/vtt`) and documents (`html/htm/txt/xml/webmanifest/mjs/map`) are bundled and served as bytes. `VALID_STATIC_EXTENSIONS` and the MIME table gained the matching entries — including `application/manifest+json` for `.webmanifest`, without which a browser ignores the manifest and the app is silently not installable.
* **feat(desktop):** `SOLI_DESKTOP_NO_WINDOW=1` stops a desktop artifact from opening a browser window, for embedding it in a native shell (Cocoa/WebView, Electron-style container) that provides the window itself. Without it the wrapper gets two windows: its own, and the browser the artifact launches. The launch URL is still printed on its own indented line, which is how the wrapper learns the port and the single-use token — pointing a web view at `http://127.0.0.1:<port>/` directly is refused by the gate. See [Desktop Applications](/docs/development-tools/desktop#embedding).

### Fixed

* **fix(serve):** **every page of a macOS standalone or desktop app rendered as the literal text `null`.** The bundle extracts under `std::env::temp_dir()` and the virtual filesystem is rooted at that path, but `serve_folder` canonicalizes the folder it is handed. On macOS the temp dir is `/var/folders/...`, a symlink to `/private/var/folders/...`, so the two disagreed about where the app lived. Nothing errored: view helpers never loaded, no template was found for any action, and the auto-render path fell through to raw value serialization — turning an action's nil return into a four-byte `null` body with no `content-type`. The extraction directory is now canonicalized before anything records it, on both the standalone and desktop paths, with a regression test that fails if it ever returns a symlinked path. Reproducible on any platform by pointing `TMPDIR` at a symlink.
* **fix(standalone):** **signed macOS apps dropped the user into the soli REPL.** `codesign` starts the code signature blob on a 16-byte boundary, so when the appended region did not end on one it inserted up to 15 bytes of padding — and the boot-side lookup anchored the footer at the signature offset *exactly*, read that padding instead of the magic, concluded the executable carried no payload, and fell through to the normal CLI. With no arguments that is the REPL, so a double-clicked app printed `>>>` instead of starting. Every `--standalone` and `soli desktop build` darwin artifact was affected whenever its size happened to be misaligned, which is most of them. The appended region is now padded to the boundary at build time, and the loader additionally walks back over an alignment gap so artifacts built before this fix boot too — validating the bundle magic before accepting a candidate, since a wrong hit there is a hard failure rather than a fall-through.


## [1.23.1] - 2026-07-19

### Added

* **feat(scaffold):** `soli generate oidc_provider` — scaffolds a working **OpenID Connect provider** (Authorization Code + PKCE, the OAuth 2.1 profile): discovery and JWKS documents, authorize/consent, token, userinfo, revoke and RP-initiated logout, five models (client, authorization code, refresh token, consent, revocation), a consent screen, and the index migration. Requires `soli generate auth` and fails fast without it rather than emitting controllers that reference an undefined `User`. Security specifics that a looser implementation would get wrong: exact byte-for-byte `redirect_uri` matching; unknown-client and unregistered-`redirect_uri` errors render a 400 page instead of redirecting (RFC 6749 §4.1.2.1 — redirecting there *is* the open redirect); PKCE `S256` only, `plain` rejected for every client type; codes single-use via one atomic `FILTER … UPDATE … RETURN NEW` statement and bound to client/redirect_uri/challenge; a replayed code revokes the whole token family; refresh rotation with reuse detection; 401 + `WWW-Authenticate` for failed Basic vs. 400 for failed post auth (§5.2); `Cache-Control: no-store` on every token response. Verified end to end against a live server, including an independent RSA signature check of the `id_token` using only the published JWKS. See [OpenID Connect Provider](/docs/security/oidc-provider).
* **feat(rsa):** `RsaKey.public_from_pem(pem)` — parses a bare RSA public key (SPKI `-----BEGIN PUBLIC KEY-----` or PKCS#1) into `{algorithm, n, e, bits}`. `X509.public_key` only covered certificates and `private_from_pem` only private keys, leaving no way to build a JWKS entry from a public PEM — the shape a rotation's outgoing key takes.

### Fixed

* **fix(scaffold):** **generated migrations were silently skipped.** `Migration::from_path` splits on the first `_` and requires a numeric version, but `soli generate auth` emitted `<ts>create_users_<ts>.sl` — so `parts[0]` was `"1784481604create"` and every migration it generated was discarded without a message. `soli db:migrate up` reported "No pending migrations" and exited 0. SoliDB auto-creates a collection on first model access, so the app still ran; it just ran with no indexes and **no unique constraints**, which for the auth scaffold means `users.email` was never enforced unique. Both generators now emit `<ts>_<name>.sl`, matching `soli db:migrate generate`, and a test asserts the convention.

## [1.23.0] - 2026-07-19

### Added

* **feat(test):** **browser testing is built in.** `soli test --browser` drives a real headless Chrome over the Chrome DevTools protocol, spoken from the `soli` binary itself — no Node, no npm, no Playwright, nothing installed into the project. New helpers sit alongside the existing HTTP ones: `visit`, `click`, `click_link`, `click_button`, `fill_in`, `select_option`, `check`/`uncheck`, `choose`, `press` (including chords like `Alt+d`), `evaluate`, `screenshot`, `wait_for`, `wait_for_text`, `page_text`/`page_html`/`page_path`/`page_url`/`page_title`/`page_errors`, plus assertions `assert_text`, `assert_no_text`, `assert_selector`, `assert_no_selector`, `assert_page_path`, `assert_no_page_errors`. Positive assertions wait for their condition (default 10s, `{"timeout": n}` to override); negative ones check immediately, since waiting for something to stay absent only slows passing tests. Clicks are real `Input.dispatchMouseEvent` events at the element's measured position rather than `element.click()`, so an element covered by an overlay fails as it would for a user; fields resolve by CSS selector, `<label>` text, `name` or `placeholder`. `evaluate` preserves JavaScript's types — deliberately not the shared `json_to_value`, which promotes numeric-looking strings to `Decimal` and would turn `textContent` of `"0"` into `0`. See [Browser Testing](/docs/testing-browser).
* **feat(test):** browser specs are **opt-in**: a spec is a browser spec when a `browser` directory appears anywhere in its path, and plain `soli test` sets them aside and reports the count. A project with no browser installed still runs green, and the default suite keeps its dependency-free, millisecond-per-test character. `--browser` verifies a browser exists up front and fails with the list of names it looked for, rather than timing out inside a worker on the first `visit()`; `SOLI_CHROME_PATH` overrides discovery and `--headed` shows the window. One browser per test worker, launched lazily on first use and reused across tests, so `--jobs 3` means three browsers and a suite of thirty specs takes seconds.
* **feat(test):** the browser shares the request helpers' cookie jar **in both directions**, so an existing `login()` / `as_user(id, opts)` in `before_each` carries into `visit()`, and a sign-in performed by clicking a form is visible to a later `get()` and to `signed_in()`. Between tests, captured page errors and `sessionStorage`/`localStorage` are cleared — the runner has no per-test teardown hook, and without this a panel one test collapsed stayed collapsed for the rest of the suite, making results depend on test order. Cookies are deliberately left alone, matching the established convention that request specs manage sign-in explicitly.
* **test(browser):** Soli's own frontend is exercised in a real browser in CI for the first time — roughly 2,500 lines of shipped JavaScript that had either no test at all or a JSDOM unit test unable to open a websocket or lay anything out. New specs in `tests-e2e/browser/` cover instant-nav (`nav.js`: link interception, body swap, head/title merge, pushState/popstate, `soli:visit`/`soli:load`, `data-no-nav` opt-out, scripts re-running after a swap), the LiveView client (`client.js`: websocket connect, event round trip, DOM morph verified by node identity, `soli-ignore` islands surviving a patch, focus retention across a patch) and the dev bar (panel toggles, `Alt+D`, body padding, state surviving navigation). A new `browser` CI job runs them against a pinned Chrome, in parallel with `test`/`clippy`/`fmt` so it stays off the critical path.

* **feat(desktop):** `soli desktop build` packages an app as a single executable carrying its own database — the user double-clicks it and the app opens in their browser. The artifact bundles the soli runtime, the application (always encrypted), a compressed database binary, and optional read-only reference data. At launch it takes a single-instance lock, fetches its key (no key, no app — the key is never written to disk, which is what makes revocation possible), starts a private database on an ephemeral loopback port with per-install credentials, imports changed reference data, gates the port behind a single-use launch token, and opens a browser. On stop it removes the decrypted tree and closes the database cleanly. Cross-builds to linux-amd64/arm64, darwin-amd64/arm64 and windows-amd64 from one machine by downloading and checksum-verifying published runtimes. Seed collections must be named `ref_*`, enforced at build time, because importing replaces a collection wholesale and the prefix is what stops it overwriting one of your models. See [Desktop Applications](/docs/development-tools/desktop) — including an explicit account of what this does *not* protect against, since the key reaches the process environment and a machine's owner can read it.
* **feat(windows):** the crate compiles for Windows. `platform::{lock,job,dirs,process}` provide `LockFileEx` single-instance locking, job objects with `KILL_ON_JOB_CLOSE` for orphan prevention, owner-only directories via a protected DACL, and cross-platform process liveness. A `windows-check` CI job keeps it from silently re-breaking, and `windows-amd64` joins the release matrix. That job now *executes* the test suite on Windows rather than only compiling it, which is what surfaced the bundle-key and rooted-path bugs listed under Fixed.
* **feat(build):** `darwin-amd64` (Intel Mac) is published. `soli update` already resolved x86_64 macOS hosts to that artifact name, so those users were getting a 404.
* **feat(pdf):** new `at` element — renders `content` at absolute page coordinates (`x`/`y` from the sheet's top-left, not the content box) and restores the flow cursor, so placed items are independent of each other and of the surrounding flow. Optional `width` sets the wrap width via the same `inset_right` mechanism `box` uses; coordinates are clamped to the page so a stale canvas value lands at the edge rather than off-sheet. This is the primitive free positioning needs in a language that is otherwise a strict top-to-bottom flow.
* **build(dev):** `scripts/cdp_drive.mjs` — drives a headless Chromium over the DevTools protocol in real time (goto / waitFor / eval / synthetic mouse paths / key / screenshot), so pages that depend on Web Workers can be verified at all. `chromium --screenshot --virtual-time-budget` cannot: virtual time does not advance a worker's event loop, so any PDF.js page is captured mid-flight and a working page is indistinguishable from a hung one.
* **feat(docs):** PDF Studio gains **Flow** and **Free** layout modes, derived from the document rather than stored as editor state. Switching converts: Free wraps each element in an `at` at its measured position (visually a no-op); Flow unwraps them ordered by y. Drawing follows the mode. A mixed document automatically gets a leading `move` sized so the flow begins below the lowest placed element — `at` restores the cursor, so without it flow content renders underneath. Header/footer bands and their elements are now reported by `pdf_layout_map` too, so band guides use the engine's measured height instead of a 26pt guess and band elements are clickable.
* **feat(docs):** PDF Studio: a drag now means what the element allows. Pinned (`at`) elements move freely; flowing elements reorder in document order with a drop indicator, because they have no coordinates to change — free-dragging them silently tore them out of the flow. Flowing elements gain a `Space before` control backed by the adjacent `move` element; `Pin to page` (explicit) and `Return to flow` remain for genuine overlays. Data-bound elements (`repeat`, bound tables) render as one magenta-dashed element labelled with their array rather than N draggable copies. Header and footer bands are always drawn, hatched when empty, and the sample picker moved from the top bar into the JSON drawer.
* **feat(pdf):** new `pdf_layout_map(template, data, options?)` builtin — lays out a template and reports **where every element landed** (`path`, `kind`, `page`, `x`, `y`, `w`, `h`) without producing a PDF. A visual editor cannot compute this itself: a flowing element's position depends on everything before it, so only the layout engine knows it. `path` addresses the element in the template (`content.3.content.0`), and a `repeat` yields one box per drawn item sharing a single path. Backed by `soli_pdf::layout_boxes` and `LaidOutDoc::element_boxes`; header/footer bands are excluded because they repeat per page from their own cursor.
* **feat(docs):** PDF Studio gains a configurable point grid, edge-aware snapping (page margins, header band, page centre and other elements' edges/centres, with live cyan/amber guides naming the catch; `Alt` bypasses), an align-to-content-border row, a chrome-free `Preview` mode and a `PDF` button that opens the rendered document in a new tab.
* **feat(docs):** new "PDF Studio" page (`/docs/builtins/pdf-studio`) — a full-screen canvas editor rendered without the docs chrome. Draw and move elements on the page with the real rendered PDF painted underneath the handles via PDF.js at matching scale; point rulers, grid snapping, margin and band guides, tool rail, live-coordinate inspector, layers, and Body/Header/Footer zones. Every placed item compiles to an `at` element, so the output is ordinary template JSON.
* **feat(pdf):** new `box` element — a container that lays out `content` inset by `padding`, then paints `fill`/`border` at the size the content measured and advances the cursor below itself. Replaces the `rect` + hand-computed height + compensating `move` idiom for panels, callouts, totals blocks and signature areas. Options: `padding` (number or per-side `{top,right,bottom,left}`), `width` (defaults to the remaining region width), `fill`, `border`, `borderWidth`, `radius`, `dash`, `gap`. Boxes nest, and children wrap at the box's padded inner edge (via a new `Engine::inset_right`). The decoration is spliced into the op stream at the index recorded before the children ran, so it paints *behind* them in one pass; a box whose content spans a page break omits the decoration and emits an `ElementSkipped` warning rather than painting it on a flushed page.
* **feat(pdf):** six new invoice/quote templates in `www/public/pdf-samples/`, each with a distinct structure rather than a recoloured header — `invoice_compliant` (per-rate VAT breakdown, both parties' VAT numbers, statutory late-payment terms, EPC payment QR), `invoice_minimal` (monochrome, amount due at 46pt, `margins: 64` → 467pt content width), `invoice_subscription` (billing-period band, prorated seat lines, metered usage, spend `donut`), `credit_note` (negative amounts, reference-to-original band, credits `invoice_compliant`), `quote_sections` (per-section subtotals + dashed signature acceptance box) and `quote_options` (base scope + tickable options with a dual total). Each ships a `.template.json` + `.data.json` pair; `invoice_compliant` additionally ships `.invoice.json` (typed invoice) and `.facturx.xml` (CII XML) so both Factur-X routes are runnable.
* **feat(docs):** new "Layout editor" page (`/docs/builtins/pdf-editor`) — a structural template editor: document tree, per-element property panel, and a live render from the real engine via the existing playground endpoint. The template JSON *is* the editor state, so export is lossless (verified byte-identical rendering across all 12 samples) and hand-written files round-trip; tables whose structure the panel cannot represent are shown read-only rather than rewritten.
* **docs(pdf):** new "Invoice & Quote Templates" page (`/docs/builtins/pdf-templates`, `www/docs/builtins/pdf-templates.md`) — gallery of the eight billing samples with a trait matrix for choosing by requirement, click-to-zoom previews, per-template playground links, and the two Factur-X routes contrasted.
* **feat(docs):** the PDF playground accepts `?sample=<name>`, which takes precedence over the remembered `pg-sample`; the six new templates are registered as presets (reachable by URL, no buttons added to the already-full bar).
* **build(pdf):** `scripts/gen_pdf_previews.sh` renders the PDF samples and rasterises page 1 of each to `www/public/images/docs/pdf/` at 150 DPI (A4 → 1240×1755), replacing an ad-hoc manual step. Accepts sample names to regenerate a subset; builds `render_pdf` on demand (`pdf/` is its own cargo workspace).

* **feat(pdf):** table cells accept **`rowspan`** — a cell claims its column slots in the rows beneath, which then supply correspondingly fewer cells, and it is drawn once at its own row tall enough to cover every row it spans. Composes with `colspan` (a span stops at the first slot already claimed from above, so the two can never overlap), and a spanning cell contributes only its per-row share to the first row's height rather than inflating it. Resolved before drawing via a new `plan_row_spans`, so the combined height is known up front; applies to a table's literal `rows` (a data-bound table repeats one template row, so there is nothing to span).

* **feat(jwt):** `jwt_sign` gains header and registered-claim control — `kid` and `typ` in the JWT header, plus `exp` (absolute Unix timestamp), `nbf`, `aud` (string *or* array per RFC 7519 §4.1.3), `iss` and `jti`. `kid` is the load-bearing one: it is how a relying party selects the right key out of a JWKS, so without it a JWKS can hold exactly one key and rotation is inexpressible. Supplying both `exp` and `expires_in` now raises rather than silently picking one — they are different units (absolute vs. relative), and guessing would mint a token expiring at a time the caller never meant. Internally the signing path builds a `serde_json::Map` instead of the typed `Claims` struct, so `aud` can serialize as either shape; `Claims` is retained for decoding, leaving `jwt_verify`'s return shape byte-identical.
* **feat(jwt):** `jwt_verify` gains `audience`, `issuer`, `subject` and `leeway` options. Setting `audience` or `issuer` also inserts that claim into `required_spec_claims`, so a token *lacking* the claim is rejected rather than passing a check the caller believed was enforced. Note that several expected audiences means subset semantics (the token must carry all of them), not "any of".
* **feat(crypto):** `Crypto.random_hex(n)`, `Crypto.random_bytes(n)` and `Crypto.random_token(n = 32)` — CSPRNG output from `OsRng`. Soli previously had **no** secure-random primitive at all: only `uuid_v4()`/`nanoid()` and `Math.random` (a non-cryptographic PRNG). `random_token` returns unpadded base64url, the shape OAuth `state`, PKCE verifiers, authorization codes and refresh tokens all want. `n` is a **byte** count throughout — `random_hex(32)` yields 64 characters, matching `openssl rand -hex 32`, which `jwt_sign`'s own secret-length error already tells users to run. Bounded to `1..=1024`: `random_hex(0)` returning `""` and being used as a token is indistinguishable from success at the call site.
* **feat(base64):** `Base64.urlsafe_encode` / `Base64.urlsafe_decode` — RFC 4648 §5 alphabet, never padded on encode, padding-tolerant on decode. JWS §2, JWK, RFC 7638 thumbprints and PKCE all mandate the unpadded URL-safe form, so there is deliberately no padding option. The byte-coercion and byte-to-value conversion shared with `encode`/`decode` were extracted into helpers rather than duplicated.
* **feat(types):** `Base64`, `Hex`, `RsaKey`, `X509` and `jwt_sign`/`jwt_verify`/`jwt_decode_unsafe` are registered in the type environment. All worked at runtime but were unknown to the checker, so `soli -e 'print(Base64.encode("abc"))'` failed with `Undefined variable 'Base64'` before executing — any script or library doing encoding or JWT work was locked out of `soli check` entirely.

### Fixed

* **fix(jwt):** **`jwt_verify` rejected every token carrying an `aud` claim.** `jsonwebtoken` defaults `Validation.validate_aud` to `true`, and neither `jwt_verify` nor `jwt_decode_unsafe` ever called `set_audience`, so any aud-bearing token failed with `InvalidAudience` — meaning no OIDC `id_token` from Google, Auth0, Okta or anywhere else could be verified, or even *inspected*. `aud` is now validated only when the caller supplies `audience` (matching how `iss` has always behaved, and every mainstream JWT library); `jwt_decode_unsafe` disables the check unconditionally, since validating audience inside an explicitly-unverified inspection helper is meaningless.
* **fix(parser):** **a statement that begins with `[` is no longer swallowed as an index into the previous line.** The postfix `[` binds at call precedence with no line check, so any expression ending a line and followed by a line starting with an array literal parsed as an index: `for n in [1, 2]` with a body opening `[10, 20].each(...)` became `[1, 2][10, 20]` and failed with "Unexpected token ',', expected ]". The same held for brace-less `if`/`while` heads and for two ordinary adjacent statements — and it was not always an error: `while i < 1` followed by `[7].each(...)` silently parsed as `i < 1[7]`. A `[` opening a new line is now always an array literal, matching Ruby; multi-line `.method` chains are unaffected because they lead with `.`, and same-line indexing is untouched. This surfaced through `soli fmt`, which rewrites braced loops to the `end` form and so turned a valid file into an unparseable one.
* **fix(bundle):** **bundle entry keys are `/`-separated on every platform.** `collect_entries` keyed entries with the native separator, so a bundle built on Windows stored `app\controllers\home_controller.sl` while every consumer looks up `/`. `soli build --protect` still *succeeded* and emitted a structurally valid `.soli` whose controller registry was empty and whose middleware directives were missing, because the `app/controllers/` and `engines/` prefix checks never matched — the failure only appeared later at serve time, and the artifact was equally broken when served on Linux. Reuses the existing `to_vfs_key` helper so bundle keys and `walk_dir` keys are produced by one function.
* **fix(security):** **rooted-but-prefixless paths are treated as absolute on Windows.** `Path::new("/etc/passwd").is_absolute()` is `false` on Windows (no drive prefix), while `join` on such a path *discards* the base — so guards written as `if is_absolute() { reject } else { base.join(p) }` took the "safe" branch and then escaped anyway. Three sites now test `is_absolute() || has_root()`: `DiskFS::resolve` (a `/`-leading VFS key resolved to `C:\app\...`, outside the serve root, instead of being grafted under it), the SEC-076 absolute-import gate in the module resolver, and the `File.*` jail helper. The resolver and file jails have canonicalising `starts_with` backstops that already caught the escape, so those are defense-in-depth; `DiskFS::resolve` had no backstop. Grafting now also strips a leading drive prefix, so an absolute sibling path can no longer replace the root through `join`.
* **fix(pdf):** a `repeat` or data-bound `table`/`chart` nested inside another `repeat` now resolves its `data` path scope-first (current item, then document root), matching how `${field}` already resolved. Previously `Layout::repeat`/`table`/`chart_series` read the array off `DataDocument` — always the root — so `"data": "lines"` inside a `repeat` over `sections` bound nothing and rendered silently empty. Adds `Resolver::array`, which is now the single binding path; the four root-only call sites are gone along with three newly-dead `&DataDocument` parameters.
* **fix(vm):** **parameter default values are now evaluated in compiled mode.** `def configure(host = "localhost", port = 8080)` called as `configure()` bound `null` to both parameters under the VM while binding the declared defaults under the tree-walking interpreter. The compiler counted defaults into `proto.defaults` but never compiled the expressions (the `default_ops` field it was meant to fill was dead code), and the call path pushed `Value::Null` into every omitted slot. Because that produced no error it triggered no engine fallback either, so the divergence was silent: correct in `soli serve --dev` (interpreter), wrong in production (VM). Defaults now compile to a prologue at the top of the callee, guarded by a per-frame supplied-parameter bitmask — so an explicitly passed `null` is preserved as `null` while an omitted argument gets its default, and a later default may reference an earlier parameter (`def f(a, b = a * 2)`).
* **fix(vm):** **named arguments compile instead of demoting the handler.** A call such as `configure(port: 3000)` or `get("/", "home#index", name: "root")` failed VM compilation, which permanently demoted the entire enclosing handler — and its whole call graph — to the tree-walking interpreter for the rest of that worker's life. Labelled calls now compile to `Op::CallNamed` / `Op::NewNamed`, which carry the argument labels as a constant and bind them at call time, where the callee is known: a user function reorders them into parameter slots (filling the rest from defaults), while a native collapses them into one trailing options hash — the same two conventions the interpreter applies. Covers plain calls, `new`, pipelines and `print`; `super(...)` with labels still falls back.
* **fix(vm):** **a JIT compile failure is no longer swallowable by user `try`/`rescue`.** When compiled code called a function the VM cannot compile, the failure surfaced as a general `RuntimeError`, so a handler that wrapped the call in `try { ... } catch` caught the VM's own internal limitation as if it were an application error and returned a rescue value — skipping the VM→interpreter fallback entirely, with no demotion recorded and no log line. It is now an `EngineFallback`, which bypasses `try`/`rescue` routing by design. (Same failure shape that was previously worked around for background jobs by forcing them onto the interpreter.)
* **fix(vm):** **a `match` expression no longer corrupts the value stack.** A literal arm's `Equal` consumes the duplicated subject, but `compile_match` popped again unconditionally, leaving every literal-arm match one stack slot short. Nothing observed it until something read the stack by depth — a `catch` binding does, so `match ...` followed by `try { } catch e { }` bound `e` to a fragment of an unrelated string, and one shape indexed out of bounds and panicked the VM outright (in a release build, `panic = "abort"` takes the worker down). The pop is now driven by an explicit contract stating whether the pattern test consumed the duplicate. Composite patterns (array, hash, and/or, destructuring), whose interleaved `Dup`/`Pop` had the same latent imbalance, now run on the tree-walking interpreter rather than miscompile.

### Documentation

* **docs(pdf):** documented that `pdf_facturx_from_invoice` builds its own render data and therefore exposes a *different* placeholder namespace than `pdf_render` (`invoice.*` / `company.*` / `customer.*` / `items[]` / `total.*` / `payment.*`), with **no** party VAT identifiers and **no** per-rate VAT breakdown array. Templates that must print those should use `pdf_facturx` with a supplied XML.
* **docs(pdf):** noted that no bundled font covers `☐`/`□`, so tick boxes must be drawn as an empty table cell with all four borders on rather than a glyph.
## [1.22.0] - 2026-07-19

### Added

* **feat(lang):** `break` is now a real loop keyword (lexer keyword + `StmtKind::Break`) — it exits the innermost enclosing `while` or `for` loop and supports postfix conditions (`break if cond` / `break unless cond`). It propagates correctly out of nested blocks, `if` branches, and `try`/`catch` (a `finally` block still runs before the loop exits). A `break` inside a lambda or function body does **not** break an outer loop — it is absorbed at the function boundary. Not supported in compiled/VM mode: a handler containing `break` falls back to the tree-walking interpreter automatically (same precedent as safe navigation `&.`), so it is fully functional but not JIT-compiled.

### Changed

* **BREAKING** **refactor(lang):** the `break()` debugger builtin — which triggers a breakpoint / the dev-page REPL — is renamed to **`debug()`**, freeing the `break` name for the new loop keyword. Behavior is unchanged: a zero-arg builtin returning a breakpoint value, active only in development and ignored in production. Any code calling `break()` must be updated to `debug()`.

### Performance

* **perf(vm/interpreter):** direct native instance-method invocation — `obj.native_method(args)` (and `super.native_method(args)`) no longer allocates a bound `NativeFunction` wrapper on every call. The receiver is prepended and the underlying native runs in place (same calling convention as before). Method-as-value access (`m = obj.method`) still binds a wrapper. Covers the bytecode VM `CallMethod` path and the tree-walker call dispatcher. Model-subclass instance natives still `EngineFallback` on the VM so lifecycle callbacks fire in the tree-walker (unchanged carve-out).
* **perf(graph):** `soli graph build` re-syncs are dramatically cheaper. Each node document now carries a deterministic content hash (`chash`) covering all stored fields and the embedding; on sync, nodes whose hash is unchanged are skipped entirely instead of being re-`UPDATE`d. Because SolidB re-serializes the whole node vector index on every write batch, a typical incremental re-sync (a handful of nodes changed) collapses from ~N/200 update batches to a couple. The vector index is also created *after* the initial bulk load rather than before, so a first build pays one index build instead of a full-index re-serialize per insert chunk. Nodes written by an older build carry no hash and take one `UPDATE` that repopulates it.

## [1.21.4] - 2026-07-16

### Added

* **feat(graph):** the multi-language extractor now builds a **C# call graph** — walking method bodies for `calls` and `new X()` `instantiates` edges, attributed to the enclosing method. Precision-first: an `instantiates` edge lands only on a project class (framework types like `new List<T>()` are skipped, never stubbed) and a `calls` edge lands only when exactly one project method carries that name (C# overloading means shared names are dropped, not mis-linked). Also generalizes the enclosing-def resolver so method-scoped edges attribute correctly for `.`/`::`-separated method names (C#, Rust), not just `#`.

### Fixed

* **fix(graph):** `soli graph build` no longer hangs indefinitely when the embedding endpoint is slow or unreachable. Embedding HTTP requests (which run by default) had no timeout, so a stalled `SOLI_EMBEDDING_URL` — a proxy that accepts the connection but never replies, a wrong URL, a stalled local model server — would block the whole build forever. Requests now time out after `SOLI_EMBEDDING_TIMEOUT_SECS` (default 60s, per request) and fail with an actionable message; `--no-embed` remains the escape hatch. The failure message also distinguishes a missing `SOLI_EMBEDDING_API_KEY` from an endpoint that errored or timed out.

## [1.21.3] - 2026-07-16

### Added

* **feat(graph):** richer Soli code-graph edges — instance method calls on locally typed variables (`let u = new User()`, typed lets, `User.find` / factories), `partial(...)` and view→partial `renders`, `redirect("/path")` as `redirects` to matching routes, and bare `super(...)` / `super.method(...)` to the parent method when present. Local type tracking is flow-aware: reassigning a tracked local to a value with no known class drops its type, and a bare `partial("form")` in a view resolves against that view's own directory first. Still precision-first: unbound receivers stay unlinked.
* **feat(graph):** `soli graph query --kind method,controller` filters seeds by node kind; results include a truncated `snippet` for agent context (the human summary shows real doc/body context, not repeated metadata); keyword fallback weights name/qualified_name over body text; neighbours are ordered with structural edges first (`routes_to`, `calls`, `renders`, `redirects`, …); a `redirect` to a path served by several verbs prefers the `GET` route.

## [1.21.2] - 2026-07-16

### Added

* **feat(graph):** `soli graph query` gains `--path <prefix>` to scope retrieval to a subtree (e.g. `--path api/` or `--path app/`), so an agent can target one side of a mono-repo without post-filtering the JSON. Only seeds whose `file` starts with the prefix are returned; neighbours are unaffected. The semantic path over-fetches then filters (so an out-of-path top ranking doesn't starve results), and the keyword fallback filters server-side in AQL.

### Fixed

* **fix(graph):** `SoliDBClient::query` now follows the SolidB cursor to completion instead of reading only the first batch. SolidB caps each cold-cache batch at 1000 rows and returns `has_more` with a continuation cursor, so any query returning >1000 rows silently truncated to 1000. In graph sync this made `soli graph build` abort on graphs larger than 1000 nodes (`Document with _key '…' already exists`) and re-embed the tail on every build; bulk mutations in the sync path also gain retry with exponential backoff so a transient timeout on one chunk no longer aborts the whole sync.

## [1.21.1] - 2026-07-16

### Fixed

* **build(ci):** vendor OpenSSL (`ssh2` `vendored-openssl`) so the release CI's linux-arm64 cross-build compiles OpenSSL from source instead of installing `libssl-dev:arm64` from the arm64 multiarch mirror (`ports.ubuntu.com`), which is unreachable from some runners. v1.21.0's arm64 binary failed to build for this reason; v1.21.1 ships the same features with a working release.

## [1.21.0] - 2026-07-16

### Security

* **security(mailer):** `Mailer` now rejects CR/LF/NUL in every header-bound field (from, subject, to, cc, reply_to, SMTP envelope from, and each recipient) — closes an email/SMTP header-injection vector where a crafted subject or address could smuggle extra headers or SMTP commands
* **security(bundle):** archive extraction (`.soli` bundles and the in-memory bundle FS) validates every entry path against Zip-Slip — a `..`, absolute, or drive-prefixed entry is rejected instead of writing outside the extraction root; offset arithmetic is overflow-checked
* **security(serve):** the dev error-page REPL (arbitrary server-side code execution) is now trusted for **loopback peers only**. Previously any private/LAN peer was trusted, so a co-resident host on a shared Wi-Fi/office/container network could scrape the token from a dev error page and run code — a LAN-wide RCE whenever `--dev` binds a non-loopback address. LAN access is now an explicit `SOLI_DEV_REPL_ALLOW_REMOTE=1` + `SOLI_DEV_REPL_SECRET` opt-in (the code now matches the already-documented loopback-only behavior)
* **security(pades):** PKCS#1 v1.5 signature verification reconstructs and compares the full encoded message instead of a suffix (`ends_with`) match — closes a Bleichenbacher-style RSA signature-forgery avenue
* **security(deflate):** `Deflate.inflate` caps decompressed output (default 64 MiB, override via `SOLI_DEFLATE_MAX_BYTES`) and fails closed — stops a decompression bomb in an unauthenticated SAML `SAMLRequest`/`SAMLResponse` from exhausting memory
* **security(deps):** the direct `quick-xml` used by the SOAP/SAML XML parsers is upgraded 0.36 → 0.41 (RUSTSEC-2026-0194 / -0195, quadratic-attribute and namespace-allocation DoS). Predefined entities and numeric character references are resolved explicitly while DTD/general entities remain rejected (XXE defense). Note: the transitive `quick-xml` reached through `calamine`/`umya-spreadsheet` (spreadsheet parsing) cannot reach ≥0.41 yet — even their latest releases pull 0.37/0.39 — so that DoS advisory stays open for untrusted `.xlsx`/`.ods` input until upstream updates
* **security(deploy):** every value interpolated into a remote `ssh` shell command is POSIX-quoted — closes command injection through branch/path/URL fields during `soli deploy`
* **security(session):** `set_cookie` validates the name and value, rejecting control characters and the `;`/`=`/whitespace delimiters that could inject extra cookie attributes
* **security(templates):** `.md` views render with link/image URLs neutralized (blocks `javascript:`/`data:` XSS), and the template `<%= %>` auto-escape now also covers non-String values (e.g. an object's `Display`), which previously rendered unescaped
* **security(jwt):** `jwt_verify` now enforces the `nbf` (not-before) claim
* **security(model):** SoliDB document keys are percent-encoded as a single URL path segment, preventing query/fragment injection into the database request URL
* **security(vm):** the unsafe VM stack ops carry `debug_assert!` bounds checks so a bytecode/stack-discipline bug surfaces as a controlled panic in debug/test builds instead of undefined behavior
* **security(rate_limit):** the process-global rate-limit bucket store is bounded (`MAX_BUCKETS` = 10,000) and auto-reclaims expired buckets, so an attacker minting a fresh key per request (rotating spoofed IPs, random tokens) can no longer grow it without limit. When the cap is reached a new key evicts an existing bucket rather than sharing one, so every live key keeps its own independent counter — limits are never mixed across keys or across different rate-limit rules (an earlier shared-overflow-bucket approach could collapse all over-cap keys into one counter and enforce the wrong limit)
* **security(regex):** `get_regex` (used by `gsub`/`match`/`scan`/`split` and model/format validation) now compiles through the same `nest_limit` + `size_limit` bounded cache as `get_safe_regex`, so a request-controlled pattern can no longer force an unbounded compile. Behavior change: a pathologically large or deeply-nested (>10) pattern that previously compiled now raises `invalid regex: …`
* **security(crypto):** `Crypto.secure_compare` (`do_secure_compare`) no longer early-returns on a length mismatch — the length difference is folded into the constant-time accumulator — removing a timing side-channel (length oracle) when comparing CSRF tokens, HMACs, and other secrets
* **security(serve):** static-file serving resolves and returns the canonical path, closing a symlink TOCTOU window where a symlink planted under `public/` between the jail check and the open could escape the public root

### Performance

* **perf(memory):** `size_of::<Value>()` dropped **64 → 24 bytes** (−62%). `Value::NativeFunction` is now a newtype over `Rc<..>` (was an inlined ~64B payload) and `Value::Method` is boxed, so the enum is pointer-sized. Every array cell, hash slot, model-row field, env binding, and VM stack slot is one `Value`, so this shrinks runtime data across the whole interpreter. Locked by a compile-time `size_of` guard.
* **perf(memory):** AST nodes shrank across the board — `Expr` **144 → 80 bytes**, `Stmt` **360 → 200 bytes**. `Span` is now `u32`×4 (32 → 16B, embedded in every node), the large `StmtKind` decl variants (`Function`/`Class`/`Enum`/`Interface`) and `ExprKind::Lambda`'s return type are boxed. Each worker holds its own copy of the parsed app, so this cuts per-worker RSS proportionally to app size (biggest for code- or i18n-heavy apps). The `SLAST` bundle format is unaffected (MessagePack encodes integers by value). Locked by compile-time `size_of` guards.
* **perf(serve):** the boot interpreter — a full interpreter built only to populate the shared route/model/controller/template registries before workers start — is now reclaimed immediately after boot instead of being parked (holding all builtins + the parsed app + the i18n locale tables) for the whole process lifetime. It also skips the test-only builtins it never used.
* **perf(serve):** background-job interpreters can skip view helpers via `SOLI_JOB_VIEW_HELPERS=0` — an app's i18n locale tables are often the largest per-interpreter cost, and most jobs never render a helper-using template, so this drops them from every job interpreter.
* **perf(serve):** the default background-job pool size (`SOLI_JOB_WORKERS`) is now **1** (was 2) — the pool is opt-in and each worker is a full interpreter copy; raise it for higher background throughput. Also documents the previously-undocumented `SOLI_WORKERS` (the primary baseline-RSS lever) and a "Keeping memory low" configuration section.
* **perf(serve):** a `multipart/form-data` request body is no longer copied into a lossy UTF-8 string alongside its raw bytes and parsed parts (a large upload was triple-buffered — bytes + string + parsed fields/files). Behavior change: `req["body"]` is now empty for multipart requests — read fields from `params`/the form hash and files from the uploads API (the raw bytes are still retained). CSRF verification and `_method` override are unaffected.
* **fix(serve):** `soli serve` now honors the `SOLI_WORKERS` environment variable. It was silently ignored by the CLI (only the `--workers` flag was read, defaulting to the CPU-core count), so operators on many-core boxes couldn't cap the worker count — and thus baseline RSS — through the environment. `--workers N` still overrides the env when given. On a 16-core box this alone takes a locale-heavy app from ~311MB (16 default workers) to ~66MB at `SOLI_WORKERS=2`.

* **perf(interpreter):** a user function's parameter list and body AST are now shared via `Rc<[Parameter]>` / `Rc<[Stmt]>` instead of owned `Vec`s. Binding `this` rebuilds a `Function` from an existing method on every model-instance / bound-method / `super` call, and that rebuild previously **deep-cloned the whole method-body AST** each time; it's now an O(1) refcount bump. The AST is copied once when a function is created from its declaration (never per call) and is never mutated afterward, so the sharing is safe. Complements the earlier direct-instance-invocation fast path, which already removed the clone for plain (non-model) instance methods.
* **perf(pdf):** embedded raster images (SVG logos, QR codes, barcodes) are now Flate-compressed, and page-content + embedded-font streams are compressed too (re-enabled `doc.compress()` in the vendored printpdf, which upstream left off). Previously every image XObject and every content/font stream was written **uncompressed** — a single 512×512 logo was ~768 KB — so an image-bearing document ballooned to megabytes. Compression is **lossless** (forced `FlateDecode`, never lossy JPEG), so QR/barcodes stay pixel-exact and scannable, and `lopdf` skips streams that already carry a filter so images aren't double-encoded. The five-page Helios annual-report sample dropped from **~2.3 MB to ~110 KB (~20×)**, cutting the base64 payload, transfer, and browser decode of the PDF playground proportionally
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

* **feat(graph):** `soli graph build` now works on **any codebase**, not just Soli apps. Point it at any repository and pass `--ext rb,erb,slim` (or commit a `.soligraph.toml`) and it indexes it into the same SolidB graph — SolidB settings come from the project's `.env`. A new feature-gated `soli-codegraph` crate uses **tree-sitter** to extract real `class`/`module`/`method`/`function` nodes + `inherits`/`implements`/`imports` edges for **Ruby, Python, JavaScript/JSX, TypeScript/TSX, Rust and C#**; every other extension (templates, config, …) is chunk-embedded so semantic search still covers it. Everything downstream is reused — incremental MD5 skip, non-destructive sync, `soli graph query`, dev-watch. Cross-file `calls` resolution is best-effort (unambiguous name match). Config precedence: flags > `.soligraph.toml` > defaults (with `.git`/`node_modules`/`vendor`/… skipped). Behind the default-on `codegraph` cargo feature (drop it for a lean standalone runtime).
* **feat(graph):** `soli graph build [folder]` — extract a **graph of your project's source code** (nodes: files, classes, models, controllers, methods, functions, routes, views; edges: `defines`, `inherits`, `implements`, `imports`, `calls`, `instantiates`, `renders`, `routes_to`, `relates`) and store it in SolidB as `soli_graph_nodes` + `soli_graph_edges`, so agents can retrieve code by **semantic search** and **graph traversal** — graph RAG over your own codebase. Every node's text is embedded (vector index over `embedding`); `--no-embed` builds a purely structural, offline graph and `--dry-run` prints the whole graph as JSON without touching SolidB or the embedding API. Connects to the same SolidB the app's Models use (via `.env`), overridable with `--database`. Call-graph resolution is precision-first: class-method / `this.` / unambiguous bare-function calls are linked, instance calls on a variable are not inferred. **Incremental & non-destructive**: it MD5-hashes every source file into a manifest so a re-run when nothing changed is a fast no-op, and otherwise updates SolidB *in place* (insert new / update changed / prune removed) rather than dropping the collections — a concurrent reader never sees an empty graph and unchanged embeddings are reused (only changed text is re-embedded). `--fresh` forces a full clean rebuild. A progress bar over the parse / embed / sync phases (a TTY bar, or sparse percentage milestones on a non-TTY) keeps big projects from looking frozen; app output from executing `routes.sl` is captured so stdout (and `--dry-run` JSON) stays clean. **`soli graph query "<question>"`** turns a natural-language task into the most relevant code *plus its immediate graph relationships* in one call (semantic ANN seed → 1-hop graph expansion → ranked), `--json` for agents, with a keyword-ranked fallback when the graph has no embeddings. **`soli serve --dev` auto-reindex** (on by default when `SOLI_EMBEDDING_API_KEY` is set — semantic search is the point of the graph; `SOLI_GRAPH_WATCH=0`/`1` forces off/on): the graph reindexes itself on every `.sl`/`.slv` save, riding the dev file-watcher on a background thread and reusing the live route table (never re-executing `routes.sl`, which would pollute the process-global WebSocket registry). It's **incremental on embeddings** — nodes whose text is unchanged keep their existing vector, only changed nodes are re-embedded — so a one-file save costs a re-parse and a handful of embeddings, and the semantic layer never goes dark. Docs: `www/docs/graph.md`
* **feat(test):** N+1 and query-budget assertions for request specs — every response from `get()`/`post()`/`request()` now carries `response["query_count"]` (AQL queries the request ran) and `response["n_plus_one"]` (`{query, count}` for every template that fired ≥2×). `assert_no_n_plus_one(response)` fails when a query was issued in a loop (the same detection behind the dev bar's N+1 badge), and `assert_query_count(response, n)` / `assert_max_queries(response, n)` hold an endpoint to a query budget. Instrumentation rides back from the `--dev` test server on `x-soli-test-query-count` / `x-soli-test-n1` headers (test-runner only; no dev/prod leak)
* **feat(test):** `soli test --fail-on-n1` — a suite-wide N+1 guard. With the flag, any `get()`/`post()`/`request()` whose response carries a non-empty `n_plus_one` fails its test automatically, with the same detection and message as `assert_no_n_plus_one` but no per-test assertion. The check runs at the response-building choke point in `request_helpers` and reuses the shared detection; clean and uninstrumented responses are untouched, so it never trips spuriously. Composes with `--jobs`/`--coverage`; ideal for a CI job that catches query regressions in specs that predate the check
* **feat(live):** reactive live queries — `Model.live_where(filter)` inside a LiveView handler runs the query like `where(filter).all()` **and** subscribes the view to the collection, so a later write re-renders it automatically (the diff gate drops frames with no visible change). Per-row matching: a flat-equality hash filter only wakes subscribers the changed row satisfies (numeric-aware; `null` matches a missing field), while the string filter form, deletes, and transaction commits wake conservatively. Transaction writes wake on commit (never on uncommitted rows / rollback). Single-process (subscriptions live in server memory)
* **feat(live):** LiveView collection streams — a handler can return a `stream` sub-hash to push targeted DOM ops (`append`/`prepend`/`insert`/`remove`/`reset`) straight to a container by id, instead of re-rendering and diffing the whole list (Phoenix LiveView streams / Turbo Streams model; ideal for chat logs, feeds, leaderboards). Streamed rows live outside the diff shadow so patches never fight them
* **feat(realtime):** unified `broadcast(channel, payload)` (+ `Model.broadcast(payload)`) — fan a payload out to every WebSocket connection in a channel **and** every SSE subscriber of the topic of the same name in one call, so a page listens over whichever transport it uses. Non-string payloads auto-serialize to JSON; returns the SSE subscriber count
* **feat(ai):** one-call RAG — `Model.rag(question[, opts])` embeds the question, ANN-searches the `vector_index` for the top-k rows, builds an LLM context from each row's text field, and returns `{ answer, sources }`. Plus streaming LLM: inside an `sse` block, `out.llm_stream(system, user)` streams a completion token-by-token to the client and returns the full answer
* **feat(api):** opt-in OpenAPI — `SOLI_OPENAPI=1` exposes an OpenAPI 3 spec generated from the routes at `/openapi.json` and a Scalar API-reference UI at `/openapi` (`:id` → `{id}` path params, `controller#action` operationId, controller tags). Opt-in (404 otherwise); served in every environment once on, like `/_metrics`. `SOLI_OPENAPI_TITLE` sets the title
* **feat(dev):** database browser at `/__soli/db` (dev-only) — browse SoliDB collections, paginate rows (`/__soli/db/<collection>`), view a document as JSON (`/__soli/db/<collection>/<_key>`), and run a read-only SDBQL query. Mutating queries are rejected, collection names are validated + allow-listed, and the routes exist only under `--dev`
* **feat(dev):** mailer preview gallery at `/__soli/mailers` (dev-only) — lists every `app/views/<name>_mailer/<action>.html.slv` and renders each email body in an iframe, with example data from a leading `<%# preview: {json} %>` header (the same convention as the component catalog)
* **feat(dev):** request replay from the dev bar — a ↻ button on each request row re-dispatches a captured request through the real worker path (same method/path/headers/body) to reproduce a bug server-side; the bar retargets its panels to the replay's fresh request id. The per-form CSRF token check is skipped for replays and responses are tagged `X-Soli-Replay: 1`. Dev-only (the capture store is empty in production)
* **feat(scaffold):** `soli generate auth` is now a one-command, Devise-style suite — beyond login/signup it scaffolds **password reset** (`/password/reset`: hashed one-time tokens via `uuid_v4` + SHA-256 digest, 2h expiry, enumeration-safe responses, auto-login on success), **email confirmation** (sent on signup, `/confirm_email` + `/confirmation/resend`; enforcement is the `auth_require_confirmed_email` toggle, off by default so the flow works before SMTP is configured), **remember-me** (HttpOnly `Max-Age` cookie carrying `key:token` with a digest-stored token, constant-time-compared and promoted to a fresh session by the `load_current_user` middleware, cleared on logout), and **account lockout** (10 failures lock for 30min with auto-unlock; correct passwords are refused while locked). All thresholds are constants at the top of the generated `app/models/user.sl`; an `AuthMailer` + email views render the links from `auth_base_url`. Routes/migrations append idempotently with separate markers, so apps generated before this pick up just the new flows on a re-run
* **feat(session):** `set_cookie(name, value, options?)` — the optional hash sets cookie attributes: `max_age` (0 expires immediately), `expires`, `http_only`, `secure`, `same_site` (Lax/Strict/None), `path`, `domain`. Attribute values are validated against header/attribute injection and unknown keys raise, so a typo can't silently weaken a cookie. Previously application cookies were emitted with `Path=/` only, which made persistent or HttpOnly cookies (e.g. remember-me) impossible
* **fix(lint):** `naming/snake-case` no longer flags `const MAX_SIZE = 100` — constants accept SCREAMING_SNAKE_CASE (or snake_case); only mixed casing is reported. The documented convention was previously unlintable
* **feat(model):** single-collection inheritance (STI) — a model inheriting from another model (`class Admin < User`) now shares its base's collection with a `type` discriminator, Rails-style. Subclass writes stamp `type` (`create`/`save`/`find_or_create_by`); rows hydrate as their stored type everywhere (guarded: a `type` value that doesn't name a model class of the row's collection is ignored, so user data fields called `type` can't hijack hydration); subclass queries are type-scoped including descendants (`where`/`all`/`count`/`find_by`/`first_by`/`delete_all` + every chained QueryBuilder) while the base class matches every row; `find` on an out-of-hierarchy row raises `RecordNotFound` and class-form `update(id)`/`delete(id)` refuse it. Metadata copies down at class definition — validations (incl. `if:`/`unless:` closures), callbacks (named + closure), relations (with the base's FK), scopes, soft-delete, `attr_accessible`, `encrypts`, enums, and state-machine closures — with the subclass's own DSL appending after. Previously `class Admin < User` silently queried a separate empty `admins` collection with none of the parent's rules
* **feat(model):** association writers — `owner.posts << record` (instance or array; adopts unpersisted records by creating them) and `owner.posts.create({...})` on any has_many accessor stamp the foreign key automatically; on polymorphic `as:` inverses both halves are stamped (`customer.comments.create({...})` sets `commentable_id` + `commentable_type`). Both route through the regular save path (validations, callbacks, counter caches, dirty tracking); the association seed wins over caller-supplied FK/type values; `.create` returns the instance with `Model.create`'s `_errors` contract while a failing `<<` raises. Unpersisted owners, non-instance pushes, and `.create` on `through:` relations raise with actionable messages
* **feat(model):** polymorphic associations — previously documented but non-functional (`polymorphic: true` was silently dropped and the relation aimed at a nonexistent collection); now implemented on both sides. Child: `belongs_to "commentable", polymorphic: true` stores `{name}_id` + `{name}_type` and the accessor resolves the class/collection from the type at runtime (null when either field is missing; unknown type raises naming it). Parent: `has_many/has_one "comments", as: "commentable"` — accessor, `includes`, `includes_count`, `join`, and cascades all carry the `{as}_type == "<OwnerClass>"` guard. `counter_cache:` works on polymorphic belongs_to (parent collection resolved from the type at bump time; cross-type reassignment moves the count; `reset_counters` counts with the guard) and `dependent:` works on `as:` relations (`"nullify"` clears both FK and type field). Eager-loading a polymorphic belongs_to raises (per-row dynamic collections — same restriction as Rails); `polymorphic: true` + `class_name:` and `as:` on belongs_to raise at class load
* **feat(model):** counter caches — `belongs_to "post", counter_cache: true` maintains `posts.comments_count` (custom column via `counter_cache: "name"`; no schema prep, a missing column reads as 0). Bumps ride the If-Match CAS loop and fire on child `create`/`save`, hard `delete` (instance + class form), FK reassignment on `update`/`save`/`Model.update(id, …)` (−1 old parent, +1 new; consumes the dirty-tracking delta), and soft `delete`/`restore` for soft-deleting children — counters track default-scope-visible rows. Bumps are best-effort (a failure never fails the committed primary write) and bulk writes (`delete_all`/`update_all`/`upsert`/`prune`) skip them by design; `Model.reset_counters(id, relation)` recounts (minus soft-deleted) and returns the fresh count. `counter_cache:` on a non-belongs_to or with a non-bool/string value raises at class load
* **feat(model):** `has_many through:` associations — `has_many "teams", through: "memberships"` traverses an intermediate relation and returns a chainable QueryBuilder over the target collection, filtered by a single-query membership subquery (`doc._key IN (FOR jt IN … RETURN jt.team_id)`; no N+1, composes with `.where`/`.order`/`.count`/aggregations). Source relation inferred by singularizing the name, overridable with `source:`; both `belongs_to` sources (join model) and `has_many` sources (distant children) supported; a soft-deleting through model excludes soft-deleted join rows. Resolution is lazy at first access — missing through/source relations raise naming exactly what was searched and suggesting `source:`. `user.teams << team` (or `<< key`) creates the join record HABTM-style (raw join-row insert; through-model validations/callbacks skipped, its counter caches bumped; `has_many`-source pushes and unpersisted owners raise). `delete_all`/`update_all` on a through association raise (they'd hit target rows, not join rows) and eager-loading (`includes`/`join`/`includes_count`) raises
* **feat(model):** cascade deletes — `has_many`/`has_one` accept `dependent: "delete"` (per-row child deletes through the interpreter: callbacks, nested cascades, child soft-delete semantics; `"destroy"` is an accepted alias), `"delete_all"` (one bulk REMOVE, no callbacks), and `"nullify"` (bulk FK → null). Ordering mirrors Rails: `before_delete` (a veto aborts cascades too) → cascades in declaration order → owner row delete → `after_delete`; a child veto/error aborts the owner delete. Cascades fire on **hard** deletes only (soft-delete owners keep their children) and on `Model.delete(id)` when the class declares dependents; bulk writes (`delete_all`/`update_all`/`prune`) never cascade. Cycles terminate via an in-flight guard + 32-level depth cap. Invalid strategies, `dependent:` on `belongs_to`, or combining with `through:` raise at class load. Relation option parsing is now shared across all four DSL entry points and accepts symbol values (`dependent: :delete_all`)
* **feat(model):** dirty tracking — model instances expose `changed?`, `changed` (sorted names), `changes` (`{name: [old, new]}`), `previous_changes`, and `attribute_was("field")`. The baseline seeds on DB load (`find`/`where` hydration) and resets on successful `create`/`save`/`update` and `reload`; atomic `increment`/`decrement` and soft `delete`/`restore` keep their written field clean; failed validations/DB errors leave the record dirty and `previous_changes` untouched. New records report every attribute as changed (`[null, value]`), matching Rails. Tracking is value-based (`enum_aware` equality): reassigning an equal value is not a change, and in-place mutation of a nested Hash/Array is documented as untracked — reassign the attribute to record it. No dynamic `name_was` methods; use `attribute_was("name")`
* **feat(liveview):** DOM-aware patching — LiveView updates no longer destroy client-side state. The server now ships positional line splices (`{type:"splice", at, del, ins}` against a client-held shadow of the exact previous render; byte-exact by construction, so the old string-matching failure mode is gone), and the client **morphs** the live region's DOM to match instead of swapping `innerHTML`: nodes are mutated in place, so focus, caret/selection, scroll, and widget state (Alpine, charts) survive every patch — including full re-renders of tiny components. New template attributes: `soli-key` (list-item identity across reorders; falls back to `id`) and `soli-ignore` (client-owned subtree: attributes stay server-driven, children are never touched). Form fields follow a "user wins" rule — a focused field is never clobbered (typing that round-trips through `soli-change` can't lose in-flight keystrokes), and an unfocused field only changes when the server actually changes the rendered value/checked/selected attribute. `<script>` tags patched into a live region never execute (unchanged contract, now enforced explicitly). A new client→server `resync` message replays the last full render when a patch can't apply, without resetting server state. The client is now **embedded in the soli binary** and served at `/live/client.js` (`no-cache` + version ETag) — always protocol-matched with the server, no vendored file, and every `soli new` app gets LiveView with zero setup (`www/public/js/live.js` is gone; update `<script src>` tags to `/live/client.js`). ~7 KB gzipped. Covered by round-trip splice tests in `src/live/diff.rs` and a jsdom morph suite (`www/test/live_morph_test.js`, `npm run test:js`)
* **feat(cli):** `soli routes [folder]` — Rails-style route lister. Prints the app's fully expanded route table (every route `config/routes.sl` and mounted engines register, including each route a `resources(...)` call generates) without starting the server: columns for method, path, `controller#action`, the generated `*_path` helper, and per-route middleware, plus WebSocket routes (`WS`). Rows print in registration order — the same order the server matches requests. `-g PATTERN` filters case-insensitively over method/path/handler/helper; `--json` emits a stable `[{method, path, handler, name, middleware}]` array for scripts and coding agents. Loading mirrors the serve boot sequence (middleware → engine mounts → routes DSL → `config/routes.sl` → engine routes) with no DB required
* **feat(model):** graph models — `edge from: "users", to: "users"` marks a model as a SolidB edge collection (auto-created with the right type + `_from`/`_to` hash indexes). `Follow.create(from: alice, to: bob)` coerces endpoints from instances, `"coll/key"` ids, or bare keys (invalid/missing endpoints surface as `_errors`, and db-qualified `_id`s are normalized to the plain `coll/key` form the traversal engine matches on). `record.traverse(Follow, depth: [1, 3], direction: "any")` returns a chainable QueryBuilder — the vertex variable is `doc`, so `.where`/`.order`/`.limit`/`.count`/`.exists` compose exactly like a collection query, and edge attributes filter via `"edge.since >= @y"`; `record.shortest_path(other, via: Follow)` runs a BFS shortest path and returns the vertex chain (`[]` when unconnected). Incompatible modes (`includes`, `join`, `group_by`, bulk writes, …) reject with a clear error
* **feat(model):** timeseries models — `timeseries retention: "30d"[, timestamp: "recorded_at"]` marks the collection insert-only (UUIDv7 keys give time ordering). Updates (`update`/`upsert`/`save`-on-existing/`increment`/`update_all`) fail fast with an actionable message before any DB round trip, mirroring the server's own rules (deletes stay allowed). `Metric.where(...).time_bucket("5m", { "avg": "value", "max": "value" })` (keyword style works too) emits `COLLECT bucket = TIME_BUCKET(...) AGGREGATE ...` and returns `{bucket, alias...}` rows sorted by bucket; `Metric.prune("30d")` / `prune("<RFC3339>")` / bare `prune` (declared retention) deletes old rows and returns the count. New plumbing: `Solidb#prune_collection` and `MigrationDb.prune_collection`
* **feat(model):** analytics-grade aggregation — `group_by("country")` / `group_by(["country", "plan"])` (multi-key), `aggregate({ "total": ["sum", "amount"], "n": ["count"] })` (multi-aggregate; also on ungrouped chains), `having("total > @min", {...})` (post-COLLECT filter over bare aliases), grouped `order`/`limit` validated against group fields + aliases, and stats terminals `median`/`stddev`/`variance`/`count_distinct` (emitted via `COLLECT_LIST` since they're array functions server-side; `percentile` is rejected with a clear message — SolidB has no such function). Result rows are plain hashes keyed by group fields + aliases. The legacy 3-arg `group_by(field, func, agg_field)` is byte-for-byte unchanged (regression-pinned); unlike it, the new grouped form honors the soft-delete scope
* **feat(model):** columnar models — `columnar [compression: "lz4"]` + `column "url", "string"[, nullable:, indexed:]` declare a model backed by SolidB's columnar engine (typed columns, LZ4). Surface maps 1:1 to the engine: `insert_rows([...])` (auto-creates the store in dev), `aggregate(column, op[, { "group_by": [...] }])`, `query({ "columns": [...], "filter": { "column", "op", "value" }, "limit": n })` (single filter, ops eq/ne/gt/gte/lt/lte/in — enforced client-side), `count`, `add_column_index(column[, "sorted"|"hash"|"bitmap"|"minmax"|"bloom"])`, `column_indexes`, `drop_column_index`, `columnar_stats`. Columnar stores have no document API — every inherited document static (`find`/`where`/`create`/…) raises an honest pointer at the columnar surface. Migrations gain `db.create_columnar(name, columns[, options])` / `db.drop_columnar(name)`
* **feat(model):** search pushdown — `vector_index "embedding", dimension: 1536[, metric:, m:, ef_construction:, quantization:]` makes `similar()` run on the DB's HNSW index (results still carry `_similarity_score`): text queries embed client-side as before, a numeric vector literal skips embedding entirely, composed `.where(...).similar(...)` over-fetches ANN candidates (4×k, cap 400) then applies the filters (may return < k — documented), and `{ "exact": true }` forces the historical client-side exact cosine. Models without the declaration keep the old behavior unchanged. `fulltext_index "title", "body"` + `Model.search("query"[, { "field":, "distance":, "limit":, "highlight": }])` returns ranked instances with `_search_score` (+`_highlighted`); `geo_index "location"` + `Model.near(lat, lon[, { "limit": n }])` / `Model.within(lat, lon, radius)` return instances with `_distance` meters. `index "email", unique: true` / `index ["a", "b"], type: "hash"` declares secondary indexes (persistent default; hash/fulltext/bloom/cuckoo). Declared indexes are ensured at dev-server boot and by the new **`soli db:indexes [folder]`** command (list-first, idempotent, auto-creates missing typed collections); `__sync_model_indexes()` exposes the same sweep to scripts/tests. Migrations remain the recommended production DDL path
* **feat(parser):** class-body DSL statements now accept Ruby-style named-argument command form (`edge from: "users", to: "users"`, `timeseries retention: "30d"`), trailing named args after positionals (`column "ms", "int", nullable: true`), array-first form (`index ["tenant_id", "email"], unique: true`), and bare macros (`timeseries`, `columnar`) — previously only paren/symbol/string forms parsed, and only for a fixed macro list
* **feat(session):** encrypted client-side sessions — `SOLI_SESSION_DRIVER=cookie` (or `session_configure({"driver": "cookie", "secret": ...})`) stores the whole session in the cookie as an AES-256-GCM-sealed blob (`v1.<base64url>`), Rails-cookie-store-style: sessions survive restarts and work across load-balanced hosts with zero infrastructure. The key is HKDF-derived from `SOLI_SESSION_SECRET` (32+ chars required; rotating it invalidates every outstanding session — the kill switch), tampered/expired/foreign blobs are silently replaced by a fresh session, the cookie is only re-emitted when the session actually changed (read-only responses stay cacheable), and `session_id()` still returns a stable internal UUID. Honest trade-offs in the docs: ~4KB ceiling (an oversized session refuses to seal, with a loud log line, rather than emit a cookie the browser drops) and no server-side revocation. A misconfigured driver at boot (e.g. missing secret) now logs the in-memory fallback loudly instead of downgrading silently
* **feat(templates):** Rails-style `content_for` / named `yield` — a view or partial can capture a named block with `<% content_for "head" do %> ... <% end %>` and the layout splices it back with `<%= yield "head" %>` (or the `content_for("head")` read-form), closing the "no way to push per-page `<head>` scripts/meta/sidebars into the layout" gap. Repeated captures for one name append in document order; a name nothing captured renders empty (no guard needed); captures inside partials reach the layout; interpolations are escaped once at capture time and spliced raw (no double-escaping), exactly like the main `yield`. A new `content_for?("name")` view helper gates conditional layout sections (`<% if content_for?("sidebar") %>`), true only for a non-empty capture. Names are string literals so insertion points are known at parse time; `<% content_for "x" %>` without `do` and non-literal names get clear parse errors, and `soli lint` handles capture blocks in `.slv` files
* **feat(model):** conditional and per-operation validations — `validates(field, {...})` now accepts `on: "create"`/`on: "update"` to restrict a rule to one persistence operation, and `if:`/`unless:` closures (receiving the attribute hash) to gate a rule on the record's data. Wrong `on:` values or non-function conditions raise at class-load time instead of silently running the rule unconditionally. `run_validations` also no longer holds the model-registry lock while running checks (uniqueness hits the DB, conditions run user closures), and field lookups use short-lived borrows so a condition/validator closure that mutates the record can't panic the run
* **feat(bundle):** `soli build --encrypt` produces an AES-256-GCM-encrypted `.soli` bundle whose decryption key is resolved at boot from `SOLI_BUNDLE_KEY` or fetched from a key server (`SOLI_BUNDLE_AUTH_URL`, authenticated with `SOLI_BUNDLE_API_KEY` as an `x-api-key` header) — revoking the key server-side is a remote kill-switch, and a wrong/rotated key fails the boot with a clear error. `soli build --protect` additionally replaces every `.sl` source with its compiled binary AST (MessagePack), so no readable source ships in the bundle (comments and formatting are gone; identifiers/literals remain, like any bytecode). The middleware directives and controller-registry metadata that the serve pipeline normally scrapes from source text are precomputed into a `__soli_meta__` entry, and a protected bundle is locked to the exact Soli version that built it. Decrypted files extract to RAM-backed `/dev/shm` (mode `0700`) and are removed on shutdown; when `/dev/shm` is unavailable the boot is refused unless `SOLI_BUNDLE_ALLOW_DISK=1` is set. Config vars can live in the `.env` next to the bundle. Honest threat model in the docs: protects against casual copying and enables revocation, not against root on the running host
* **fix(bundle):** `soli serve app.soli` now renders templates and loads `.env` correctly. `DiskFS` no longer double-prefixes absolute paths that are already under its root (every `render()` in a bundle used to fail with "Template not found"), and the `.env` beside the `.soli` file is loaded before boot so `SOLIDB_*`/session config resolves
* **feat(pdf):** multi-series charts — give a chart `values` (an array of `{ field, name?, color? }`) instead of a single `value` to draw **grouped** or **stacked** (`mode: "stacked"`) bars, or multiple lines, with a shared legend of the series names; opt-in `gridlines: true` adds horizontal value-axis gridlines + tick labels. Single-series charts are unchanged. The annual-report sample's quarterly chart is now a grouped FY2024-vs-FY2025 comparison, and a new chart appears in the playground "Report" preset
* **feat(pdf):** data-driven control flow in templates — `repeat` lays out a block of elements once per item of a data array (with `${field}` scoped to each item, like a data-bound table row), and `if`/`unless` conditionally render a block (truthiness, or string `equals`, with an optional `else`). Documents can now be data-driven beyond table rows (sections per item, conditional banners). A new "Dynamic" playground preset demonstrates it
* **feat(pdf):** `options.color` colors a whole plain `paragraph` (and list items / footer text) — previously a non-black color required the `spans` form; and document `options.background` paints a page background fill behind every page, beneath any watermark and content
* **docs(pdf):** the PDF gallery's "Report" sample is now a five-page **annual report** — a branded cover with an SVG sun mark and KPI cards, `bar`/`line`/`pie` charts, data-bound P&L and regional tables with a filled header band, a nested list, a running header with a "Page X of Y" footer, a diagonal `CONFIDENTIAL` watermark, and a verification QR + barcode — replacing the single-page quarterly report. The template + data (`pdf/samples/report.json`) drive both the docs gallery and the live playground; sample glyphs are kept within the bundled Titillium/JetBrains Mono coverage so nothing falls back to the heavy CJK font
* **docs(pdf):** the PDF playground now shows the **server-side render time** next to the round-trip — the render endpoint returns an `X-Render-Ms` header and the status line reads `✓ N KB · engine N ms · total M ms`. Makes it obvious that the engine is ~tens of ms and the rest of any perceived latency is round-trip (network + base64 transfer + browser decode), not the renderer
* **feat(serve):** auto-loading is now recursive. `app/models/`, `app/services/`, and `app/policies/` are walked into subdirectories, so nested files like `app/models/billing/invoice.sl` or `app/services/payments/stripe.sl` are loaded without an `import` (previously only top-level `*.sl` files in each directory were picked up). Within a directory, files load alphabetically and before their subdirectories (top-down) so a base class at an equal-or-shallower depth is defined first; the `--dev` watchers for models/services are now recursive too, so edits to nested files hot-reload
* **feat(controllers):** per-action layouts — a controller can now map different layouts to different actions, declared once in its `static { ... }` block instead of repeating `layout:` on every `render(...)`. `this.layout = "admin"` stays the controller-wide default; `this.layout("print", only: [:invoice, :receipt])` / `except: [:index]` add Rails-style filtered overrides (checked in declaration order, first match wins, then the default, then `"application"`). An explicit `layout:`/`layout: false` on `render(...)` still wins, and per-action rules are inherited by subclasses (a child rule for the same action overrides the parent's). Resolved against the in-flight action via the controller registry, so `--dev` edits apply on the next request with no restart. The controller static-block DSL (`layout`/`before_action`/`after_action`) is now treated as declarative — calling one on the class inside `static {}` is a no-op rather than trying to invoke the field's value, which is what makes `this.layout("...")` viable alongside `this.layout = "..."`
* **feat(serve):** Instant Navigation — Turbo-Drive-style body swapping, on by default. A framework script (`/__soli/nav.js`, auto-injected like prefetch.js) intercepts same-origin GET link clicks, fetches the page (reusing an in-memory hover-prefetch cache that rides the existing `Purpose: prefetch` / `SOLI_PREFETCH_TTL` server machinery), swaps `<body>` in place merging title/stylesheets/meta, and manages history with `pushState`/`popstate` (scroll restore, cheap 304 refetch on back/forward). Inline body scripts re-run per visit, external scripts once per URL (no Alpine/htmx double-boot), all sequenced in document order with externals awaited (parser semantics — an inline `tailwind.config` after the Tailwind CDN script keeps working), then `Alpine.initTree` + `htmx.process` re-wire the new body only after the script chain settles; `DOMContentLoaded`/`load`/`alpine:init` listeners registered after the events already fired are replayed (jQuery-ready semantics), so existing init code wrapped in `DOMContentLoaded` and page bundles registering `Alpine.data` components via `alpine:init` keep working after swaps; the incoming body carries `x-ignore` until its scripts settle so Alpine's MutationObserver can't initialize it early; `soli:visit` / `soli:before-render` (cancelable) and `soli:load` events for userland hooks; opt-in View Transitions via `<meta name="view-transition" content="same-origin">`. Skips htmx-managed links, `data-method`, downloads, new-tab intent; falls back to a real navigation for non-HTML responses, cross-origin redirects, and `x-teleport` pages. Opt out per link (`data-no-nav`), per page (`<meta name="soli-nav" content="off">`), or globally (`SOLI_NAV=off`, which restores plain hover prefetch)
* **feat(serve):** `SOLI_SLOW_REQUEST_MS` — production slow-request logging: a request whose total time (queue wait + handler) crosses the threshold prints a full `[SLOW]` detail block (every `SOLI_LOG` channel plus the queue-wait split); faster requests stay silent. Composes with `SOLI_LOG`. The access line now shows queue wait (`(12.3ms + 0.4ms queue)`) when request logging is active, so a request stuck behind a busy worker is distinguishable from a slow handler
* **feat(lang):** `Int#to_s(base)` — Ruby-style radix conversion for bases 2–36: `255.to_s(16)` → `"ff"`, `255.to_s(2)` → `"11111111"` (lowercase digits, leading `-` for negatives, `i64::MIN`-safe). Complements the existing `"ff".hex` reverse direction
* **feat(lang):** explicit empty parens on zero-arg builtin methods now work in both engines (`n.abs()`, `x.to_f()`, `dt.year()`) — previously "Cannot call non-function value" on primitives while collections accepted them; the type checker also types bare zero-arg member access as the method's return type (`s.length` is an `Int`, matching runtime auto-invoke)
* **feat(i18n):** `I18n.cache_table(locale, table)` / `I18n.cached_table(locale)` — a per-worker-thread translation-table cache. App-level i18n that builds each locale's table from a hash literal in a view helper (which runs in an isolated per-thread env with nowhere to memoize) can now build it once per thread instead of on every `tr()` call; on a chat-heavy page calling `tr()` thousands of times the per-call table rebuild had dominated render. Cleared on view-helper hot-reload so `--dev` edits aren't stale; cached hashes share storage and are read-only

### Language

* **language:** removed the dead `async`/`await` **keywords** and the unreachable `ExprKind::Await` AST node. They were never implemented — writing `await <expr>` hit an `unimplemented!()` that panicked the interpreter and type-checker — so this deletes dead surface only. The **`await()` builtin** (resolves a `Future`, e.g. the handle from `System.run(...)`) is unchanged; `await(x)` is now an ordinary function call, and `async`/`await` are freed as ordinary identifiers

### Fixed

* **fix(vm):** named-argument calls (`f(x: 1)`, `T.new(field: v)`, `x |> f(a: 1)`) no longer emit broken bytecode in the compiled VM. The old path emitted a no-op `Op::NamedArg` that over-counted the argument count, so the call could resolve the *wrong* callee (silent misdispatch) or underflow the value stack and panic a worker thread. The compiler now rejects named-arg calls with a clean `CompileError`, so the request deterministically falls back to the tree-walking interpreter (which reorders named args correctly) — the same VM→interpreter fallback already used for `&.` safe navigation and block-form `transaction`/`grouped`. Pinned by `named_args_compile_tests` (all four call-compilation paths) plus a `named_args_spec.sl` behavior spec.
* **fix(vm):** removed two dead, never-emitted opcodes (`GetAndIncrLocal`/`GetAndDecrLocal`) that carried latent `.unwrap()` panics on non-numeric operands, and dropped a wasted per-`Import` string allocation in the dispatch loop.
* **fix(interpreter/types):** the unreachable `spread`- and `throw`-as-expression paths now return a clean error / type instead of `unimplemented!()` (which would panic if a future grammar change ever routed to them). Guarded by `spread_eval_tests` and `throw_check_tests`.
* **fix(vm):** `array.flatten()` now flattens recursively (and accepts an optional non-negative depth) in the compiled VM, matching the tree-walking interpreter. The VM previously flattened only one level and rejected a depth argument, so under `--vm` `[[1, [2]], 3].flatten()` returned the shallow `[1, [2], 3]` instead of the correct `[1, 2, 3]`. The interpreter and VM now share one `array_ops` implementation of `flatten`/`uniq`/`compact`, pinned by differential-engine tests so they can't drift again.
* **fix(scaffold):** generated resource views were broken end-to-end and now work: row links and delete forms used `record["id"]` (SoliDB rows carry `_key`) so every Show/Edit/Delete link pointed at `/posts//edit`; the new-form action was `"/ posts"` (embedded space → 404 on submit); edit/delete forms emitted `_method` hidden fields nothing honored (updates hit the wrong route); and the `_form` error block read `["valid"]`/`["errors"]` keys the ORM never populates (real shape: `_errors`). Views are now generated on the form builder (`form_with` + `button_to` + `error_summary`), and the form partial is included via `partial("res/form", {..., "f": f})` instead of the view-scoped `render(...)` that produced no output
* **fix(scaffold):** `soli generate auth` forms (login, signup, password reset/edit, confirmation resend) now embed `csrf_field()`, and the `soli new` layout carries `csrf_meta_tag()` for fetch/htmx clients
* **fix(interpreter):** named arguments on built-in method receivers (`Value::Method` — QueryBuilder chaining and friends) were **silently dropped**; they now collapse into a trailing options hash exactly like native-function calls, so `.time_bucket("1h", avg: "value")` works. Strictly an improvement — the old behavior lost the arguments without any error
* **fix(migrations):** `db.create_collection(name, "columnar")` now raises with a pointer at `db.create_columnar(name, columns)` — it used to silently create a mislabeled *document* collection (the columnar engine lives behind a different endpoint), so every "columnar" migration written against the old docs produced a store that wasn't columnar at all
* **fix(solidb):** `Solidb#create_index` options hash accepts `type:` (`hash` stays the default for compatibility; `persistent`/`fulltext`/`bloom`/`cuckoo` now reachable — the client previously hardcoded `hash`), and the `sparse` key is no longer sent on the wire (the server never read it)
* **fix(model):** a chained `.where()` no longer wipes internal machinery bind vars (`__soli_`-prefixed, e.g. the traversal start vertex) — user binds keep the historical replace semantics
* **fix(cli):** `soli build` accepts its folder argument in any position relative to the flags. Previously the folder had to come first, so `soli build --protect my_app` treated `--protect` as the folder name and rejected `my_app` with "Unknown option for build". The folder is now parsed as the first non-flag token, matching the usual Unix convention; a stray second positional or an unknown `-`-prefixed option still errors clearly
* **fix(migrations):** `soli db:migrate up` (and `down`/`status`) now create the target SoliDB database when it doesn't exist yet, instead of failing with `HTTP 404 … Database '<name>' not found` while listing collections. A new `ensure_database` step lists the databases and creates the configured one if absent — the database-level analogue of the existing `_migrations`-collection bootstrap — so a fresh project can migrate against a brand-new database with no manual setup
* **fix(pdf):** inline SVG `data:` URI colors written URL-encoded as `%23RRGGBB` (what a browser emits when you copy an SVG) now render — `fetch_bytes` leniently percent-decodes `%XX` while leaving a bare `%` (SVG percentages like `width='50%'`) intact. Previously `%23` reached usvg verbatim as an invalid color and the fill rendered black; a literal `#` keeps working too
* **fix(build):** capped the `time` crate at `<0.3.52`. time 0.3.52 changed `Parsable::parse` to a 2-argument signature that `cookie` 0.18.1 still calls with one argument, so an **unlocked** build (`cargo install` without `--locked`, or after a `cargo update`) failed to compile `cookie` with `E0061`. The floor stays `>=0.3.47` (RUSTSEC-2026-0009) and lopdf 0.43 needs `>=0.3.51`, so the unified version resolves to exactly 0.3.51. Drop the ceiling once `cookie` ships a release built on time 0.3.52
* **fix(serve):** auto-render now honors the controller's registered layout. An action that sets `@vars` and lets the matching view render *without* an explicit `render(...)` call went through a path that passed no layout and silently fell back to the `"application"` layout — so `static { this.layout = "admin" }` (including layouts inherited from a base controller and per-action rules) was ignored unless every action called `render(...)` with an explicit `layout:`. The auto-render path now resolves the registered layout for the in-flight action exactly like the explicit `render(...)` builtin; controllers that declare no layout still fall back to `"application"`. Covered by a new auto-render case in the hooks e2e spec
* **fix(serve):** WebSocket registry no longer holds the connections lock across `send().await` in `send_to` / `broadcast_all` / `broadcast_to_channel` / `broadcast_to_channel_except` / `close` — one slow or stalled client could block every other WS/LiveView operation (joins, presence, other broadcasts). Senders are cloned out under the lock and sends happen lock-free
* **fix(serve):** dev bar stayed correct across Instant-Navigation swaps — the duplicate-bar cleanup only ran on `htmx:afterSwap`, so an instant-nav (`soli:load`) swap could leave a stale bar in front of the fresh one (`getElementById` binds the first match), showing the old near-zero render time while the live panel had real per-view rows. The injected script now self-heals on every run (removes all but the newest bar) and the cleanup is bound to both `htmx:afterSwap` and `soli:load` (once-guarded)
* **fix(serve):** dev bar "view" aggregate row showed 0ms while the per-template sub-rows showed real time — the aggregate summed the `"view"` phase marker, which isn't emitted on every render path. It now derives from the root view spans the breakdown already renders (each root already includes its nested partials, so summing only roots avoids double-counting), and the controller row no longer absorbs the missing view time

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

* **feat(model):** dev-mode visibility for the permit()/attr_accessible intersection — a mass-assign key dropped by a model's `attr_accessible` whitelist now logs `[WARN] attr_accessible on <Model> dropped mass-assign key(s): …` under `--dev`, surfacing the silent-drop drift between controller `permit()` shapes and model whitelists; docs on both surfaces now state the layering (permit = primary/controller-side, attr_accessible = flat defense-in-depth, effective result = intersection)
* **feat(serve):** Rack-style nested params — bracket keys nest across form bodies, multipart bodies, and query strings: `author[name]` → `params["author"]["name"]`, `tags[]` → ordered array, `items[][sku]` → array of hashes; numeric segments are hash keys; depth capped at 32 (malformed/over-deep keys stay flat literal keys); repeated `[]` fields now survive multipart parsing (ordered pairs replaced the collapsing HashMap)
* **feat(template):** `fields_for` nested form sub-builders — `f.fields_for("author") do |author| ... end` renders `author[name]` fields prefilled from the nested document (ids flatten to `author_name` for label linkage); `fields_for("items", 0)` for indexed collections; `select` gains `{"multiple": true}` (name gets `[]`), and every field helper accepts a `{"name": "..."}` override
* **feat(security):** `permit(params, shape)` — strong parameters for documents: `true` keeps a scalar (container values are dropped so structure can't smuggle through a scalar slot), `[]` an array of scalars, `{...}` a nested whitelist, `[{...}]` an array of hashes (also accepts the numeric-keyed `items[0][x]` form, returning an array); unlisted keys are dropped. Essential with a schemaless store, where unfiltered mass-assignment persists anything; `soli generate scaffold` now writes `_permit_params` with `permit()`
* **feat(serve):** built-in CORS — `cors("/api/*", {"origins": [...], "credentials": true, ...})` in `config/routes.sl`; the server answers preflights before routing, stamps the allow headers (+ `Vary: Origin`) onto every response of the path (rendered, streamed, static, and error responses alike), and an allowed `Origin` passes the same-origin CSRF gate for that path — a more precise cross-origin opt-in than `skip_csrf`, since the origin is checked against the declared list. Options: origins/methods/headers/expose/credentials/max_age; unknown keys raise; `credentials: true` echoes the origin (never `*`); re-declaring a pattern replaces the rule so hot reload picks up edits
* **feat(template):** Rails-style form builder — `form_with(record)` returns a builder whose `open()` derives the action URL and verb from the record (new → `POST /collection`, persisted → `PATCH /collection/key` via `_method`), embeds the per-session CSRF token, and whose field helpers (`text_field`/`email_field`/`password_field`/`number_field`/`date_field`/`datetime_field`/`hidden_field`/`file_field`/`text_area`/`check_box`/`radio_button`/`select`/`label`/`submit`) prefill escaped values and decorate errored fields (`field-error` class + `aria-invalid`); `error_summary()`/`errors_for(field)` render `_errors`; plus `button_to` (one-button forms with method override, token, and JS confirm), `csrf_field()`, and `csrf_meta_tag()`. Implemented as engine-embedded Soli evaluated into the template environment
* **feat(serve):** HTML form method override — a POST whose form body carries `_method=PUT|PATCH|DELETE` is routed and dispatched as that verb (form content types only, no verb downgrades), so `resources(...)` update/destroy work from plain HTML forms. The scaffold has emitted these hidden fields since day one; the server now honors them
* **feat(security):** per-form CSRF tokens — `csrf_token()` builtin (session-backed, created on first use); state-changing requests carrying a token (`_csrf_token` field or `X-CSRF-Token` header) are verified against the session with a constant-time compare and rejected with 403 on mismatch, layered on top of the Origin/Referer gate; `SOLI_CSRF_TOKENS=require` makes tokens mandatory for browser form posts; `skip_csrf`/`SOLI_DISABLE_CSRF` opt out of both layers
* **feat(template):** `form_with` block syntax — `<%- form_with(post) do |f| -%> ... <%- end -%>` binds the builder and wraps the body in `open()`/`close()` (implicit `f` with a bare `do`); the opener is accepted in any tag style, and ERB-style `-%>` now swallows the newline following any template tag
* **feat(template):** `<% xs.each do |x| %>` (and `<% xs.each do |x, i| %>`) now work as template iteration blocks — normalized at tokenize time onto the same machinery as `<% for x in xs %>`. The docs and scaffold have used this Ruby-style spelling since the syntax sweep, but the engine never actually supported it: every scaffold-generated index/show view crashed with "Unexpected token 'EOF'"
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