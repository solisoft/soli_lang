# Middleware

One function per file. The function name **must match** the filename (snake_case): `app/middleware/auth.sl` defines `def authenticate(req)`.

## File-top directives

```soli
# order: 20
# scope_only: true
```

| Directive             | Meaning                                                       |
|-----------------------|---------------------------------------------------------------|
| `# order: N`          | Lower runs first. Default 100.                                |
| `# global_only: true` | Always runs; cannot be scoped.                                |
| `# scope_only: true`  | Only runs inside `middleware("name", -> { ... })` blocks.     |

## Return shape

- Proceed: `{ "continue": true, "request": req }` — pass an updated `req` to the next layer.
- Short-circuit: `{ "continue": false, "response": { "status": 401, "body": "..." } }`.

Don't mutate `req` in place. Return the modified copy in the `request` field.

## Scoping in routes

```soli
# config/routes.sl
middleware("authenticate", -> {
  get("/admin", "admin#index")
  resources("admin/users")
})
```

The `authenticate` middleware in this scaffold is `scope_only`, so unscoped routes are unaffected.
