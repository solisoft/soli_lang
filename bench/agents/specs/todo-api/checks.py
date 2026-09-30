"""Acceptance checks for specs/todo-api/SPEC.md. Each check gets a Tester `t`."""


def _create(t, title="Buy milk", **extra):
    status, body = t.post("/todos", {"title": title, **extra})
    t.expect(status == 201, f"POST /todos -> {status}, expected 201: {body!r}")
    return body


def create_returns_the_todo(t):
    todo = _create(t, "  Buy milk  ")
    t.expect(t.has_id(todo), f"no id in {todo!r}")
    t.expect(todo.get("title") == "Buy milk", f"title not trimmed: {todo.get('title')!r}")
    t.expect(todo.get("done") is False, f"done should default to false: {todo.get('done')!r}")
    t.expect(t.is_iso(todo.get("created_at")), f"created_at not ISO 8601: {todo.get('created_at')!r}")


def create_accepts_done_and_200_chars(t):
    todo = _create(t, "x" * 200, done=True)
    t.expect(todo.get("done") is True, f"done not kept: {todo!r}")


def create_rejects_invalid_input(t):
    for payload in ({}, {"title": ""}, {"title": "   "}, {"title": "x" * 201},
                    {"title": 42}, {"title": "ok", "done": "yes"}):
        status, body = t.post("/todos", payload)
        t.expect(status == 422, f"{payload!r} -> {status}, expected 422")
        t.expect(isinstance(body, dict) and "errors" in body, f"{payload!r}: no errors key in {body!r}")


def show_and_404(t):
    todo = _create(t, "Show me")
    status, body = t.get(f"/todos/{todo['id']}")
    t.expect(status == 200 and body.get("title") == "Show me", f"GET -> {status} {body!r}")
    status, _ = t.get("/todos/999999999")
    t.expect(status == 404, f"unknown id -> {status}, expected 404")


def list_is_oldest_first(t):
    ids = [_create(t, f"order {i}")["id"] for i in range(3)]
    status, body = t.get("/todos")
    t.expect(status == 200 and isinstance(body, list), f"GET /todos -> {status} {body!r}")
    seen = [todo["id"] for todo in body if todo.get("id") in ids]
    t.expect(seen == ids, f"creation order {ids}, listed {seen}")


def list_filters_on_done(t):
    done_id = _create(t, "finished", done=True)["id"]
    open_id = _create(t, "pending")["id"]
    status, done = t.get("/todos?done=true")
    t.expect(status == 200 and all(todo.get("done") is True for todo in done), f"?done=true -> {done!r}")
    t.expect(done_id in [todo["id"] for todo in done], "done todo missing from ?done=true")
    status, pending = t.get("/todos?done=false")
    t.expect(status == 200 and all(todo.get("done") is False for todo in pending), f"?done=false -> {pending!r}")
    t.expect(open_id in [todo["id"] for todo in pending], "open todo missing from ?done=false")


def patch_updates_and_validates(t):
    todo = _create(t, "Before")
    status, body = t.patch(f"/todos/{todo['id']}", {"title": " After ", "done": True})
    t.expect(status == 200, f"PATCH -> {status} {body!r}")
    t.expect(body.get("title") == "After" and body.get("done") is True, f"PATCH result {body!r}")
    status, body = t.get(f"/todos/{todo['id']}")
    t.expect(body.get("title") == "After" and body.get("done") is True, f"not saved: {body!r}")
    status, body = t.patch(f"/todos/{todo['id']}", {"title": ""})
    t.expect(status == 422, f"blank title PATCH -> {status}, expected 422")
    status, _ = t.patch("/todos/999999999", {"done": True})
    t.expect(status == 404, f"PATCH unknown -> {status}, expected 404")


def delete_then_404(t):
    todo = _create(t, "Doomed")
    status, _ = t.delete(f"/todos/{todo['id']}")
    t.expect(status == 204, f"DELETE -> {status}, expected 204")
    status, _ = t.get(f"/todos/{todo['id']}")
    t.expect(status == 404, f"GET after DELETE -> {status}, expected 404")
    status, _ = t.delete(f"/todos/{todo['id']}")
    t.expect(status == 404, f"second DELETE -> {status}, expected 404")


def survives_restart(t):
    todo = _create(t, "Persistent")
    t.restart()
    status, body = t.get(f"/todos/{todo['id']}")
    t.expect(status == 200 and body.get("title") == "Persistent", f"after restart -> {status} {body!r}")


CHECKS = [
    create_returns_the_todo,
    create_accepts_done_and_200_chars,
    create_rejects_invalid_input,
    show_and_404,
    list_is_oldest_first,
    list_filters_on_done,
    patch_updates_and_validates,
    delete_then_404,
    survives_restart,
]
