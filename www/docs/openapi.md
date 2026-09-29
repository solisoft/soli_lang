# OpenAPI & API Reference

Soli describes your app as an [OpenAPI 3](https://spec.openapis.org/oas/v3.0.3)
document, built from its routes, and serves a browsable API reference over it.
Add a route and it is in the spec; add a doc comment above the action and it is
described — parameters, request body, responses.

| URL | What it serves |
|-----|----------------|
| `GET /openapi.json` | The OpenAPI 3.0.3 document, generated from the route table |
| `GET /openapi` | A [Scalar](https://scalar.com) API reference over that document |

## Quick start

```bash
soli serve . --dev
# then open http://localhost:<port>/openapi
```

Under `--dev` both endpoints are on by default, and the dev bar's **tools**
panel links the reference ("api").

## Turning it on and off

| Situation | Result |
|-----------|--------|
| `soli serve . --dev` | on |
| `SOLI_OPENAPI=0 soli serve . --dev` | off (404) |
| `soli serve` (production) | off (404) |
| `SOLI_OPENAPI=1 soli serve` | on, in every environment |

`SOLI_OPENAPI` wins when it is set: `1` or `true` turns the endpoints on,
anything else turns them off. Without it, `--dev` decides. `SOLI_OPENAPI_TITLE`
sets the document title (default `Soli API`).

## What the spec contains

Every route in `config/routes.sl` becomes an operation. Without a doc comment
([Documenting an action](#documenting-an-action)), this is all it has:

| From the route | In the spec |
|----------------|-------------|
| path `/posts/:id` | path `/posts/{id}` |
| wildcard `/files/*path` | path `/files/{path}` |
| each `:param` / `*param` | a path parameter: `in: path`, `required: true`, `type: string` |
| method (`get`, `post`, `put`, `patch`, `delete`, `head`, `options`) | the operation's method |
| `posts#show` | `summary: posts#show`, `operationId: get_posts_show` |
| the controller (`posts`) | the tag, so the reference groups operations by controller |
| — | one generic response, `200 OK` |

Routes sharing a path collapse into one path item with several methods.
Framework paths (anything starting with `/_`, including `/__soli/*`) and
WebSocket routes are left out.

A scaffolded `posts` resource gives, for `/posts/{id}`:

```json
{
  "openapi": "3.0.3",
  "info": { "title": "Soli API", "version": "1.0.0" },
  "paths": {
    "/posts/{id}": {
      "get": {
        "operationId": "get_posts_show",
        "summary": "posts#show",
        "tags": ["posts"],
        "parameters": [
          { "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }
        ],
        "responses": { "200": { "description": "OK" } }
      },
      "put":    { "operationId": "put_posts_update",    "summary": "posts#update", "...": "..." },
      "delete": { "operationId": "delete_posts_delete", "summary": "posts#delete", "...": "..." }
    }
  }
}
```

The document is valid OpenAPI 3.0.3 (`redocly lint` with the `minimal`
ruleset passes). Stricter rulesets flag what the generator does not describe —
no `servers`, no `security`, no 4xx responses.

## Documenting an action

Write a comment block directly above the action. It becomes the operation's
documentation **when it contains at least one `@tag` line** — an ordinary comment
(`# GET /posts — lists them`) stays out of the spec.

```soli
class PostsController < Controller
  # List posts.
  # Newest first, 20 per page.
  # @query page Int  Page number, from 1
  # @query q String  Full-text filter
  # @response 200 [{"_key": "42", "title": "Hello", "views": 3}]
  def index
    @posts = Post.all
  end

  # Show one post.
  # @param id String  The post's key
  # @response 200 {"_key": "42", "title": "Hello",
  #                "tags": ["soli", "openapi"]}
  # @response 404 No post with that key
  def show
    @post = Post.find(params["id"])
  end

  # Create a post.
  # @header X-Request-Id String! Idempotency key
  # @body {"title": "Hello", "tags": ["soli"]}
  # @response 201 {"_key": "42", "title": "Hello"}
  # @response 422 Validation failed
  def create
    ...
  end

  # @hidden
  def delete
    ...
  end
end
```

| Line | In the spec |
|------|-------------|
| first text line | `summary` (without one, the summary stays `controller#action`) |
| further text lines | `description` |
| `@param <name> <Type> <text>` | describes the `:name` path parameter — always required |
| `@query <name> <Type> <text>` | a query parameter, optional |
| `@header <name> <Type> <text>` | a header parameter, optional |
| `Type!` (`String!`, `Int!`) | marks a `@query` / `@header` required |
| `@body <JSON>` | the request body: the JSON is the example, its schema is inferred |
| `@body <text>` | a request body described in words |
| `@response <code> <JSON>` | a response with that example and its inferred schema |
| `@response <code> <text>` | a response described in words |
| `@tag <Name>` | the group in the reference, instead of the controller name |
| `@deprecated` | `deprecated: true` |
| `@hidden` | the operation is left out of the spec |

- **Types** — `String`, `Int`, `Float`, `Bool`, `Array`, `Hash`; the type may be
  omitted (`String`).
- **JSON on several lines** — a `#` line that does not start with `@` continues the tag
  above it, as in `show` above.
- **Schemas from examples** — `"views": 3` becomes `integer`, `1.5` `number`,
  `"x"` `string`, `true` `boolean`; arrays take their first item's schema; objects
  nest.
- **Responses** — once an action declares one, its `@response` lines replace the
  default `200 OK`; a code without text is described by its standard reason
  (`201` → `Created`).
- Docs are read when the app boots and again on every `--dev` reload, so an edited
  comment shows up on the next request to `/openapi.json`. `soli build --protect`
  bundles keep them.

### The request body from `permit()`

An action with no `@body` gets its request body from its `permit(...)` whitelist —
in the action itself or in the `_permit_params` helper it calls, which is what
`soli generate scaffold` writes:

```soli
def create
  @post = Post.create(permit(params, {"title": true, "tags": [], "author": {"name": true}}))
  ...
end
```

gives a JSON body with `title` (any value), `tags` (an array) and `author` (an object
with `name`). This applies to `POST`, `PUT` and `PATCH` routes, with or without a doc
block — so a scaffolded resource documents its create and update bodies with no
comment at all. A `@body` line wins over the whitelist.

### Checking doc comments

`soli lint` reports mistakes in a controller's doc comments under the
`docs/openapi` rule: an unknown tag (`@returns`), a `@response` without a status
code, a `@param` without a name, and a `@body` / `@response` that starts like JSON
but does not parse. The spec itself never fails on them — the bad part is dropped or
used as text — which is why the lint rule exists.

```text
app/controllers/posts_controller.sl:61:1 - [docs/openapi] unknown doc tag `@hiden` (known: @param, @query, @header, @body, @response, @tag, @deprecated, @hidden)
```

## Browsing it

`/openapi` is a Scalar page pointed at `/openapi.json`: operations grouped by
controller, a search box, and a request panel to try an endpoint against the
running server. Scalar loads from a CDN, so the page needs network access in
the browser; `/openapi.json` itself does not.

Any OpenAPI viewer can load the same URL — Swagger UI, Redoc, or your API
client's import.

## Using the spec

**TypeScript types for a frontend:**

```bash
npx openapi-typescript http://localhost:<port>/openapi.json -o src/api.d.ts
```

This produces a `paths` interface with one entry per route and the path
parameters typed.

**An API client** (Postman, Insomnia, Bruno, Hoppscotch): import from the URL
`http://localhost:<port>/openapi.json`. Clients that group by tag show one
group per controller.

**A route-change check in CI:** commit a snapshot and diff it, so a pull request
shows the routes it adds or removes:

```bash
SOLI_OPENAPI=1 soli serve . --port 5099 --strict-port &
sleep 2
curl -s localhost:5099/openapi.json > openapi.json
git diff --exit-code openapi.json
```

## In production

Once `SOLI_OPENAPI=1` is set, the endpoints answer in every environment, like
`/_metrics`. They are answered **before the application runs** — before
middleware, sessions and controllers — so a `before_action` or an
authentication middleware does **not** protect them. Anyone who can reach the
server can read the full route table.

Keep them off in production (the default), or put them behind your reverse
proxy's access rules if you need the reference there.

## Limits

- The spec knows what the doc comments and `permit()` say, nothing more: an
  undocumented action has path parameters, a `controller#action` summary and a
  generic `200`.
- Schemas are inferred from examples and whitelists, so they give types, not
  constraints — no `required` fields inside a body, formats, enums or lengths.
- Path parameters without a `@param` line are `string`s.
- HTML routes (`/posts/new`, `/posts/{id}/edit`) are listed alongside JSON ones —
  hide them with `@hidden` if the reference is for API clients only.

## See also

- [Routing](routing.md) — the routes the spec is built from
- [Configuration](configuration.md) — `SOLI_OPENAPI`, `SOLI_OPENAPI_TITLE`
- [Debugging](debugging.md) — the dev bar and its tools panel
