# Changelog

## [Unreleased]

### Features

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