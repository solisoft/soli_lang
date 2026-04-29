# Changelog

## [Unreleased]

### Bug Fixes

* **fix(i18n):** correct `I18n.format_currency` carry bug — rounding to total cents first prevents `9.995` from formatting as `"9,100 €"` instead of `"10,00 €"` ([bec9c30](https://github.com/solisoft/soli_lang/commit/bec9c30))

### Features

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

### Performance

* **perf(model):** dedupe validation rule registration ([aa66cd1](https://github.com/solisoft/soli_lang/commit/aa66cd1))
* **perf(test):** cut `--jobs N` startup overhead and balance work across workers

### Tests

* **test(http):** replace httpbin.org with in-process mock server — faster, non-flaky, works offline
* **test: improved error formatting with box-drawing characters** ([41c14a6](https://github.com/solisoft/soli_lang/commit/41c14a6))
* **test: added controller_spec tests for respond_to content negotiation** ([82c61ab](https://github.com/solisoft/soli_lang/commit/82c61ab))
* **test: auto-display coverage when tests pass** ([9550941](https://github.com/solisoft/soli_lang/commit/9550941))

### Documentation

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