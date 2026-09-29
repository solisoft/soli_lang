# OpenAPI & API Reference

Soli describes your app as an [OpenAPI 3](https://spec.openapis.org/oas/v3.0.3)
document, built from its routes, and serves a browsable API reference over it.
There is nothing to write and nothing to install: add a route, reload, and it is
in the spec.

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

Every route in `config/routes.sl` becomes an operation:

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

The spec is **structural**: Soli actions take an untyped `req`, so the
generator knows which endpoints exist and their path parameters, not what they
accept or return.

- No request bodies, query parameters, headers or response schemas.
- Every path parameter is a `string`, whatever the action does with it.
- Every operation declares one `200` response, including redirects and errors.
- HTML routes (`/posts/new`, `/posts/{id}/edit`) are listed alongside JSON
  ones: the route table does not say which is which.

It is a map of the API and a seed for generated clients, not a hand-written
contract.

## See also

- [Routing](routing.md) — the routes the spec is built from
- [Configuration](configuration.md) — `SOLI_OPENAPI`, `SOLI_OPENAPI_TITLE`
- [Debugging](debugging.md) — the dev bar and its tools panel
