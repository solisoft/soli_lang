#!/usr/bin/env python3
"""Reference implementation of every spec, used only by `bench.py validate`.

It proves each acceptance suite is passable and agrees with its SPEC.md. Plain standard
library: http.server + sqlite3. PORT and REFERENCE_DB come from the environment.
"""

import datetime
import hashlib
import json
import os
import re
import secrets
import sqlite3
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

db = sqlite3.connect(os.environ.get("REFERENCE_DB", "reference.sqlite3"), check_same_thread=False)
db.row_factory = sqlite3.Row
lock = threading.Lock()
db.executescript("""
CREATE TABLE IF NOT EXISTS todos (id INTEGER PRIMARY KEY, title TEXT, done INTEGER, created_at TEXT);
CREATE TABLE IF NOT EXISTS users (id INTEGER PRIMARY KEY, email TEXT UNIQUE, salt TEXT, hash TEXT);
CREATE TABLE IF NOT EXISTS tokens (token TEXT PRIMARY KEY, user_id INTEGER);
CREATE TABLE IF NOT EXISTS notes (id INTEGER PRIMARY KEY, user_id INTEGER, title TEXT, body TEXT, created_at TEXT);
CREATE TABLE IF NOT EXISTS posts (id INTEGER PRIMARY KEY, title TEXT, body TEXT, tags TEXT, published_at TEXT);
DELETE FROM tokens;
""")


def now():
    return datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="microseconds")


def valid_title(value):
    return isinstance(value, str) and 1 <= len(value.strip()) <= 200


def parse_iso(value):
    if not isinstance(value, str):
        return None
    try:
        parsed = datetime.datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        return None
    return parsed if parsed.tzinfo else parsed.replace(tzinfo=datetime.timezone.utc)


def password_hash(password, salt):
    return hashlib.pbkdf2_hmac("sha256", password.encode(), salt.encode(), 100_000).hex()


class Reply(Exception):
    def __init__(self, status, body=None):
        self.status, self.body = status, body


def invalid(field, message):
    raise Reply(422, {"errors": {field: [message]}})


def todo_json(row):
    return {"id": row["id"], "title": row["title"], "done": bool(row["done"]), "created_at": row["created_at"]}


def note_json(row):
    return {"id": row["id"], "title": row["title"], "body": row["body"], "created_at": row["created_at"]}


def post_json(row):
    return {"id": row["id"], "title": row["title"], "body": row["body"], "tags": json.loads(row["tags"]),
            "published_at": row["published_at"]}


def one(sql, *args):
    return db.execute(sql, args).fetchone()


# ------------------------------------------------------------------ todo-api

def todos(method, parts, query, body, headers):
    if parts == ["todos"] and method == "GET":
        rows = db.execute("SELECT * FROM todos ORDER BY id").fetchall()
        if "done" in query:
            wanted = query["done"][0] == "true"
            rows = [row for row in rows if bool(row["done"]) == wanted]
        raise Reply(200, [todo_json(row) for row in rows])
    if parts == ["todos"] and method == "POST":
        fields = validate_todo(body, creating=True)
        cursor = db.execute("INSERT INTO todos (title, done, created_at) VALUES (?, ?, ?)",
                            (fields["title"], fields.get("done", False), now()))
        raise Reply(201, todo_json(one("SELECT * FROM todos WHERE id = ?", cursor.lastrowid)))
    row = one("SELECT * FROM todos WHERE id = ?", parts[1]) if len(parts) == 2 else None
    if row is None:
        raise Reply(404, {"error": "not found"})
    if method == "GET":
        raise Reply(200, todo_json(row))
    if method == "PATCH":
        fields = validate_todo(body, creating=False)
        db.execute("UPDATE todos SET title = ?, done = ? WHERE id = ?",
                   (fields.get("title", row["title"]), fields.get("done", bool(row["done"])), row["id"]))
        raise Reply(200, todo_json(one("SELECT * FROM todos WHERE id = ?", row["id"])))
    if method == "DELETE":
        db.execute("DELETE FROM todos WHERE id = ?", (row["id"],))
        raise Reply(204)
    raise Reply(405, {"error": "method not allowed"})


def validate_todo(body, creating):
    body = body if isinstance(body, dict) else {}
    fields = {}
    if creating or "title" in body:
        if not valid_title(body.get("title")):
            invalid("title", "must be 1-200 characters")
        fields["title"] = body["title"].strip()
    if "done" in body:
        if not isinstance(body["done"], bool):
            invalid("done", "must be a boolean")
        fields["done"] = body["done"]
    return fields


# ---------------------------------------------------------------- auth-notes

EMAIL = re.compile(r"^[^@\s]+@[^@\s]+$")


def current_user(headers):
    match = re.fullmatch(r"Bearer (\S+)", headers.get("Authorization", ""))
    row = one("SELECT user_id FROM tokens WHERE token = ?", match.group(1)) if match else None
    if row is None:
        raise Reply(401, {"error": "unauthorized"})
    return row["user_id"]


def auth(method, parts, query, body, headers):
    body = body if isinstance(body, dict) else {}
    if parts == ["signup"] and method == "POST":
        email = body.get("email").strip().lower() if isinstance(body.get("email"), str) else ""
        if not EMAIL.match(email):
            invalid("email", "is invalid")
        if not isinstance(body.get("password"), str) or len(body["password"]) < 8:
            invalid("password", "must be at least 8 characters")
        if one("SELECT id FROM users WHERE email = ?", email):
            raise Reply(409, {"errors": {"email": ["is already registered"]}})
        salt = secrets.token_hex(8)
        cursor = db.execute("INSERT INTO users (email, salt, hash) VALUES (?, ?, ?)",
                            (email, salt, password_hash(body["password"], salt)))
        raise Reply(201, {"id": cursor.lastrowid, "email": email})
    if parts == ["login"] and method == "POST":
        email = body.get("email", "").strip().lower() if isinstance(body.get("email"), str) else ""
        user = one("SELECT * FROM users WHERE email = ?", email)
        password = body.get("password") if isinstance(body.get("password"), str) else ""
        if user is None or password_hash(password, user["salt"]) != user["hash"]:
            raise Reply(401, {"error": "invalid credentials"})
        token = secrets.token_urlsafe(24)
        db.execute("INSERT INTO tokens (token, user_id) VALUES (?, ?)", (token, user["id"]))
        raise Reply(200, {"token": token})
    if parts == ["logout"] and method == "POST":
        current_user(headers)
        db.execute("DELETE FROM tokens WHERE token = ?", (headers["Authorization"].split(" ", 1)[1],))
        raise Reply(204)
    raise Reply(404, {"error": "not found"})


def notes(method, parts, query, body, headers):
    user_id = current_user(headers)
    body = body if isinstance(body, dict) else {}
    if parts == ["notes"] and method == "GET":
        rows = db.execute("SELECT * FROM notes WHERE user_id = ? ORDER BY created_at DESC, id DESC", (user_id,))
        raise Reply(200, [note_json(row) for row in rows])
    if parts == ["notes"] and method == "POST":
        if not valid_title(body.get("title")):
            invalid("title", "must be 1-200 characters")
        note_body = body.get("body", "")
        if not isinstance(note_body, str):
            invalid("body", "must be a string")
        cursor = db.execute("INSERT INTO notes (user_id, title, body, created_at) VALUES (?, ?, ?, ?)",
                            (user_id, body["title"], note_body, now()))
        raise Reply(201, note_json(one("SELECT * FROM notes WHERE id = ?", cursor.lastrowid)))
    row = one("SELECT * FROM notes WHERE id = ? AND user_id = ?", parts[1], user_id) if len(parts) == 2 else None
    if row is None:
        raise Reply(404, {"error": "not found"})
    if method == "GET":
        raise Reply(200, note_json(row))
    if method == "PATCH":
        if "title" in body and not valid_title(body["title"]):
            invalid("title", "must be 1-200 characters")
        if "body" in body and not isinstance(body["body"], str):
            invalid("body", "must be a string")
        db.execute("UPDATE notes SET title = ?, body = ? WHERE id = ?",
                   (body.get("title", row["title"]), body.get("body", row["body"]), row["id"]))
        raise Reply(200, note_json(one("SELECT * FROM notes WHERE id = ?", row["id"])))
    if method == "DELETE":
        db.execute("DELETE FROM notes WHERE id = ?", (row["id"],))
        raise Reply(204)
    raise Reply(405, {"error": "method not allowed"})


# --------------------------------------------------------------- blog-search

def posts(method, parts, query, body, headers):
    if parts == ["tags"] and method == "GET":
        counts = {}
        for row in db.execute("SELECT tags FROM posts"):
            for tag in json.loads(row["tags"]):
                counts[tag] = counts.get(tag, 0) + 1
        ordered = sorted(counts.items(), key=lambda item: (-item[1], item[0]))
        raise Reply(200, [{"name": name, "count": count} for name, count in ordered])
    if parts == ["posts"] and method == "POST":
        body = body if isinstance(body, dict) else {}
        if not valid_title(body.get("title")):
            invalid("title", "must be 1-200 characters")
        if parse_iso(body.get("published_at")) is None:
            invalid("published_at", "must be an ISO 8601 timestamp")
        tags = body.get("tags", [])
        if not isinstance(tags, list) or not all(isinstance(tag, str) for tag in tags):
            invalid("tags", "must be an array of strings")
        normalised = list(dict.fromkeys(tag.strip().lower() for tag in tags if tag.strip()))
        post_body = body.get("body", "")
        cursor = db.execute("INSERT INTO posts (title, body, tags, published_at) VALUES (?, ?, ?, ?)",
                            (body["title"], post_body, json.dumps(normalised), body["published_at"]))
        raise Reply(201, post_json(one("SELECT * FROM posts WHERE id = ?", cursor.lastrowid)))
    if parts == ["posts"] and method == "GET":
        rows = [post_json(row) for row in db.execute("SELECT * FROM posts")]
        if "tag" in query:
            tag = query["tag"][0].strip().lower()
            rows = [post for post in rows if tag in post["tags"]]
        if "q" in query:
            word = query["q"][0].lower()
            rows = [post for post in rows if word in post["title"].lower() or word in post["body"].lower()]
        rows.sort(key=lambda post: parse_iso(post["published_at"]), reverse=True)
        page = max(1, int(query.get("page", ["1"])[0]))
        per_page = min(50, max(1, int(query.get("per_page", ["10"])[0])))
        items = rows[(page - 1) * per_page: page * per_page]
        raise Reply(200, {"items": items, "total": len(rows), "page": page, "per_page": per_page})
    if len(parts) == 2 and parts[0] == "posts" and method == "GET":
        row = one("SELECT * FROM posts WHERE id = ?", parts[1])
        raise Reply(200, post_json(row)) if row else Reply(404, {"error": "not found"})
    raise Reply(404, {"error": "not found"})


ROUTES = {"todos": todos, "signup": auth, "login": auth, "logout": auth, "notes": notes, "posts": posts, "tags": posts}


class Handler(BaseHTTPRequestHandler):
    def handle_any(self):
        url = urlparse(self.path)
        parts = [part for part in url.path.split("/") if part]
        length = int(self.headers.get("Content-Length") or 0)
        try:
            body = json.loads(self.rfile.read(length)) if length else None
        except json.JSONDecodeError:
            body = None
        try:
            route = ROUTES.get(parts[0]) if parts else None
            if route is None:
                raise Reply(404, {"error": "not found"})
            with lock:
                try:
                    route(self.command, parts, parse_qs(url.query), body, self.headers)
                except Reply as reply:  # replies are raised; only a success keeps its writes
                    (db.commit if reply.status < 400 else db.rollback)()
                    raise
                except Exception:
                    db.rollback()
                    raise
            raise Reply(500, {"error": "no reply"})
        except Reply as reply:
            payload = b"" if reply.body is None else json.dumps(reply.body).encode()
            self.send_response(reply.status)
            if payload:
                self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

    do_GET = do_POST = do_PATCH = do_DELETE = handle_any

    def log_message(self, *args):
        pass


if __name__ == "__main__":
    ThreadingHTTPServer(("127.0.0.1", int(os.environ["PORT"])), Handler).serve_forever()
