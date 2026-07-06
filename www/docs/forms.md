# Forms & CSRF Protection

Soli ships a Rails-style **form builder** for `.html.slv` views: `form_with`
derives the action URL and HTTP verb from a model record, prefills field
values, renders validation errors, and embeds a per-session **CSRF token**
that the server verifies on submit. HTML forms can only express GET and POST,
so the builder also emits the hidden `_method` field the server honors to
route PUT/PATCH/DELETE.

## Quick start

```erb
<%- form_with(post) do |f| -%>
  <%- f.error_summary() %>

  <%- f.label("title") %>
  <%- f.text_field("title", {"placeholder": "Title"}) %>
  <%- f.errors_for("title") %>

  <%- f.text_area("body") %>
  <%- f.check_box("published") %>
  <%- f.submit("Save") %>
<%- end -%>
```

The `do |f|` block binds the builder (name it whatever you like, or write a
bare `do` for an implicit `f`); the block emits the `<form>` tag, the hidden
`_method` override, and the CSRF token before the body, and `</form>` after
it. The opener reads naturally in any tag style (`<% %>`, `<%= %>`, `<%- %>`),
and a `-%>` closer swallows the newline that follows the tag — on any
template tag — so blocks don't leave blank lines in the output.

Use `<%-` (raw output) for every builder call — the helpers return HTML.
`<%=` would escape it into visible text.

Prefer the block form. The explicit builder is still there when you need it
(a form assembled across non-contiguous markup):

```erb
<% f = form_with(post) %>
<%- f.open() %>
  ...
<%- f.close() %>
```

With a **new record**, `f.open()` renders a form that POSTs to
`/<collection>`; with a **persisted record** (one that has a `_key`) it
targets `/<collection>/<key>` and emits `<input type="hidden" name="_method"
value="PATCH">` so the POST routes to your `update` action. The per-session
CSRF token rides along as a hidden `_csrf_token` input on every non-GET form.

## form_with

```erb
<% f = form_with(record, options) %>
```

Both arguments are optional. With no record, pass `"url"`:

```erb
<% f = form_with(null, {"url": "/search", "method": "get"}) %>
```

| Option | Effect |
|--------|--------|
| `"url"` | Override the derived action URL (required with no record) |
| `"method"` | `"post"` / `"patch"` / `"put"` / `"delete"` / `"get"` — overrides the derived verb |
| `"multipart": true` | Adds `enctype="multipart/form-data"` (required for `file_field`) |
| anything else | Becomes an attribute on the `<form>` tag (`"class"`, `"id"`, `data-*`, …) |

GET forms skip both the CSRF token and the `_method` field.

## Field helpers

Every field helper takes `(field, options)`; options become HTML attributes
(`true` renders a bare attribute, `false`/`nil` skips it), with `"value"` and
`"class"` treated specially. Values prefill from the record (`record[field]`)
and are always HTML-escaped.

| Helper | Renders |
|--------|---------|
| `f.text_field("title")` | `<input type="text">` |
| `f.email_field("email")` | `<input type="email">` |
| `f.password_field("password")` | `<input type="password">` — never prefills |
| `f.number_field("age")` | `<input type="number">` |
| `f.date_field("due_on")` | `<input type="date">` |
| `f.datetime_field("starts_at")` | `<input type="datetime-local">` |
| `f.hidden_field("token")` | `<input type="hidden">` |
| `f.file_field("avatar")` | `<input type="file">` — pair with `"multipart": true` |
| `f.text_area("body")` | `<textarea>` with the escaped value as content |
| `f.check_box("published")` | checkbox, `value="true"`, checked when the field is `true`/`"true"` |
| `f.radio_button("size", "xl")` | radio, checked when the field equals the value |
| `f.select("status", choices)` | `<select>` with the current value `selected` |
| `f.label("title", text?, opts?)` | `<label for="title">` — text defaults to a humanized field name |
| `f.submit("Save", opts?)` | `<button type="submit">` |

`select` accepts an array of strings or of `[label, value]` pairs:

```erb
<% choices = [ ["On time", "up"], ["Late", "late"] ] %>
<%- f.select("status", choices) %>
```

Two gotchas: build the pairs in a `<% %>` code block (complex nested literals
don't parse inside output tags), and put a space between the brackets —
a leading `[[` lexes as a Lua-style raw string, not a nested array.

Field names are **flat** (`name="title"`), matching Soli's flat `params`
model — your controller reads `params["title"]` exactly as before. An
unchecked `check_box` submits nothing; read it as
`params["published"] == "true"`.

## Validation errors

When a record failed a `create`/`save`, its `_errors` drive three things:

- `f.error_summary(opts?)` — a `<div class="form-errors"><ul>…` listing every
  message (renders nothing for a valid record). `{"class": "…"}` restyles it.
- `f.errors_for("title")` — inline `<span class="field-error-message">` per
  message on that field.
- Any field helper for an errored field gains a `field-error` class and
  `aria-invalid="true"`.

```soli
def create(req)
  post = Post.create(this._permit_params(params))
  if post._errors
    return render("posts/new", { "post": post })
  end

  return redirect("/posts/" + post._key)
end
```

## button_to

State-changing links (delete buttons, logout) belong in forms, not `<a>`
tags. `button_to` renders a one-button form with the CSRF token and method
override built in:

```erb
<%- button_to("Delete", "/posts/" + post["_key"].to_s, {
  "method": "delete", "confirm": "Are you sure?",
  "class": "btn-danger", "form_class": "inline"
}) %>
```

`"confirm"` wraps the submit in a JS `confirm()`; `"form_class"` styles the
`<form>`; every other option becomes a button attribute.

## Method override (`_method`)

Browsers only submit GET and POST. A POST whose form body carries
`_method=PUT|PATCH|DELETE` is routed — and dispatched to your controller —
as that verb, so `resources("posts")` update/destroy routes work from plain
HTML forms. Only those three verbs are honored (no downgrading to GET), only
on POST, and only for form content types (`application/x-www-form-urlencoded`
or `multipart/form-data`) — JSON APIs are never affected. The builder and
`button_to` emit the field for you.

## CSRF tokens

Soli's baseline CSRF protection is the Origin/Referer same-site gate (see
[Routing — CSRF](/docs/core-concepts/routing)). Forms add a second,
Rails-style layer:

- `csrf_token()` — the per-session token (builtin, available in controllers
  and views; created on first use).
- `csrf_field()` — hidden `_csrf_token` input; `f.open()` and `button_to`
  embed it automatically.
- `csrf_meta_tag()` — `<meta name="csrf-token">` for layouts, so JS clients
  (fetch/htmx) can send the `X-CSRF-Token` header.

On any state-changing request that **carries** a token — the `_csrf_token`
form field or the `X-CSRF-Token` header — the server verifies it against the
session with a constant-time compare and rejects a mismatch with `403`, even
when the Origin check passed. Requests without a token keep the
Origin/Referer posture, so existing apps and JSON APIs are unaffected.

To make tokens mandatory for browser form posts, set:

```bash
SOLI_CSRF_TOKENS=require   # form posts without a valid token → 403
```

JSON/API traffic is never token-gated; `skip_csrf("/path")` and
`SOLI_DISABLE_CSRF` opt-outs apply to both layers.

For htmx, wire the header once:

```html
<body hx-headers='{"X-CSRF-Token": "<%= csrf_token() %>"}'>
```

## Partials

Partials render in a fresh scope, so pass the builder explicitly:

```erb
<%- form_with(post) do |f| -%>
  <%- partial("posts/form", { "post": post, "f": f }) %>
<%- end -%>
```

## See also

- [Views](/docs/core-concepts/views) — templates, layouts, partials
- [Routing](/docs/core-concepts/routing) — `resources`, CSRF origin gate
- [Sessions](/docs/security/sessions) — where the token lives
