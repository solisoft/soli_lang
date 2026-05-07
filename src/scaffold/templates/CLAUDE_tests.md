# Tests

BDD-style specs in `tests/`. File pattern: `*_spec.sl`. Run with `soli test`.

## Shape

```soli
describe("PostsController", fn() {
  before_each(fn() { as_guest(); });

  describe("GET /posts", fn() {
    test("returns a list", fn() {
      let response = get("/posts");
      assert_eq(res_status(response), 200);
      assert_hash_has_key(assigns(), "posts");
    });
  });
});
```

## Controller spec helpers

`get(path)`, `post(path, body)`, `put`, `delete` — HTTP client for routes.
`res_status(response)`, `res_body(response)` — inspect response.
`assigns()` — controller-set fields exposed to views (`this.foo = ...`).
`view_path()` — name of the rendered view.
`as_guest()`, `as_user(user)` — auth helpers.

## Assertions

`assert_eq(actual, expected)`, `assert(cond)`, `assert_null(v)`, `assert_not_null(v)`, `assert_hash_has_key(h, k)`.

## Running

- One spec (fast feedback): `soli test tests/posts_controller_spec.sl`
- Full suite with coverage gate: `soli test --coverage --coverage-min 90.0`

## Rules

- New behavior → new test. The 90% coverage gate is non-negotiable.
- Don't lower the threshold to ship — write the missing test instead.
- Don't mock the database in controller specs unless the spec is purely about the controller's branching logic; integration coverage is the point.
