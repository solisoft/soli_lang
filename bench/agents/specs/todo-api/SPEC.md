# Todo API

A JSON HTTP API for a todo list. Every request and response body is JSON
(`Content-Type: application/json`), except `204` responses, which have no body.

## Resource

A todo has:

| Field        | Type                          | Notes                                   |
|--------------|-------------------------------|-----------------------------------------|
| `id`         | string or integer             | assigned by the server, unique          |
| `title`      | string                        | required, 1–200 characters after trimming surrounding whitespace |
| `done`       | boolean                       | defaults to `false`                     |
| `created_at` | string, ISO 8601 timestamp    | assigned by the server                  |

`title` is stored trimmed.

## Endpoints

### `POST /todos`
Body: `{"title": "...", "done": false}` (`done` optional).
- `201` with the created todo.
- `422` with `{"errors": {...}}` when `title` is missing, not a string, blank, or longer
  than 200 characters, or when `done` is present and not a boolean.

### `GET /todos`
- `200` with a JSON array of todos, oldest first (creation order).
- `?done=true` / `?done=false` returns only the matching todos.

### `GET /todos/:id`
- `200` with the todo, `404` if it does not exist.

### `PATCH /todos/:id`
Body: any subset of `{"title": "...", "done": true}`.
- `200` with the updated todo.
- `422` with `{"errors": {...}}` on the same validation rules as creation.
- `404` if it does not exist.

### `DELETE /todos/:id`
- `204` on success, `404` if it does not exist.

## Persistence

Data must survive a server restart.
