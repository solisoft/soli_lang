"""Acceptance checks for specs/auth-notes/SPEC.md. Each check gets a Tester `t`."""

import time

PASSWORD = "correct horse battery"


def _user(t, name):
    email = f"{name}-{t.nonce}@example.com"
    status, body = t.post("/signup", {"email": email, "password": PASSWORD})
    t.expect(status == 201, f"signup -> {status} {body!r}")
    return email


def _token(t, email, password=PASSWORD):
    status, body = t.post("/login", {"email": email, "password": password})
    t.expect(status == 200 and isinstance(body, dict) and isinstance(body.get("token"), str),
             f"login -> {status} {body!r}")
    return body["token"]


def _auth(token):
    return {"Authorization": f"Bearer {token}"}


def _note(t, token, title="A note", **extra):
    status, body = t.post("/notes", {"title": title, **extra}, headers=_auth(token))
    t.expect(status == 201, f"POST /notes -> {status} {body!r}")
    return body


def signup_hides_password(t):
    email = f"  Mixed-{t.nonce}@Example.COM "
    status, body = t.post("/signup", {"email": email, "password": PASSWORD})
    t.expect(status == 201, f"signup -> {status} {body!r}")
    t.expect(body.get("email") == email.strip().lower(), f"email not normalised: {body.get('email')!r}")
    t.expect(t.has_id(body), f"no id in {body!r}")
    flat = repr(body).lower()
    t.expect(PASSWORD not in flat and "password" not in flat and "hash" not in flat,
             f"password leaks in {body!r}")


def signup_validates(t):
    for payload in ({}, {"email": f"nope-{t.nonce}", "password": PASSWORD},
                    {"email": f"a@b@{t.nonce}.com", "password": PASSWORD},
                    {"email": "@example.com", "password": PASSWORD},
                    {"email": f"short-{t.nonce}@example.com", "password": "1234567"},
                    {"email": f"nopass-{t.nonce}@example.com"}):
        status, body = t.post("/signup", payload)
        t.expect(status == 422, f"{payload!r} -> {status}, expected 422")
        t.expect(isinstance(body, dict) and "errors" in body, f"{payload!r}: no errors key in {body!r}")


def signup_rejects_duplicates(t):
    email = _user(t, "dup")
    status, body = t.post("/signup", {"email": email.upper(), "password": PASSWORD})
    t.expect(status == 409, f"duplicate email -> {status}, expected 409: {body!r}")


def login_checks_credentials(t):
    email = _user(t, "login")
    _token(t, email)
    status, _ = t.post("/login", {"email": email, "password": "wrong password"})
    t.expect(status == 401, f"wrong password -> {status}, expected 401")
    status, _ = t.post("/login", {"email": f"ghost-{t.nonce}@example.com", "password": PASSWORD})
    t.expect(status == 401, f"unknown email -> {status}, expected 401")


def notes_require_a_valid_token(t):
    for headers in ({}, {"Authorization": "Bearer not-a-real-token"}, {"Authorization": "Token abc"}):
        status, _ = t.get("/notes", headers=headers)
        t.expect(status == 401, f"GET /notes with {headers!r} -> {status}, expected 401")
    status, _ = t.post("/notes", {"title": "sneaky"})
    t.expect(status == 401, f"POST /notes without token -> {status}, expected 401")


def create_note_and_validate(t):
    token = _token(t, _user(t, "writer"))
    note = _note(t, token, "Groceries")
    t.expect(note.get("body") == "", f"body should default to \"\": {note!r}")
    t.expect(t.is_iso(note.get("created_at")), f"created_at not ISO 8601: {note!r}")
    for payload in ({}, {"title": ""}, {"title": "x" * 201}):
        status, _ = t.post("/notes", payload, headers=_auth(token))
        t.expect(status == 422, f"{payload!r} -> {status}, expected 422")


def list_shows_own_notes_newest_first(t):
    alice = _token(t, _user(t, "alice"))
    bob = _token(t, _user(t, "bob"))
    first = _note(t, alice, "first")["id"]
    time.sleep(1.1)
    second = _note(t, alice, "second")["id"]
    _note(t, bob, "bob's")
    status, body = t.get("/notes", headers=_auth(alice))
    t.expect(status == 200 and isinstance(body, list), f"GET /notes -> {status} {body!r}")
    t.expect([note.get("id") for note in body] == [second, first], f"expected [second, first], got {body!r}")


def other_users_notes_are_404(t):
    alice = _token(t, _user(t, "owner"))
    mallory = _token(t, _user(t, "mallory"))
    note_id = _note(t, alice, "secret", body="launch codes")["id"]
    status, _ = t.get(f"/notes/{note_id}", headers=_auth(mallory))
    t.expect(status == 404, f"foreign GET -> {status}, expected 404")
    status, _ = t.patch(f"/notes/{note_id}", {"title": "pwned"}, headers=_auth(mallory))
    t.expect(status == 404, f"foreign PATCH -> {status}, expected 404")
    status, _ = t.delete(f"/notes/{note_id}", headers=_auth(mallory))
    t.expect(status == 404, f"foreign DELETE -> {status}, expected 404")
    status, body = t.get(f"/notes/{note_id}", headers=_auth(alice))
    t.expect(status == 200 and body.get("title") == "secret", f"owner's note changed: {status} {body!r}")


def patch_and_delete_own_note(t):
    token = _token(t, _user(t, "editor"))
    note_id = _note(t, token, "draft")["id"]
    status, body = t.patch(f"/notes/{note_id}", {"body": "final text"}, headers=_auth(token))
    t.expect(status == 200 and body.get("body") == "final text" and body.get("title") == "draft",
             f"PATCH -> {status} {body!r}")
    status, _ = t.patch(f"/notes/{note_id}", {"title": ""}, headers=_auth(token))
    t.expect(status == 422, f"blank title PATCH -> {status}, expected 422")
    status, _ = t.delete(f"/notes/{note_id}", headers=_auth(token))
    t.expect(status == 204, f"DELETE -> {status}, expected 204")
    status, _ = t.get(f"/notes/{note_id}", headers=_auth(token))
    t.expect(status == 404, f"GET after DELETE -> {status}, expected 404")


def logout_revokes_the_token(t):
    token = _token(t, _user(t, "leaver"))
    status, _ = t.post("/logout", None, headers=_auth(token))
    t.expect(status == 204, f"logout -> {status}, expected 204")
    status, _ = t.get("/notes", headers=_auth(token))
    t.expect(status == 401, f"token after logout -> {status}, expected 401")


def survives_restart(t):
    email = _user(t, "durable")
    note_id = _note(t, _token(t, email), "kept")["id"]
    t.restart()
    status, body = t.get(f"/notes/{note_id}", headers=_auth(_token(t, email)))
    t.expect(status == 200 and body.get("title") == "kept", f"after restart -> {status} {body!r}")


CHECKS = [
    signup_hides_password,
    signup_validates,
    signup_rejects_duplicates,
    login_checks_credentials,
    notes_require_a_valid_token,
    create_note_and_validate,
    list_shows_own_notes_newest_first,
    other_users_notes_are_404,
    patch_and_delete_own_note,
    logout_revokes_the_token,
    survives_restart,
]
