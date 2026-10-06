# Testing Assertions

> **Markdown-only, deliberately.** No `.html.slv` counterpart: this is a reference table
> people read next to their code, and `www/docs/` ships inside every scaffolded
> application under `docs/`. See `www/docs/CLAUDE.md`.

Assertions are **builtins** of the test runner: they are defined by the runtime and
available inside any `test(...)` body run by `soli test`. Nothing to import, and no
`tests/helpers/assertions.sl` to write.

Every assertion **raises** on failure — the runner catches it, marks the test failed,
and prints a message that shows the values, with the file and line. On success an
assertion returns `1` and bumps the run's assertion counter (the count printed after
each file). There is no result hash to inspect.

Every `assert_*` below takes an **optional trailing message**, prefixed to the failure —
useful when the same check runs in a loop or the values alone do not say what broke.

```soli
describe("User", fn() {
    test("stores a normalized email", fn() {
        user = User.create({ "email": " A@Example.com " })
        assert_eq(user["email"], "a@example.com")
        assert_not_null(user["_key"])
    })
})
```

## Value Assertions

| Assertion | Passes when |
|---|---|
| `assert(value)` | `value` is the boolean `true` (anything else fails, naming its type) |
| `assert_not(value)` | `value` is the boolean `false` |
| `assert_eq(a, b)` | `a` and `b` are equal |
| `assert_ne(a, b)` | `a` and `b` differ |
| `assert_null(value)` | `value` is `null` |
| `assert_not_null(value)` | `value` is anything but `null` |
| `assert_gt(a, b)` | `a > b` (an Int and a Float compare mixed) |
| `assert_lt(a, b)` | `a < b` |
| `assert_match(string, pattern)` | the regex `pattern` matches `string` |
| `assert_contains(collection, item)` | an array contains `item`, or a string contains the substring |
| `assert_hash_has_key(hash, key)` | `hash` has that key |
| `assert_json(string)` | `string` parses as JSON |
| `assert_raises(fragment?) do … end` | the block raises (with the fragment in its message); returns the message |

`assert` and `assert_not` are strict about the boolean: `assert(user)` on a hash fails
(`assert expects a Bool, got hash (…)`) rather than passing, so a typo cannot quietly
succeed. Compare explicitly (`assert_not_null(user)`) instead.

```soli
assert(order["paid"])                          # a real boolean field
assert_eq(response["status"], 200)
assert_ne(user["_key"], other["_key"])
assert_gt(total, 0)
assert_match(slug, "^[a-z0-9-]+$")
assert_contains(["draft", "open"], order["state"])
assert_contains(response["body"], "Saved")     # substring on a string
assert_hash_has_key(payload, "token")
assert_json(response["body"])
```

### Failure messages

| Assertion | Fails with |
|---|---|
| `assert_eq(actual, expected)` | `expected <expected>, got <actual>` |
| `assert_ne(a, b)` | `expected a value other than <b>` |
| `assert_null(value)` | `expected nil, got <value>` |
| `assert_not_null(value)` | `expected a value, got nil` |
| `assert_not(value)` | `expected false, got <value>` |
| `assert_gt(a, b)` / `assert_lt(a, b)` | `expected <a> to be greater than <b>` / `… less than …` |
| `assert_match(string, pattern)` | `expected "<string>" to match /<pattern>/` |
| `assert_contains(collection, item)` | `expected <collection> to contain <item>` |
| `assert_hash_has_key(hash, key)` | `expected <hash> to have key "<key>"` |
| `assert_json(string)` | `expected valid JSON, got "<string>" (<parse error>)` |

With a message, it comes first:

```soli
assert_eq(cart.length, 3, "the cart size")   # the cart size: expected 3, got 2
assert(order["paid"], "order paid")          # order paid: assertion failed
```

### assert_raises

```soli
message = assert_raises("out of stock") do
    cart.add(sold_out_item)
end
assert_contains(message, sold_out_item.sku)

assert_raises() do
    divide(1, 0)
end

assert_raises("division by zero", fn() { divide(1, 0) })
```

Passes when the block raises — and, given a fragment, when the error message contains
it — and **returns the error message** so you can assert more on it. Fails when the block
raises nothing (`expected the block to raise, but it raised nothing`) or a different
error (`expected an error containing "timeout", got "boom"`). A failed assertion or a
`skip` / `pending` inside the block is not taken for the expected error: it propagates.

Keep the parentheses: a bare `assert_raises do … end` does not parse. It replaces the
hand-rolled pattern:

```soli
# Before
raised = false
try
    divide(1, 0)
catch error
    raised = true
end
assert(raised)
```

## Query Assertions

### assert_no_n_plus_one(response)

Asserts the request that produced `response` did not trigger an N+1 query
pattern (the same AQL template firing 2+ times in a loop). Uses the same
detection as the dev bar's N+1 badge. Pass the response from `get()` / `post()`.

```soli
response = get("/posts");
assert_no_n_plus_one(response);
```

To enforce this across every request spec without per-test calls, run
`soli test --fail-on-n1` — any response that triggers an N+1 fails its test
with the same message.

### assert_no_ungrouped_reads(response)

Asserts the request did not leave reads uncoalesced: three or more **distinct**
read templates, each run **once**, none inside a `grouped(fn() { ... })` block.

This is the complement of `assert_no_n_plus_one`, which fingerprints by template
and so only ever fires on a *repeated* one. Three unrelated reads are three
distinct templates with a count of one each, which no N+1 scan can see — yet that
is exactly the shape `grouped` exists for.

```soli
response = get("/dashboard");
assert_no_ungrouped_reads(response);
assert_query_count(response, 1);
```

The test server coalesces (unlike interactive `--dev`), so a grouped action
reports **one** query here — the same number production makes.

**Caveat worth knowing:** the runtime cannot prove the reads are independent.
`User.find(id)` followed by a query on `user._key` is genuinely two round-trips,
and this assertion will flag it. Use it on actions whose reads really are
unrelated; the failure message states the precondition so a false positive is
recognisable.

### assert_query_count(response, n) / assert_max_queries(response, n)

Assert the request ran exactly `n` (or at most `n`) AQL queries — a query
budget for the endpoint.

```soli
response = get("/dashboard");
assert_query_count(response, 3);   # exactly three
assert_max_queries(response, 5);   # at most five
```

See [E2E Controller Testing → Query Assertions](testing-e2e.md#query-assertions-n1-detection)
for details.

### Browser assertions

Available in browser specs (`soli test --browser`). Unlike the assertions above,
the positive ones **wait** for the condition — a browser round trip takes time,
and a spec should not have to guess how much.

```soli
assert_text("Saved")             # visible page text contains this
assert_no_text("Error")
assert_selector("#toast")        # element is present
assert_no_selector(".error")
assert_page_path("/posts/1")     # the browser's current path
assert_no_page_errors()          # no uncaught exception or console.error
```

Negative assertions do not wait: checking that something stays absent would slow
every passing test by the full timeout. Override the wait per call with
`assert_text("Ready", {"timeout": 30})` (seconds; default 10).

See [Browser Testing](testing-browser.md) for the full set of helpers.

## Expect API

Soli provides a chainable `expect()` API for more expressive assertions:

### expect(value)

Creates an expectation with the given value. Chain with `to_*()` methods:

```soli
expect(42).to_equal(42);
expect("hello").to_contain("ell");
expect(10).to_be_greater_than(5);
expect(user).to_not_be_null();
```

### to_be(expected)

Asserts that the actual value is the same as expected (identity check):

```soli
expect(42).to_be(42);
expect(true).to_be(true);
```

### to_equal(expected)

Asserts that the actual value equals expected (value equality):

```soli
expect(42).to_equal(42);
expect("hello").to_equal("hello");
```

### to_not_be(expected)

Asserts that the actual value is not the same as expected:

```soli
expect(42).to_not_be(43);
expect("hello").to_not_be("world");
```

### to_not_equal(expected)

Asserts that the actual value does not equal expected:

```soli
expect(42).to_not_equal(43);
expect("hello").to_not_equal("world");
```

### to_be_null()

Asserts that the actual value is null:

```soli
expect(result.error).to_be_null();
expect(user.deleted_at).to_be_null();
```

### to_not_be_null()

Asserts that the actual value is not null:

```soli
expect(user.id).to_not_be_null();
expect(response.body).to_not_be_null();
```

### to_be_greater_than(expected)

Asserts that the actual number is greater than expected. This and the three other
comparisons accept an Int and a Float mixed (`expect(3).to_be_greater_than(2.5)`):

```soli
expect(10).to_be_greater_than(5);
expect(count).to_be_greater_than(0);
```

### to_be_less_than(expected)

Asserts that the actual number is less than expected:

```soli
expect(5).to_be_less_than(10);
expect(len(items)).to_be_less_than(100);
```

### to_be_greater_than_or_equal(expected)

Asserts that the actual number is greater than or equal to expected:

```soli
expect(10).to_be_greater_than_or_equal(10);
expect(count).to_be_greater_than_or_equal(1);
```

### to_be_less_than_or_equal(expected)

Asserts that the actual number is less than or equal to expected:

```soli
expect(5).to_be_less_than_or_equal(5);
expect(len(items)).to_be_less_than_or_equal(10);
```

### to_contain(item)

Asserts that the actual value (array or string) contains the given item:

```soli
expect([1, 2, 3]).to_contain(2);
expect("hello world").to_contain("world");
```

### to_match(substring)

Asserts a string **contains** `substring`. Despite the name this is not a regex —
it is the same test as `to_contain` on a string. For a pattern, use the builtin
`assert_match(string, pattern)`, which is regex-based.

```soli
expect(response["body"]).to_match("Saved");
expect(slug).to_match("-");
```

### to_have_key(key)

Asserts that the actual hash has the key:

```soli
expect(payload).to_have_key("token");
```

### to_be_valid_json()

Asserts that the actual string is valid JSON:

```soli
expect('{"name": "Alice"}').to_be_valid_json();
expect(response.body).to_be_valid_json();
```

## Custom Assertions

A custom assertion is an ordinary function that raises — the runner treats a raise as
a failure, so `throw` is the whole contract:

```soli
def assert_length(collection, expected)
    actual = collection.length()
    throw "expected #{expected} items, got #{actual}" unless actual == expected
end

def assert_starts_with(text, prefix)
    throw "#{text} does not start with #{prefix}" unless text.starts_with(prefix)
end
```

Composing the builtins works too, since they raise on their own:

```soli
def assert_valid_slug(slug)
    assert_not_null(slug)
    assert_match(slug, "^[a-z0-9-]+$")
end
```

## Best Practices

1. **One concern per assertion** — a failure should name the thing that broke.
2. **Prefer the specific assertion** — `assert_null(x)` beats `assert_eq(x, null)`; it
   says what it means and fails with a clearer message.
3. **Pass a message only when the values are not enough** — the failure already shows
   them with the file and line. A message earns its place in a loop
   (`assert_eq(row["total"], 0, "row #{i}")`) or when two checks compare the same values.
4. **Keep `assert` for real booleans** — for presence, use `assert_not_null`.
5. **Budget your queries** — an endpoint spec that asserts `assert_max_queries` or
   `assert_no_n_plus_one` catches a regression that a value assertion never will.

## Running Them

```bash
soli test                       # everything under tests/
soli test tests/user_test.sl    # one file
soli test --fail-on-n1          # fail any request spec that triggers an N+1
soli test --filter "lists posts" # only tests whose describe/test names match
soli test --fail-fast           # stop after the first failing test
soli test --require-assertions  # fail a test that makes no assertion
soli test --browser             # also run browser specs
```

See [Testing](testing.md) for the runner, fixtures, and lifecycle hooks,
[E2E Controller Testing](testing-e2e.md) for request specs, and
[Browser Testing](testing-browser.md) for the browser helpers.
