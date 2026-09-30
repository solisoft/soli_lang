"""Acceptance checks for specs/blog-search/SPEC.md. Each check gets a Tester `t`."""


def _post(t, title="Hello", published_at="2026-01-01T10:00:00Z", **extra):
    status, body = t.post("/posts", {"title": title, "published_at": published_at, **extra})
    t.expect(status == 201, f"POST /posts -> {status} {body!r}")
    return body


def _page(t, query):
    status, body = t.get(f"/posts?{query}")
    t.expect(status == 200 and isinstance(body, dict), f"GET /posts?{query} -> {status} {body!r}")
    for key in ("items", "total", "page", "per_page"):
        t.expect(key in body, f"missing {key!r} in {body!r}")
    return body


def create_normalises_tags(t):
    post = _post(t, "Tags", tags=["Rust", " rust ", "Web"])
    t.expect(post.get("tags") == ["rust", "web"], f"tags not normalised: {post.get('tags')!r}")
    t.expect(post.get("body") == "", f"body should default to \"\": {post!r}")
    t.expect(t.has_id(post), f"no id in {post!r}")


def create_validates(t):
    for payload in ({"published_at": "2026-01-01T10:00:00Z"},
                    {"title": "", "published_at": "2026-01-01T10:00:00Z"},
                    {"title": "no date"},
                    {"title": "bad date", "published_at": "yesterday"},
                    {"title": "bad tags", "published_at": "2026-01-01T10:00:00Z", "tags": "rust"},
                    {"title": "bad tags", "published_at": "2026-01-01T10:00:00Z", "tags": [1, 2]}):
        status, body = t.post("/posts", payload)
        t.expect(status == 422, f"{payload!r} -> {status}, expected 422")
        t.expect(isinstance(body, dict) and "errors" in body, f"{payload!r}: no errors key in {body!r}")


def show_and_404(t):
    post = _post(t, "Findable")
    status, body = t.get(f"/posts/{post['id']}")
    t.expect(status == 200 and body.get("title") == "Findable", f"GET -> {status} {body!r}")
    status, _ = t.get("/posts/999999999")
    t.expect(status == 404, f"unknown id -> {status}, expected 404")


def tag_filter_sorts_newest_first(t):
    tag = f"sort{t.nonce}"
    middle = _post(t, "middle", "2025-06-01T00:00:00Z", tags=[tag])["id"]
    newest = _post(t, "newest", "2026-03-01T00:00:00Z", tags=[tag])["id"]
    oldest = _post(t, "oldest", "2024-01-01T00:00:00Z", tags=[tag])["id"]
    page = _page(t, f"tag={tag.upper()}")
    t.expect(page["total"] == 3, f"total {page['total']}, expected 3")
    t.expect([post["id"] for post in page["items"]] == [newest, middle, oldest],
             f"expected newest→oldest, got {[post.get('title') for post in page['items']]}")


def pagination(t):
    tag = f"page{t.nonce}"
    for i in range(12):
        _post(t, f"post {i}", f"2026-02-{i + 1:02d}T00:00:00Z", tags=[tag])
    page = _page(t, f"tag={tag}")
    t.expect(page["page"] == 1 and page["per_page"] == 10 and len(page["items"]) == 10,
             f"defaults: page={page['page']} per_page={page['per_page']} items={len(page['items'])}")
    page = _page(t, f"tag={tag}&per_page=5&page=3")
    t.expect(len(page["items"]) == 2 and page["total"] == 12, f"page 3 of 5: {len(page['items'])} items, total {page['total']}")
    t.expect(page["items"][0]["title"] == "post 1", f"page 3 should start at 'post 1': {page['items'][0].get('title')!r}")
    page = _page(t, f"tag={tag}&per_page=5&page=4")
    t.expect(page["items"] == [] and page["total"] == 12, f"past the end: {page!r}")
    page = _page(t, f"tag={tag}&per_page=500")
    t.expect(page["per_page"] == 50 and len(page["items"]) == 12, f"per_page cap: {page['per_page']}, {len(page['items'])} items")


def search_combines_with_tag(t):
    word = f"zanzibar{t.nonce}"
    tag = f"search{t.nonce}"
    in_title = _post(t, f"Trip to {word.capitalize()}", tags=[tag])["id"]
    in_body = _post(t, "Notes", body=f"remember {word.upper()} next year", tags=[tag])["id"]
    _post(t, "Unrelated", body="nothing here", tags=[tag])
    _post(t, f"Other {word}", tags=["elsewhere"])
    page = _page(t, f"tag={tag}&q={word.capitalize()}")
    found = sorted(str(post["id"]) for post in page["items"])
    t.expect(found == sorted([str(in_title), str(in_body)]) and page["total"] == 2,
             f"q+tag found {[post.get('title') for post in page['items']]} total {page['total']}")


def tags_are_counted_and_sorted(t):
    busy, quiet = f"busy{t.nonce}", f"quiet{t.nonce}"
    for i in range(3):
        _post(t, f"busy {i}", tags=[busy])
    _post(t, "quiet", tags=[quiet, busy])
    status, tags = t.get("/tags")
    t.expect(status == 200 and isinstance(tags, list), f"GET /tags -> {status} {tags!r}")
    counts = {tag.get("name"): tag.get("count") for tag in tags}
    t.expect(counts.get(busy) == 4 and counts.get(quiet) == 1, f"counts busy={counts.get(busy)} quiet={counts.get(quiet)}")
    order = [(-tag["count"], tag["name"]) for tag in tags]
    t.expect(order == sorted(order), "not sorted by count desc, then name asc")


def survives_restart(t):
    post = _post(t, "Durable")
    t.restart()
    status, body = t.get(f"/posts/{post['id']}")
    t.expect(status == 200 and body.get("title") == "Durable", f"after restart -> {status} {body!r}")


CHECKS = [
    create_normalises_tags,
    create_validates,
    show_and_404,
    tag_filter_sorts_newest_first,
    pagination,
    search_combines_with_tag,
    tags_are_counted_and_sorted,
    survives_restart,
]
