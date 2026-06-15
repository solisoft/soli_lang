# scoped_middleware

Two pieces of a small access-control slice: a reusable model **scope** and a
**middleware** guard.

Implement, in `stub.sl`:

- `class User extends Model` with a scope named `admins` that filters to
  records whose `role` is `"admin"`. Define it with the class-body DSL:

  ```soli
  scope("admins", fn() this.where("doc.role == @r", {"r": "admin"}))
  ```

- `require_admin(req)` — a middleware function. `req` is a hash that may carry a
  `"user"` hash with a `"role"`. Return `null` to let the request through when
  the user's role is `"admin"`; otherwise return
  `{"status": 403, "body": "Forbidden"}` (including when there is no user).

Idiomatic touches:

- Query the scope with `User.admins.all()`.
- Reach for a postfix guard / `unless` rather than a nested `if`.

## Requires SoliDB

Export `SOLIDB_HOST`, `SOLIDB_USERNAME`, and `SOLIDB_PASSWORD` before grading
(see the suite README).
