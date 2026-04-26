# Changelog

## [Unreleased]

### Features
* **feat(respond_to): content negotiation built-in** - Rails-style `respond_to` for handling multiple formats (html, json, etc.) in controller actions ([82c61ab](https://github.com/solisoft/soli_lang/commit/82c61ab))
* **feat(solidb): improved SolidB client integration** - enhanced SolidB HTTP operations ([82c61ab](https://github.com/solisoft/soli_lang/commit/82c61ab))
* **feat(migration): enhanced migration DSL** - improved database migration capabilities ([82c61ab](https://github.com/solisoft/soli_lang/commit/82c61ab))
* **feat(uploads): URL-driven image transforms on attachment endpoints** ([ef7c2ef](https://github.com/solisoft/soli_lang/commit/ef7c2ef))
* **feat(uploads): model-level uploader DSL with auto-routed attachments** ([6102481](https://github.com/solisoft/soli_lang/commit/6102481))
* **feat(vm): support hash attributes in Class.new() and fix function body compilation** ([c128c23](https://github.com/solisoft/soli_lang/commit/c128c23))
* **feat(model): `Model.create` returns instance (not hash); `_errors` array on failure, `nil` on success** — replaces the old `{ valid: bool, record: instance }` and `{ valid: bool, errors: [] }` hash returns with a single consistent shape. Use `if instance._errors` to distinguish success from failure.
* **feat(model): `Model.find` raises `RecordNotFound` when id is missing** — the HTTP layer auto-converts this to a 404 response. Use `Model.find_by` or wrap in `try/catch` for optional lookups.
* **feat(repl): display the result of `@sdql{ ... }` expressions** — the REPL now wraps `@sdql{...}` blocks in `println(_.inspect)` so their result is visible instead of silently discarded. ([1454b22](https://github.com/solisoft/soli_lang/commit/1454b22))
* **feat(template): bind `locals` hash to every partial context (Rails-style `local_assigns`)** — partials can now read keys whose names collide with reserved words (`class`) or builtin functions (`type`) via `locals["class"]` / `locals["type"]`. Bare-identifier access is unchanged and remains the idiom for non-reserved keys; `locals` is the escape hatch.
* **feat(serve): conditional-GET revalidation on `render()` HTML responses** — `html_response` now emits `ETag: "<fnv1a-64-hex>"` and `Cache-Control: private, no-cache`; requests carrying `If-None-Match` get a `304 Not Modified` short-circuit at the response boundary. Makes the shipped hover-prefetch feature actually deliver "instant navigation": the prefetched body is reused on click, validated by a tiny round-trip instead of a full re-render. Controllers can override per-response by setting their own `Cache-Control` / `ETag` in the `headers` hash.
* **feat(model): `instance.save(hash?)` and `instance.update(hash?)` accept bulk-attribute hash** — optional hash argument is merged onto the instance before validations + DB write, so `user.save({ "name": "Alice", "role": "admin" })` replaces N assignments + a save call. Hash wins on conflict, keys you don't pass keep their current value, framework-internal `_`-fields are silently skipped. Zero-arg callers keep working unchanged.

### Tests
* **test: improved error formatting with box-drawing characters** ([41c14a6](https://github.com/solisoft/soli_lang/commit/41c14a6))
* **test: added controller_spec tests for respond_to content negotiation** ([82c61ab](https://github.com/solisoft/soli_lang/commit/82c61ab))
* **test: auto-display coverage when tests pass** ([9550941](https://github.com/solisoft/soli_lang/commit/9550941))

### Bug Fixes
* **fix(model): fix description** — ([commithash](link))

## [0.80.1](https://github.com/solisoft/soli_lang/compare/0.80.0...0.80.1) (2026-04-23)

### Other
* **chore: release v0.80.1** ([92f653e](https://github.com/solisoft/soli_lang/commit/92f653e37473315226eeb25c8414b0cf5c958f4f))
* **chore: bump version to v0.80.1** ([9a2cdf7](https://github.com/solisoft/soli_lang/commit/9a2cdf7cdd000b300e75536eba3e2d31ba8987b1))

## [0.80.0](https://github.com/solisoft/soli_lang/compare/0.79.1...0.80.0) (2026-04-23)

### Bug Fixes
* **fix(template): route paren-form render(...) through the core parser** ([06508fe](https://github.com/solisoft/soli_lang/commit/06508fe1c12f93ef3f306a96067c1c23440cc137))

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
