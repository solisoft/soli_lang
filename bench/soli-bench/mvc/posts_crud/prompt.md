# posts_crud

Build the model + controller actions for a classic RESTful `Post` resource,
backed by SoliDB. A `Post` has a `title` (required) and a `body`.

Implement, in `stub.sl`:

- `class Post extends Model` with a presence validation on `title`
- `create_post(params)` — create from `params`. On success return
  `{"status": 201, "key": <new _key>, "title": <title>}`; on a validation
  failure return `{"status": 422, "errors": <the instance errors>}`.
- `show_post(key)` — `{"status": 200, "title": <title>}` for the post with that
  key. (Remember: `Model.find` raises when the key is missing, and the handler
  turns that into a 404 — no manual nil-check needed.)
- `update_post(key, params)` — apply `params`, return
  `{"status": 200, "title": <new title>}`.
- `delete_post(key)` — delete it, return `{"status": 204}`.
- `index_posts` — `{"status": 200, "count": <number of posts>}`.

Idiomatic touches:

- `Model.create` returns an instance; check `._errors` (or `.errors`) for
  validation failures rather than a wrapper hash.
- The record key is `._key`.
- Use the inherited `Model` methods (`create`, `find`, `update`, `delete`,
  `all`) — don't hand-roll SDQL.

## Requires SoliDB

These tasks run against a live SoliDB. Export `SOLIDB_HOST`, `SOLIDB_USERNAME`,
and `SOLIDB_PASSWORD` before grading (see the suite README).
