# Full End-to-End Tests in Pure Soli (No Playwright, No Node)

Most teams today write their application in one language and their integration tests in another.

You have a beautiful Soli backend… and then you reach for Playwright, Cypress, or a pile of `fetch` calls in a separate test runner just to verify that your controllers actually return the right HTML and that sessions work.

Soli takes a different stance: the best way to test a Soli application is with Soli.

The E2E testing framework gives you a real HTTP server that starts and stops automatically, a clean BDD DSL, powerful request/response helpers, session inspection, and coverage reporting — all without leaving the language or adding a single Node dependency.

<figure style="margin:1.5rem auto;max-width:1024px;">
  <img src="/images/blog/e2e-testing.jpg" width="1024" height="576" alt="Dark developer workspace showing Soli E2E tests running successfully in a terminal with green passes, alongside a code editor displaying describe and test blocks using the pure-Soli testing framework." style="display:block;width:100%;height:auto;border-radius:12px;border:1px solid #30363d;background:#0b0d0f;">
  <figcaption style="text-align:center;color:#8b949e;font-size:0.875rem;margin-top:0.5rem;">Real integration tests running entirely inside Soli — no external test runners required.</figcaption>
</figure>

## The Test Server

When you run `soli test`, any E2E spec files automatically bring up an isolated test server on a random port. Each test file gets a clean instance. No port conflicts, no manual server management, no state leaking between runs.

```soli
describe("Checkout Flow", fn()
    before_each(fn()
        # Fresh data for every test
        @user = create_test_user()
    end)

    test("completes a successful purchase", fn()
        response = post("/checkout", {
            "product_id": @product.id,
            "quantity": 2
        })

        assert_eq(res_status(response), 302)
        assert(res_location(response).includes?("/orders/"))

        # Verify side effects
        order = Order.last
        assert_eq(order.user_id, @user.id)
        assert_eq(order.total, 199.00)
    end)
end)
```

## Request & Response Helpers

The framework provides ergonomic helpers that feel natural in Soli:

- `get(path)`
- `post(path, body)`
- `put(path, body)`
- `delete(path)`

Response inspection is equally pleasant:

```soli
response = get("/products/" + product.id)
assert_eq(res_status(response), 200)

html = res_body(response)
assert(html.includes?("Add to cart"))

json = res_json(response)          # automatic JSON parsing
assert_eq(json["name"], "Wireless Headphones")
```

You also get:

- `res_headers(response)`
- `res_location(response)` for redirect targets

## Real Session Testing

Because requests go through the real session middleware, you can test authenticated flows properly. Three helpers cover the common cases:

- `as_user(user_id)` — simulate a logged-in user for the next request.
- `create_session(user_id)` — issue a real session cookie reused across requests.
- `with_session({...})` — seed arbitrary server-side session data (test-only).

```soli
test("requires login to view orders", fn()
    response = get("/orders")
    assert_eq(res_status(response), 302)
    assert(res_location(response).includes?("/login"))
end)

test("logged in user sees their orders", fn()
    as_user(@user.id)

    response = get("/orders")
    assert_eq(res_status(response), 200)
    assert(res_body(response).includes?("Order #1234"))
end)
```

## Coverage That Actually Matters

Running with `--coverage` instruments your application code (controllers, models, etc.) and reports real line coverage from the E2E tests. The project template even enforces a minimum via `--coverage-min 90`.

This is integration coverage — the kind that actually protects you — not just unit test theater.

## Why This Approach Wins

- **Zero context switching** — You stay in Soli the entire time.
- **Fast feedback** — No slow browser automation unless you truly need it.
- **Reliable** — Same runtime, same ORM, same everything as production.
- **Low maintenance** — No separate test infrastructure to keep green.

For the majority of controller and flow testing, this is dramatically more effective than driving a real browser for every scenario.

When you *do* need full browser tests (complex JavaScript interactions, visual regressions), you can still add Playwright later. But most teams discover they need far less of it than they expected once they have powerful pure-Soli E2E tests.

---

If your current testing story involves two languages and a slow CI stage just to hit your own controllers, Soli’s approach is worth experiencing. The difference in speed and joy is substantial.