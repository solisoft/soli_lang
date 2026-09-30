# Tests

This directory holds **BDD-style specs** that run with `soli test`. Every
behavior worth shipping has a spec; the coverage gate is non-negotiable.

File pattern: `*_spec.sl`. Controller specs run as full end-to-end HTTP
requests against a real worker; model and helper specs exercise objects
directly.

## File layout

```
tests/
├── posts_controller_spec.sl     # E2E spec for PostsController
├── post_model_spec.sl           # Unit spec for the Post model
├── authenticate_middleware_spec.sl  # Middleware tested via E2E
└── helpers/
    └── markdown_helper_spec.sl  # Helper unit spec
```

One spec file per controller / model / helper. Match the filename to the
target.

## The DSL

```soli
describe("PostsController") do
  before_each() do
    as_guest()
  end

  describe("GET /posts") do
    test("returns a list of posts") do
      response = get("/posts")
      assert_eq(res_status(response), 200)
      assert_hash_has_key(assigns(), "posts")
    end
  end

  describe("POST /posts") do
    test("creates a post with valid params") do
      as_user(1)
      response = post("/posts", { "title": "Hi", "body": "..." })
      assert_eq(res_status(response), 302)
    end

    test("re-renders new on validation failure") do
      as_user(1)
      response = post("/posts", { "title": "" })
      assert_eq(res_status(response), 200)
      assert_eq(view_path(), "posts/new.html")
    end
  end
end
```

Keywords:

| Keyword                         | What it does                                              |
|---------------------------------|-----------------------------------------------------------|
| `describe(name) do ... end`     | Suite. Can nest.                                          |
| `context(name) do ... end`      | Alias for `describe` — use it when reading flows better.   |
| `test(name) do ... end`         | A single case.                                            |
| `it(name) do ... end`           | Alias for `test`.                                         |
| `specify(name) do ... end`      | Also an alias for `test`.                                 |
| `before_each() do ... end`      | Runs before every `test` inside the current suite.        |
| `after_each() do ... end`       | Runs after every `test`.                                  |
| `before_all() do ... end`       | Runs once before any tests in the suite.                  |
| `after_all() do ... end`        | Runs once after all tests in the suite.                   |
| `pending()`                     | Marks the current test as pending (not run, not failing). |
| `skip()`                        | Marks the current test as skipped.                        |

## Assertions

Pick the assertion that gives the **best failure message**.
`assert_eq(status, 200)` reads better than `assert(status == 200)` and the
runner shows the actual + expected on failure.

```soli
assert(cond)                       # truthy
assert_not(cond)                   # falsy
assert_eq(actual, expected)        # value equality
assert_ne(actual, expected)        # inequality
assert_null(v)                     # v is nil
assert_not_null(v)
assert_gt(a, b)                    # numeric: a > b
assert_lt(a, b)                    # numeric: a < b
assert_match(string, regex)        # regex match
assert_contains(arr_or_str, item)  # array/string containment
assert_hash_has_key(hash, key)
assert_json(string)                # parses as valid JSON
```

### `expect(...)` DSL

For people who prefer reading left-to-right:

```soli
expect(user.name).to_equal("Alice")
expect(user.posts).to_contain(post)
expect(user.errors).to_be_null()
expect(post.views).to_be_greater_than(0)
expect(response_body).to_be_valid_json()
expect(html).to_match("welcome")
```

Available matchers: `to_be`, `to_equal`, `to_not_be`, `to_not_equal`,
`to_be_null`, `to_not_be_null`, `to_be_greater_than`, `to_be_less_than`,
`to_be_greater_than_or_equal`, `to_be_less_than_or_equal`, `to_contain`,
`to_match`, `to_be_valid_json`. Either style is fine — pick one per file and
stick with it.

## E2E controller helpers

A controller spec gets a full HTTP client. The request runs through the real
middleware stack, real controllers, real views, real DB.

### Making requests

```soli
get(path)
get(path, { "headers": {...} })            # with custom headers
post(path, body)                            # body is a hash (form-style) or a string
put(path, body)
patch(path, body)
delete(path)
head(path)
options(path)
request(method, path, body, options)        # generic — when the named helpers don't fit
```

### Inspecting the response

```soli
res_status(response)         # 200, 302, 404, ...
res_body(response)           # raw response body (string)
res_json(response)           # parsed JSON body
res_header(response, "Location")
res_headers(response)        # all headers as a hash
res_redirect?(response)      # boolean — is 3xx?
res_location(response)       # Location header
res_ok?(response)            # 2xx
res_client_error?(response)  # 4xx
res_server_error?(response)  # 5xx
res_not_found?(response)     # 404
res_unauthorized?(response)  # 401
res_forbidden?(response)     # 403
res_unprocessable?(response) # 422
```

### Inspecting the view

```soli
assigns()                       # hash of @field values exposed to the view
assign("post")                  # single field
view_path()                     # e.g. "posts/show.html"
render_template?()              # whether a template was rendered (false in pure JSON paths)
```

`assigns()` is the cleanest way to verify a controller did the right setup
work without coupling to the rendered HTML.

### Auth & session

```soli
as_guest()                          # clear all auth (default state for before_each)
as_user(user_id)                    # log in as a given user
as_user(user_id, { "role": "..." }) # user + session opts (writes to server-side store)
as_role("admin")                    # first user with role == "admin" (single-table apps)
sign_in("admin", id)                # separate-collection auth: writes session.admin_id
sign_in("admin")                    # same, but uses Admin.first
as_admin()                          # convenience: log in as user 1
login(email, password)              # run the real login flow
logout()
current_user()                      # the logged-in user, or nil
signed_in?()
with_token("abc...")                # set a Bearer token header for the next request
with_session({ "foo": "bar" })      # forge arbitrary session keys (gated by SEC-040)
```

Pick the helper that matches your auth shape:

- **One `users` collection with a role string field** → `as_role("admin")` or `as_user(id, { "role": "admin" })`.
- **Distinct `User` / `Admin` models with separate session keys** → `sign_in("admin", id)` / `sign_in("admin")`.
- **Just need a logged-in user, no role checks** → `as_user(id)`.
- **Non-conventional session keys, arbitrary seeding** → `with_session({ ... })`.

### Request modifiers

```soli
set_header("X-Foo", "bar")    # add a header to all subsequent requests
set_authorization("Bearer ...")
clear_authorization()
set_request_cookie("name", "value")
clear_cookies()
clear_headers()
```

Header/auth state persists across requests in the same `test` block. Use
`before_each` to reset:

```soli
before_each() do
  as_guest()
  clear_headers()
  clear_cookies()
end
```

## Testing a model in isolation

Models don't need the HTTP machinery — call them directly:

```soli
describe("Post") do
  test("rejects empty title") do
    post = Post.new({ "title": "", "body": "..." })
    post.save
    assert(post._errors)
    assert_eq(post._errors[0].field, "title")
  end

  test("normalize_title trims whitespace before save") do
    post = Post.create({ "title": "  hello  ", "body": "x" })
    assert_eq(post.title, "hello")
  end

  test("recent scope orders by created_at desc") do
    Post.create({ "title": "old", "body": "x", "created_at": "2024-01-01" })
    Post.create({ "title": "new", "body": "x", "created_at": "2026-01-01" })
    recent = Post.recent.all
    assert_eq(recent[0].title, "new")
  end
end
```

Hit the **real database** in model specs. Mocks let bugs through — the whole
point of the model layer is the round-trip with the DB.

## Coverage

```bash
soli test --coverage                      # generate a console report
soli test --coverage --coverage-min 90.0  # fail if total line coverage is < 90%
soli test --coverage=html                 # also write an HTML report
soli test --coverage=json,xml             # multiple report formats
```

- Without `--coverage`, no coverage is collected at all.
- With `--coverage` but no explicit `--coverage-min`, the default threshold
  is `80.0`.
- The **project policy** is 90% (see top-level scaffold `CLAUDE.md`).
- Don't lower `--coverage-min` to ship — write the missing test.

## Selecting which specs to run

```bash
soli test                                 # all specs in tests/
soli test tests/posts_controller_spec.sl  # one file
soli test tests/controllers/              # one directory
soli test --jobs 4                        # parallelism (see below)
```

There's no `--only` / `--focus` / `--grep` filter today — narrow by path.
Inside a file, use `pending()` / `skip()` to disable a single test, or
comment out a whole `describe` block.

## Parallelism

`--jobs N` runs N workers in parallel.

- Default: **3** if `app/controllers/` exists; **1** otherwise.
- Each worker gets its own database (`<base>_w<i>_<suffix>`). Worker isolation
  is automatic — you don't need to reset between specs to avoid bleed across
  workers, just within them.
- Cap is the number of spec files.

## Test database lifecycle

- Before the suite starts, each worker's database is **dropped and recreated**.
- Migrations are **not** auto-run by the test runner. Either:
  - Run them yourself: `before_all() do db_migrate("up") end`, or
  - Let the test helper crate handle it if your project sets one up.
- Between tests, state from earlier specs in the same worker may persist.
  Use `before_each` to set up known starting state — don't rely on alphabetic
  ordering between tests.

## Running the verification loop

Before reporting a feature done:

```bash
soli lint <files-you-changed>             # 1. style/smell rules
soli test tests/<relevant_spec>.sl        # 2. narrow, fast feedback
soli test --coverage --coverage-min 90.0  # 3. full sweep + gate
```

If a UI changed, also start the app and exercise the page in a browser.

## Do / Don't

| Do                                                            | Don't                                                              |
|---------------------------------------------------------------|--------------------------------------------------------------------|
| Write a new spec for every new behavior                       | Add code without a failing test first                               |
| Hit the real DB in model specs                                | Mock the DB just to make a model spec faster                        |
| Use `assert_eq` / `expect(...).to_equal(...)` for clarity     | Use `assert(a == b)` — failure messages are worse                   |
| Reset auth/session/headers in `before_each`                   | Assume the previous spec cleaned up                                 |
| Inspect `assigns()` to verify controller behavior             | Grep the rendered HTML for `<h1>` text                              |
| Use `as_user(id)` to set up an authenticated session           | Reimplement login by setting cookies by hand                        |
| Use `#{expr}` for string interpolation                        | Use `\(expr)` — the lexer rejects it                                |
| Use `pending()` for a known-flaky test you're investigating   | Leave a `#` skipped block with no tracking                          |
| Pick one of `test` / `it` / `specify` per file and stick to it | Mix all three in one file — readers shouldn't have to think         |
