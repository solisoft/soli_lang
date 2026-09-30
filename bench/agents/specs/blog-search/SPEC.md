# Blog Search API

A JSON HTTP API for blog posts with tags, search and pagination. Every request and
response body is JSON (`Content-Type: application/json`).

## Resource

A post has:

| Field          | Type                       | Notes                                          |
|----------------|----------------------------|------------------------------------------------|
| `id`           | string or integer          | assigned by the server                         |
| `title`        | string                     | required, 1–200 characters                     |
| `body`         | string                     | may be empty, defaults to `""`                 |
| `tags`         | array of strings           | defaults to `[]`; stored lowercased, trimmed, without duplicates, in first-seen order |
| `published_at` | string, ISO 8601 timestamp | required                                       |

## Endpoints

### `POST /posts`
- `201` with the created post.
- `422` with `{"errors": {...}}` when `title` is invalid, `published_at` is missing or
  not an ISO 8601 timestamp, or `tags` is not an array of strings.

### `GET /posts`
Returns `200` with:

```json
{"items": [...], "total": 42, "page": 1, "per_page": 10}
```

- `items` are sorted by `published_at`, newest first.
- `total` counts every post matching the filters, across all pages.
- `?tag=x` — only posts carrying tag `x` (case-insensitive).
- `?q=word` — only posts whose title or body contains `word`, case-insensitively.
- Filters combine (AND).
- `?page=` (default `1`) and `?per_page=` (default `10`, capped at `50`: a larger
  value behaves as `50`). A page past the end returns an empty `items` array and the
  real `total`.

### `GET /posts/:id`
- `200` with the post, `404` if it does not exist.

### `GET /tags`
- `200` with `[{"name": "rust", "count": 3}, ...]` — one entry per tag used by at
  least one post, sorted by `count` descending, then `name` ascending.

## Persistence

Data must survive a server restart.
