# Testing MVC Applications

Soli provides a comprehensive testing framework for MVC applications with BDD-style DSL, parallel execution, and coverage reporting.

## Test Structure

Tests live in the `tests/` directory of your application:

```
myapp/
├── app/
│   ├── controllers/
│   ├── models/
│   └── views/
└── tests/
    ├── users_spec.sl
    ├── posts_spec.sl
    └── integration/
        └── api_spec.sl
```

Every spec file, whatever its name, starts with the app's `app/models`,
`policies`, `services`, `helpers`, `middleware` and `jobs` loaded. Job classes
get the same facade as in the server (`perform_later`, `perform_now`,
`perform_in`…), and `HTTP.*` calls block as they do in a request, so a `try`
or `rescue` around one catches its failure.

## Test DSL

### Basic Test Structure

```soli
describe("UsersController", fn()
  test("creates a new user", fn()
    # Test code here
    expect(true).to_be(true)
  end)
  
  context("when valid", fn()
    test("returns success", fn()
      # Nested context
    end)
  end)
end)
```

### Available Functions

| Function | Purpose |
|----------|---------|
| `describe(name, fn)` | Group related tests |
| `context(name, fn)` | Group tests with conditions |
| `test(name, fn)` | Define a test case |
| `it(name, fn)` | Alias for test |
| `specify(name, fn)` | Alias for test |
| `before_each(fn)` | Setup before each test |
| `after_each(fn)` | Teardown after each test |
| `before_all(fn)` | Setup before all tests |
| `after_all(fn)` | Teardown after all tests |
| `pending(reason?)` / `skip(reason?)` | Stop a test and count it as pending — not a failure |

Write hooks and `assert_raises` with parentheses before `do`: `before_each() do … end`.
A bare `before_each do` does not parse.

### Hooks

- **Nested suites inherit hooks.** Every enclosing `before_each` runs, outermost
  first; `after_each` hooks run innermost first. One hook of each kind per `describe`.
- **A raising hook fails the test, it is never swallowed.** A `before_each` that raises
  fails the test with `before_each: <error>` and the body does not run. A raising
  `after_each` fails the test (`after_each: …`). Teardown hooks always run.
- A raising `before_all` fails the suite — `<suite> (before_all, N test(s) not run)` —
  without running its tests; a raising `after_all` is reported as a failure.
- `skip("why")` or `pending("why")` in a `before_each` marks each test pending: this is
  how a suite skips itself (see [Specs that need a service](#specs-that-need-a-service)).

```soli
cart = []                      # shared state lives at the top level

describe("Cart") do
  before_each() do
    cart = ["book"]
  end

  describe("with a gift") do
    before_each() do           # runs after the outer one
      cart.push("wrapping")
    end

    test("holds both items") do
      assert_eq(cart, ["book", "wrapping"])
    end
  end

  test("starts with the book only") do
    assert_eq(cart, ["book"])
  end
end
```

### How the runner reads a spec

The runner reads the spec tree from the file **without running the `describe`
bodies**. So a variable assigned in a `describe` body never reaches its tests — only
top-level assignments and `def`s do; put shared state at the top level and (re)assign
it in a `before_each`. Whatever the runner cannot register fails the file, one line
per problem, rather than being silently dropped:

```
posts_spec.sl                                  0 ✗
┌─ 2 spec declaration problem(s):
- line 5: `describe` inside an `if`, loop or `try` is never registered
- line 17: a `describe` body is never executed, so this statement never runs
```

- a `describe`/`context`/`test`/`it`/`specify` inside a top-level `if`, `unless`, loop
  or `try` — the old `if db_available … describe(…) … end` guard ran nothing;
- a `test` outside any `describe`;
- any statement in a `describe` body other than `test`/`it`/`specify`,
  `describe`/`context`, the four hooks and `viewport` (`url = "/posts"` there never ran);
- a suite or test name that is not a string literal (interpolated or computed);
- a second `before_each` (or other hook) in the same `describe`.

### Expectations

```soli
expect(value).to_equal(expected);
expect(value).to_be(expected);
expect(value).to_not_equal(other);
expect(value).to_be_null();
expect(value).to_not_be_null();
expect(value).to_be_greater_than(10);     # an Int and a Float compare mixed
expect(value).to_be_less_than(100);
expect(value).to_contain("substring");
expect(value).to_match("Saved");          # substring check, NOT a regex
expect(hash).to_have_key("name");
expect(json_string).to_be_valid_json();
```

For a regex use `assert_match(string, pattern)`.

### Assertions

`assert_eq`, `assert_null`, `assert_match` and the rest are builtins — see
[Testing Assertions](testing-assertions.md). A failure shows the values
(`expected 3, got 2`), and every `assert_*` takes an optional trailing message
prefixed to it: `assert_eq(count, 3, "the cart size")` fails with
`the cart size: expected 3, got 2`.

`assert_raises` checks that a block raises, and returns the error message:

```soli
message = assert_raises("out of stock") do
  cart.add(sold_out_item)
end
assert_contains(message, sold_out_item.sku)
```

## HTTP Integration Testing

### Making Requests

```soli
describe("Users API", fn()
  test("GET /users returns list", fn()
    response = TestHTTP.get("/users")
    expect(response.status).to_equal(200)
    expect(response.body).to_contain("users")
  end)
  
  test("POST /users creates user", fn()
    response = TestHTTP.post("/users", hash(
      "email": "test@example.com",
      "name": "Test User"
    ))
    expect(response.status).to_equal(201)
  end)
  
  test("PUT /users/:id updates user", fn()
    response = TestHTTP.put("/users/1", hash("name": "Updated"))
    expect(response.status).to_equal(200)
  end)
  
  test("DELETE /users/:id removes user", fn()
    response = TestHTTP.delete("/users/1")
    expect(response.status).to_equal(204)
  end)
end)
```

### Request Options

```soli
TestHTTP.get("/users");
TestHTTP.get("/users", query: hash("page": "2"));
TestHTTP.post("/users", payload);
TestHTTP.post("/users", payload, headers: hash("Content-Type": "application/json"));
TestHTTP.put("/users/1", payload);
TestHTTP.patch("/users/1", payload);
TestHTTP.delete("/users/1");
```

## Controller Testing

### Direct Action Calls

```soli
describe("UsersController", fn()
  before_each(fn()
    Factory.clear
  end)
  
  test("create action", fn()
    result = ControllerTest.helpers.users_controller.create(
      params: hash("email": "test@example.com"),
      session: Session.new(),
      headers: Headers.new()
    )
    expect(result.status).to_equal(201)
  end)
  
  test("show action", fn()
    user = Factory.create("user")
    result = ControllerTest.helpers.users_controller.show(
      params: hash("id": user.id),
      session: Session.new(),
      headers: Headers.new()
    )
    expect(result.status).to_equal(200)
  end)
end)
```

## Database Testing

### Transaction Rollback

The test runner **drops** its worker databases when a suite finishes, so a machine
running many projects doesn't accumulate one empty `*_spec` database per worker per
app. Set `SOLI_TEST_KEEP_DB=1` to keep them and truncate their collections instead.
Dropping is serialised server-side and the next run has to recreate the schema, which
costs little on a small app but grows with the collection count — use `SOLI_TEST_KEEP_DB=1`
when the tight test loop matters more than the leftovers.

The base test database is created with the app's schema: when `soli test` creates (or
recreates) it, it runs `db/migrations` against it and truncates any rows they seed, so
every collection exists before the first spec runs. A failed migration is reported and the
run continues; collections are then created on first write, as before. Because the migrations
also create the app's indexes, a spec that inserts a duplicate value into a uniquely indexed
field fails as it would in production — give such fixtures distinct values. Every other worker
database gets the base database's collections and indexes, so a spec sees the same schema
whichever worker runs it.

`SOLI_TEST_SOLIDB_HOST` sends test runs to a SoliDB of their own, overriding the
`SOLIDB_HOST` from `.env.test` for the runner and its test servers. Set it in the shell on a
workstation whose `localhost:6745` is the dev instance every `soli serve --dev` shares —
a suite that creates, truncates and drops databases there runs at its pace, not yours. CI
leaves it unset and keeps `.env.test`.

```bash
SOLI_TEST_SOLIDB_HOST=http://localhost:6746 soli test
```

For **per-example** isolation inside a spec, wrap DB writes in `with_transaction` — it begins a SolidB transaction, runs your block, then **always rolls back** (even when the block succeeds):

```soli
describe("User model") do
  test("creates user") do
    Factory.define("user", {"email": "tx@test.com", "name": "Test"})
    Factory.bind("user", User)

    with_transaction(fn() {
      user = Factory.insert("user")
      assert(user._key.present?)
      assert_eq(user.name, "Test")
    })
    # Rolled back — nothing persisted
    assert_eq(User.count, 0)
  end
end
```

Assert on the records the block creates, not on reads: only the writes join
the transaction. A query inside the block (`User.count`, `User.all`,
`find`) reads committed data and does not see the block's own inserts —
`User.count` there is still `0`.

Unlike `Model.transaction { }`, which commits on success, `with_transaction` is test-only and never commits.

### Specs that need a service

Call `requires_solidb()` or `requires_solikv()` from a `before_each`. When the service
does not answer, each test is skipped (`needs SoliDB: …`) and counted as pending, rather
than failing on a connection error:

```soli
describe("PriceCache") do
  before_each() do
    requires_solikv()
  end

  test("keeps a price") do
    PriceCache.store("SKU-1", 1200)
    assert_eq(PriceCache.fetch("SKU-1"), 1200)
  end
end
```

- SoliDB is probed with a health check of the models' `SOLIDB_HOST`; SoliKV with a
  `PING` to `SOLIKV_RESP_HOST` / `SOLIKV_RESP_PORT`.
- `solidb_available?()` / `solikv_available?()` return the same answer as a Bool.
- Under `SOLI_REQUIRE_DB=1` — what CI should set — the skip becomes a failure
  (`SOLI_REQUIRE_DB=1 but this test needs SoliKV: …`), so a misconfigured CI cannot
  pass by skipping.

This replaces probing with `Model.create` in a top-level `try` and `return`-ing early
from each test, which made tests pass while testing nothing.

### Time Travel

Pin `datetime_now()` for cron, TTL, and expiration specs:

```soli
freeze_time(1_700_000_000)          # int timestamp
travel_to("2024-06-15")             # alias — parses date strings too
assert_eq(datetime_now(), 1_715_212_800)
unfreeze_time()                     # also cleared automatically before each test
```

### Factory Pattern

`#{n}` in a template string becomes a per-factory counter on each create — inside a
**raw string** only (`r"…"`). In a normal string Soli interpolates `#{n}` itself first
and fails with `Undefined variable 'n'`.

```soli
# Static template
Factory.define("user", {
  "email": r"user#{n}@example.com",
  "name": "Test User"
})

# Callable template (fresh data every create)
Factory.define("post", fn() {
  {"title": "Post #{Factory.sequence("post")}"}
})

# Build hashes (no DB)
user = Factory.create("user")
post = Factory.create_with("post", {"title": "Custom Title"})
users = Factory.create_list("user", 5)

# Persist through a model
Factory.bind("user", User)
persisted = Factory.insert("user")
```

## Parallel Execution

The default is **3 workers** when the app has `app/controllers` (request specs spend
their time waiting on test servers) and **1** otherwise, capped at the number of spec
files. `--jobs N` overrides it:

```bash
soli test                    # 3 workers with app/controllers, else 1
soli test --jobs=4           # 4 workers
soli test --jobs=1           # Sequential (debug)
```

## Filtering and Fail-Fast

```bash
soli test --filter "creates a post"   # only tests whose full name contains the text
soli test -n checkout                 # short form; matches describe names too
soli test --fail-fast                 # stop scheduling tests after the first failure
soli test --require-assertions        # fail any test that makes no assertion
soli test --watch                     # rerun the suite whenever a .sl / .slv file changes
```

`--filter` matches case-insensitively against the full description: the enclosing
`describe` names followed by the test name, space-joined (`Posts creates a post`).
Files with no matching test still load, but run nothing. With `--fail-fast`, the
remaining tests and files are skipped once one test fails, so the report lists the
failure without a wall of follow-on noise. `--require-assertions` fails a test that
passes without asserting anything (`made no assertions`); without it the summary only
counts them (`3 tests made no assertions (--require-assertions fails them)`). A mock's
`assert_received` / `assert_not_received` count as assertions; a test with nothing to
check should call `skip("why")`. `--watch` (`-w`) runs the suite, then polls `app/`, `config/`, `lib/`, `db/`, `tests/` and any path you passed, and reruns it in a fresh process on every change; the other flags are passed through. Both compose with `--jobs`, `--coverage`
and the rest.

## Coverage Reporting

Coverage is **on by default** — every `soli test` prints the console report.
`--no-coverage` turns it off for a faster loop.

```bash
soli test --no-coverage              # Skip coverage
soli test --coverage                 # Generate coverage
soli test --coverage=html            # HTML report
soli test --coverage=json            # JSON for CI
soli test --coverage=xml              # Cobertura XML
soli test --coverage-min=80          # Fail if < 80%
```

### Coverage Features

- **Tests excluded**: The `tests/` directory is automatically excluded from coverage reports
- **Relative paths**: Coverage reports display relative paths for easier reading
- **Global tracker**: HTTP request coverage tracking via global coverage tracker for test server

### Coverage Output

```
Coverage: 87.5% (1250/1428 lines) ✓

src/controllers/users.sl     ▓▓▓▓▓▓▓▓▓▓▓▓▓▓░░░░░░░░  94.2%
src/models/user.sl           ▓▓▓▓▓▓▓▓▓▓▓▓▓░░░░░░░░  91.1%
src/controllers/posts.sl     ▓▓▓▓▓▓░░░░░░░░░░░░░░░  78.5%
```

### Coverage Configuration

```soli
coverage_threshold(80)           # Fail if < 80%
coverage_exclude("**/migrations/**")
```

## Complete Example

```soli
describe("UsersController", fn()
  before_each(fn()
    Factory.clear
    Database.clean_all()
  end)
  
  context("POST /users", fn()
    test("creates user with valid data", fn()
      response = TestHTTP.post("/users", hash(
        "email": "user@example.com",
        "name": "Test User"
      ))
      expect(response.status).to_equal(201)
      expect(response.body).to_contain("Test User")
    end)
    
    test("returns 422 with invalid email", fn()
      response = TestHTTP.post("/users", hash(
        "email": "invalid-email"
      ))
      expect(response.status).to_equal(422)
    end)
    
    test("returns 422 without email", fn()
      response = TestHTTP.post("/users", hash(
        "name": "Test"
      ))
      expect(response.status).to_equal(422)
    end)
  end)
  
  context("GET /users/:id", fn()
    test("shows user profile", fn()
      user = Factory.create("user")
      response = TestHTTP.get("/users/" + user.id)
      expect(response.status).to_equal(200)
      expect(response.body).to_contain(user.name)
    end)
    
    test("returns 404 for unknown user", fn()
      response = TestHTTP.get("/users/99999")
      expect(response.status).to_equal(404)
    end)
  end)
  
  context("DELETE /users/:id", fn()
    test("removes user", fn()
      user = Factory.create("user")
      response = TestHTTP.delete("/users/" + user.id)
      expect(response.status).to_equal(204)
      expect(User.find(user.id)).to_be_null()
    end)
  end)
end)
```

## Running Tests

```bash
# Run all tests
soli test

# Run specific file
soli test tests/users_spec.sl

# Run with coverage
soli test --coverage

# Sequential execution
soli test --jobs=1

# JSON output for CI
soli test --reporter=json
```

### Password hashing under test

`soli test` sets `SOLI_ARGON2_FAST=1` for itself and for the servers it
starts. New password hashes are then made at 4 MiB and one pass instead of
the RFC 9106 default of 19 MiB and two — roughly 2 ms instead of 20.

A suite pays the full price twice per authenticated test: once creating the
fixture user, once verifying at login. On one real application — 1 013
logins and as many fixture users — that was 38 of the 106 seconds the run
took, and the average login went from 62 ms to 29 ms.

Two things keep this from reaching anything real:

- It changes **hashing only**. `Crypto.argon2_verify` reads the cost from the
  stored hash (Argon2 records `m`, `t` and `p` in the PHC string it returns),
  so a password hashed in production keeps its full cost however the variable
  is set, and one database may hold both kinds.
- Only `soli test` sets it. It is read once, and only `1` or `true` count —
  an empty `SOLI_ARGON2_FAST=` is off.

Never set it on a machine that stores real passwords: a hash made under it
carries its weakness for as long as it is stored.

## Test Doubles: `Mock`

`Mock` is a hand-rolled double for the collaborator your code takes as an argument.
It is defined in the test environment only (`soli test`), so it never shadows an app
class in production.

```soli
test("charges through the gateway") do
  gateway = new Mock("gateway", {
    "charge": fn(amount) { {"ok": true, "amount": amount} },
    "currency": "EUR"
  })

  result = Checkout.new(gateway).pay(1200)

  assert(result["ok"])
  gateway.assert_received("charge", [1200])
  gateway.assert_not_received("refund")
end
```

- Stubs are a hash of method name to a value, or to a lambda called with the arguments (up to four).
- `stub(name, value)` adds one later and returns the double.
- Every call is recorded: `calls`, `calls_to(name)`, `call_count(name)`, `received?(name)`, `reset_calls`.
- `assert_received(name)` / `assert_received(name, args)` and `assert_not_received(name)` throw a readable message on failure.
- A message with no stub throws (`gateway received unexpected message refund`), so a typo fails loudly.
- Call a double's methods **with parentheses** (`gateway.currency()`): a bare `gateway.currency` yields the bound method, not its result.

### Stubbing a real class

`Mock.stub_class` and `Mock.stub_instance` replace one method on a class other code
already references, for the rest of the current test:

```soli
test("checkout charges through the gateway") do
  charge = Mock.stub_class(Gateway, "charge", fn(amount) { {"ok": true} })

  assert(Checkout.new.pay(1200)["ok"])       # Checkout calls Gateway.charge(...)
  charge.assert_received("charge", [1200])

  Mock.stub_instance(User, "save", true)     # every User#save answers true
end
```

- The stub is a `Mock`, so it records calls and supports `assert_received`. The handler is a value or a lambda.
- It is undone automatically when the test ends (`Mock.unstub_all` does it early). A stub set in `before_all` lasts the whole `describe` (nested ones included) and is undone after its `after_all`; one set in a test or `before_each` ends with that test.
- Works on model finders (`Post.find`, `Post.all`) and instance methods, including native ones such as `save`. A stubbed `save` / `update` / `create` answers from the stub even when the model has callbacks; a spy on one still runs the callbacks. Only the stubbed method changes; the rest of the class is untouched.
- It patches the code that runs in the test process. A request spec's app runs in the test *server*, a separate process, so stub there with a dependency you pass in.
- Don't stub a name `Mock` itself defines (`stub`, `calls`, `received?`, ...).

#### RSpec-shaped chains and spies

```soli
Mock.allow(Gateway).to_receive("charge").and_return({"ok": true})
Mock.allow(Gateway).to_receive("charge").and_call(fn(amount) { {"ok": amount > 0} })
spy = Mock.allow(Gateway).to_receive("charge").and_call_original
Mock.allow_any_instance(User).to_receive("save").and_return(true)
```

`and_call_original` makes a **spy**: the call is recorded on the returned `Mock` and the
real method still runs, so `spy.assert_received("charge", [1200])` checks what happened
without changing it (`Mock.spy_class(Gateway, "charge")` / `Mock.spy_instance(...)` are the
same thing spelled directly). Every chain ends by returning the `Mock`. A stub matches any
arguments; there is no `with(...)` filter, so branch inside an `and_call` lambda when a
method must answer differently per argument.

For the database and HTTP use the sections below.

## Mock Database Queries

For integration tests that don't need a real database, use `Model.mock_query_result()` to intercept queries and return predefined data.

### Registering Mocks

```soli
describe("User queries", fn()
  before_each(fn()
    User.clear_mocks()
  end)
  
  after_each(fn()
    User.clear_mocks()
  end)
  
  test("finds user by id", fn()
    User.mock_query_result(
      "FOR doc IN users FILTER doc._key == @key RETURN doc",
      [
        {
          "_key": "123",
          "_id": "default:users/123",
          "name": "Alice",
          "email": "alice@example.com"
        }
      ]
    )
    
    user = User.find("123")
    expect(user.name).to_equal("Alice")
    expect(user.class_name).to_equal("User")
  end)
end)
```

### Mocking Include Queries

When testing relations with `includes()`, mock both the parent and related queries:

```soli
describe("Contact with organisation", fn()
  before_each(fn()
    Contact.clear_mocks()
    Organisation.clear_mocks()
  end)
  
  test("returns correct class for included relations", fn()
    # Mock Contact query
    Contact.mock_query_result(
      "FOR doc IN contacts RETURN doc",
      [
        {
          "_key": "c1",
          "_id": "default:contacts/c1",
          "name": "Bob",
          "organisation_id": "default:organisations/o1"
        }
      ]
    )
    
    # Mock Organisation query (for includes)
    Organisation.mock_query_result(
      "FOR doc IN organisations FILTER doc._key IN @keys RETURN doc",
      [
        {
          "_key": "o1",
          "_id": "default:organisations/o1",
          "name": "Acme Corp"
        }
      ]
    )
    
    contact = Contact.includes("organisation").first
    org = contact.organisation
    
    # Verify the relation has the correct class
    expect(org.class_name).to_equal("Organisation")
    expect(org.name).to_equal("Acme Corp")
  end)
end)
```

### How It Works

The mock system intercepts queries at the database layer before HTTP calls are made:

1. `Model.mock_query_result(query, results)` registers mock data for a specific AQL query
2. `Model.clear_mocks()` removes all registered mocks
3. Mocks are stored per-model using thread-local storage

### Mock Query Format

The query string should match the AQL being generated. You can find the exact query by:
- Looking at generated queries in tests
- Using logging to capture queries
- Inspecting the model's query builder output

```soli
# Common query patterns
User.mock_query_result("FOR doc IN users RETURN doc", [...])
User.mock_query_result("FOR doc IN users FILTER doc._key == @key RETURN doc", [...])
Post.mock_query_result("FOR doc IN posts SORT doc.created_at DESC LIMIT 10 RETURN doc", [...])
```

### Best Practices

```soli
describe("Model Integration Tests", fn()
  before_each(fn()
    # Clear mocks before each test to avoid cross-test contamination
    Model.clear_mocks()
  end)
  
  after_each(fn()
    # Ensure clean state
    Model.clear_mocks()
  end)
  
  test("specific scenario", fn()
    # Register only the mocks needed for this test
    Model.mock_query_result("FOR doc IN model RETURN doc", [...])
  end)
end)
```

## Mock HTTP Services

When the code under test calls **another** service — a payment API, an
OpenID provider, a webhook receiver — start the local mock server and script
what it answers:

```soli
let port = mock_http_server_start()
let base = "http://127.0.0.1:" + port.to_s

# Any method on this path answers 200 with this body. The query string is
# ignored; scripting the same path again replaces the answer.
mock_http_route("/idp/.well-known/openid-configuration", 200, json_stringify({
  "issuer": base + "/idp",
  "token_endpoint": base + "/idp/token",
  "jwks_uri": base + "/idp/keys"
}))
mock_http_route("/idp/token", 400, "{\"error\":\"invalid_grant\"}")

# ... drive the app ...

# What the app SENT is often the thing to prove.
let sent = mock_http_last_body("/idp/token")
assert(sent.includes?("code_verifier="))
```

- The server is a real socket on `127.0.0.1`, so the app's **test server** —
  a separate process — reaches it exactly as it would reach the real service.
  Point the app at `base` through whatever configuration it reads.
- A path that was never scripted answers `200` with `{"ok":true}`.
- `mock_http_last_body(path)` returns the body of the last request on that
  path (up to 64 KiB), or `nil` if none arrived.
- Routes are shared by the whole spec file. Give each test its own path
  prefix so one test's answers never reply in another's place.
- Test-only: none of the three exists in a served app.

## Test Results

While the suite runs, each worker gets a row and the aggregate bar at the bottom
carries the running totals. A row names its spec by its path under the tested
directory, and a narrow terminal shortens the folders before the filename:

```
 W0  [████████░░░░░░] ⠇ models/users_spec                2.4s  6
 W1  [██████░░░░░░░░] ⠹ controllers/buildings/pages_spec 10.0s  4

[██████████░░░░░░░░░░░░░░░░░░░░] ⠇ 41/158 1 204 tests · 6 018 assertions
```

The three counters advance at different rates on purpose. `41/158` is **files**,
and only moves when one finishes. `tests` counts each `test(...)` block as it
ends, and `assertions` counts every assertion as it fires — both are live, so a
spec file that runs for twenty seconds visibly contributes while it is still
running rather than landing all at once at the end. A failing test is called out
in red on the bar as soon as it fails, before its file has finished:

```
[██████████░░░░░░░░░░░░░░░░░░░░] ⠇ 41/158 1 204 tests 2 failed · 6 018 assertions
```

The summary repeats the three, and says which unit each line counts:

```
❌
  156 files passed, 2 failed (158 total)
  1 204 tests, 2 failed
  6 018 assertions
  Time: 41.2s

Coverage: 87.5% (1250/1428 lines) ✓
```

A file that panics outright (rather than failing an assertion) loses its own
assertion count, so the summary reports what the per-file tally saw. The test
count has no second source — it is counted as each block ends, so the tests a
panicking file ran before it died are still there.
