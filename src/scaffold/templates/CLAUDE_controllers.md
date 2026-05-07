# Controllers

One file per resource. `posts_controller.sl` defines `class PostsController < Controller`.

## Shape

- Actions take `req`, return `render("path/name", {...})`, `redirect(...)`, or a raw `{"status": ..., "headers": {...}, "body": "..."}` hash.
- `req.params["id"]` merges route + query + JSON body. Bare `params` is also a global inside actions (= `req.params`).
- `req["json"]`, `req["headers"]`, `req["cookies"]`, `req["method"]` for the rest.

## Rules

1. **Don't import models.** They're auto-loaded under `soli serve`. Adding `import "../models/*.sl"` triggers the `style/redundant-model-import` lint.
2. **Use named route helpers.** `posts_path()`, `post_path(post)`, `new_post_path()` come from `resources("posts")` in `config/routes.sl`. Never hand-build URLs.
3. **Whitelist params for mass-assignment.** Add a private `_permit_params(params)` and only pass its result to `Model.create` / `update`.
4. **Let `Model.find` raise on miss.** The framework maps the exception to a 404 — don't wrap in try/catch.
5. **On validation failure**, `Model.create(...)` returns the instance with `_errors` populated. Check `if post._errors` and re-render the form view with the instance as a local.
6. **Auto-exposed locals**: assigning `this.post = post` in an action automatically exposes `post` as a view local — you don't have to repeat it in the `render(...)` data hash.

## Spec location

Every controller has a sibling spec at `tests/<name>_controller_spec.sl`. Use the controller HTTP client (`get`, `post`, `put`, `delete`, `res_status`, `assigns`).
