# Private Notes API

A JSON HTTP API where users sign up, log in with a bearer token and keep private notes.
Every request and response body is JSON (`Content-Type: application/json`), except
`204` responses, which have no body.

## Users

### `POST /signup`
Body: `{"email": "...", "password": "..."}`.
- `201` with `{"id": ..., "email": "..."}`. The response must never contain the password
  or any hash of it.
- `email` is stored lowercased and trimmed; it must contain exactly one `@` with
  characters on both sides.
- `password` must be at least 8 characters.
- `422` with `{"errors": {...}}` when either field is invalid or missing.
- `409` with `{"errors": {...}}` when the email is already registered (compared
  case-insensitively).

### `POST /login`
Body: `{"email": "...", "password": "..."}`.
- `200` with `{"token": "..."}` — an opaque string.
- `401` when the email is unknown or the password is wrong.

### `POST /logout`
Requires the token. `204`; the token no longer works afterwards.

## Authentication

Every `/notes` endpoint and `/logout` require `Authorization: Bearer <token>`.
A missing, malformed, unknown or logged-out token gets `401`.

## Notes

A note has `id` (string or integer), `title` (string, required, 1–200 characters),
`body` (string, may be empty, defaults to `""`) and `created_at` (ISO 8601).

### `POST /notes`
- `201` with the note. `422` with `{"errors": {...}}` on an invalid title.

### `GET /notes`
- `200` with an array of **the current user's** notes, newest first.

### `GET /notes/:id`, `PATCH /notes/:id`, `DELETE /notes/:id`
- `200` (`GET`, `PATCH` with the updated note) or `204` (`DELETE`).
- A note that belongs to another user must be indistinguishable from a missing one:
  `404`.
- `PATCH` accepts any subset of `{"title", "body"}` and validates like `POST`.

## Persistence

Users and notes must survive a server restart. Tokens may be invalidated by a restart.
