# Views

This directory holds ERB-style templates with the `.html.slv` extension.

**Convention**: a controller action renders `app/views/<controller>/<action>.html.slv`
by default. `PostsController#show` → `app/views/posts/show.html.slv`.
Partials live next to their parent view and start with an underscore:
`_post.html.slv`.

## Template tags

| Tag                  | Behavior                                                       |
|----------------------|----------------------------------------------------------------|
| `<%= expr %>`        | Evaluate and **HTML-escape**. Safe default for user input.      |
| `<%- expr %>`        | Evaluate, output **raw** (no escaping). Trusted HTML only.      |
| `<% stmt %>`         | Evaluate; no output. Used for control flow.                     |
| `<%# comment %>`     | Comment — content is silently dropped, even across lines.       |

`<%== expr %>` was removed (SEC-023). If you need to render pre-escaped HTML
(e.g. a Markdown render), use `<%- html %>` *or* `<%= html_unescape(html) %>`,
not `<%==`.

XSS is the default risk. Default to `<%= %>` for anything that touches user
content; reach for `<%- %>` only when you can prove the value is trusted.

## Locals — three ways to pass data into a template

1. **Implicit via `@`-variables on the controller** (recommended; covers most
   cases). Anything you assign to `@field` inside an action becomes available
   as both `@field` and bare `field` in the template — no need to pass it in
   `render(...)`'s data hash. Underscore-prefixed fields (`@_internal`) are
   private and not exposed.

   ```soli
   # in the controller
   def show
     @post = Post.find(params["id"])
     @title = "Read \"#{@post.title}\""
   end
   ```

   ```erb
   <h1><%= @title %></h1>
   <article><%= @post.body %></article>
   ```

2. **Explicit data hash on `render`**. Use this when you want to render a
   different template, or pass a value under a name that doesn't match a
   controller field.

   ```soli
   render("posts/show_summary", { "summary_text": build_summary(@post) })
   ```

3. **`locals[...]` for collisions**. When a local name shadows a builtin or
   helper, read it through the `locals` hash instead of bare. `locals` is
   always defined (even when no data was passed).

   ```erb
   <%# `partial` is a builtin; locals["partial"] disambiguates %>
   <%= locals["partial"] %>
   ```

## Iteration

```erb
<% for post in @posts %>
  <article>
    <h2><%= h(post.title) %></h2>
    <%= post.body %>
  </article>
<% end %>

<% for post, i in @posts %>
  <p>#<%= i %>: <%= h(post.title) %></p>
<% end %>

<% if @posts.length() == 0 %>
  <p>No posts yet.</p>
<% end %>
```

`<% xs.each do |x| %> ... <% end %>` does **NOT** work inside ERB. The
template engine only recognises `for x in xs` / `for x, i in xs` as the loop
syntax. Use `each` in regular `.sl` code, `for` in templates.

## Escape helpers

Pick the helper that matches the *context* where the value lands. `<%= %>`
calls `h()` automatically, which is right for HTML body text but wrong for
attributes / JS / URLs.

| Helper                   | Use when the value goes into…                                       |
|--------------------------|----------------------------------------------------------------------|
| `h(value)`               | HTML body text (default — auto-applied by `<%= %>`).                 |
| `attr(value)`            | An HTML attribute. Escapes `"`, `'`, `<`, `>`, `&`.                  |
| `j(value)`               | A `<script>` block or inline JS.                                      |
| `url(value)`             | A query-string or path component.                                    |
| `sanitize_html(value)`   | Allow some tags (links, basic formatting), strip the rest.            |
| `strip_html(value)`      | Remove all tags, keep text content.                                  |
| `html_unescape(value)`   | Decode `&amp;` / `&lt;` etc. back to characters.                     |

```erb
<a href="<%= attr(post.url) %>" title="<%= attr(post.title) %>">read</a>

<script>
  window.post = { id: <%= post.id %>, title: <%= j(post.title) %> };
</script>

<a href="/search?q=<%= url(params["q"]) %>">search</a>

<%# user-authored rich text: sanitize, then emit as raw output %>
<%- sanitize_html(post.body_html) %>
```

## Partials

Render a partial with `partial("dir/name", { "local_key": value })`. The
template file **must** be named `_name.html.slv` — the leading underscore is
mandatory. `render_partial(...)` is an alias for the same builtin.

For named, reusable UI pieces (cards, rows, badges, etc.) prefer the dedicated component helper instead:

```erb
<%- component("user_card", { "user": user }) %>
```

Components are looked up in `app/views/components/` (subdirectories allowed). See the full "Components" section in the Views documentation.

Both partials and components inherit the controller's `@instance` variables, so you can read `@current_user` inside them without passing it explicitly (an explicit local of the same name wins).

```erb
<% for post in @posts %>
  <%- partial("posts/post", { "post": post }) %>
<% end %>
```

`partial` returns HTML, so render it with `<%-` (raw output). `<%=` would
HTML-escape the partial's own markup and show tags as text.

> **`<%-` skips escaping — sanitize untrusted HTML first.** `<%-` is safe for
> trusted, framework-produced HTML (like a `partial`). For any HTML that
> originated from a user (rich-text fields, imported content, Markdown
> rendered to HTML), pass it through `sanitize_html(value)` first — it keeps
> safe tags (links, basic formatting) and strips scripts/event handlers/
> `javascript:` URLs. Use `strip_html(value)` instead when you want plain text
> with no tags at all. Never put raw user HTML straight into `<%-`.

`app/views/posts/_post.html.slv`:

```erb
<article>
  <h2><%= h(post.title) %></h2>
  <%= h(post.body) %>
</article>
```

Inside the partial, `post` is read as a bare local. Use `locals["post"]` if
the name collides with anything.

## View helpers

Always available inside templates:

| Helper                                     | What it does                                                       |
|--------------------------------------------|--------------------------------------------------------------------|
| `partial("dir/name", { ... })`             | Render a partial.                                                  |
| `public_path("css/app.css")`               | Cache-busted asset URL (fingerprinted).                            |
| `upload_url(record, "field", opts?)`       | URL for an uploader-managed blob (with optional transforms — see **Rendering uploads**). |
| `t("posts.title")`                         | i18n lookup; respects current locale.                              |
| `time_ago(timestamp)`                      | "3 minutes ago"-style relative time.                                |
| `current_path()`                           | Current request path.                                              |
| `current_method()`                         | Current HTTP method (`"GET"`, `"POST"`, ...).                       |
| `current_path?(path)`                      | Boolean — is the request on this path?                              |
| `range(start, stop)`                       | `[start, ..., stop-1]` for `for i in range(0, 5)`.                  |
| `sanitize_html(html)` / `strip_html(html)` | See "Escape helpers".                                              |
| `dev_queries()`                            | AQL stack for the current request (`--dev` only — `[]` otherwise). |

**Named route helpers** (`posts_path()`, `post_path(post)`, `new_post_path()`,
`edit_post_path(post)`, plus `*_url` variants) come from `resources(...)` in
`config/routes.sl`. Use them in templates — never hand-build URLs.

```erb
<a href="<%= posts_path() %>">All posts</a>
<a href="<%= post_path(post) %>">Read</a>
<a href="<%= edit_post_path(post) %>">Edit</a>
```

There is **no `link_to` helper** in Soli — write the `<a>` tag yourself with
`<%= attr(...) %>` around URLs that contain user data.

## Rendering uploads and image transforms

For attachment fields declared with `uploader(...)` on a model (see
`app/models/CLAUDE.md` → **Attachments and uploads**), the auto-generated
`<field>_url` instance method — or the standalone `upload_url(record, "field")`
helper — returns a URL served by the framework's `AttachmentsController`.
The URL hits SoliDB, decodes the blob, and streams it back.

```erb
<% if !@contact.photo_url().nil? %>
  <img src="<%= attr(@contact.photo_url()) %>" alt="<%= attr(@contact.name) %>">
<% end %>
```

For a `multiple: true` field, the per-record getter returns `nil` — iterate
the stored blob ids and ask for each URL:

```erb
<% for blob_id in (@document.attachments_blob_ids ?? []) %>
  <li>
    <a href="<%= attr(@document.attachments_url(blob_id)) %>">Download</a>
  </li>
<% end %>
```

### Image transforms via query string

When the stored blob is an image (`Content-Type: image/*`), the URL accepts a
small set of query parameters and the controller pipes the blob through
Soli's `Image` builtin before responding. Browsers cache each (URL,
query-string) combination independently — same params → cache hit, no
re-transformation cost. Failed transforms fall back to streaming the original
bytes.

Pass options as a hash to `upload_url` / `<field>_url`:

```erb
<img src="<%= attr(@contact.photo_url({ "thumb": 120 })) %>">

<img src="<%= attr(@contact.photo_url({ "w": 400, "h": 300, "fit": "cover" })) %>">

<img src="<%= attr(@contact.photo_url({ "square": 64, "fmt": "webp", "q": 80 })) %>">
```

| Key                  | Effect                                                                  |
|----------------------|--------------------------------------------------------------------------|
| `w`, `h`             | Resize to those dimensions (may distort if both set without `fit`).      |
| `thumb`              | Square-fit thumbnail, max edge = value.                                  |
| `square`             | Sugar for `w=N&h=N&fit=cover` — square crop to N×N.                      |
| `fit`                | `"cover"` (fill + center-crop) or `"contain"` (fit inside). Needs `w`+`h`.|
| `crop`               | `"x,y,w,h"` — pick a source region first; runs before sizing.            |
| `flipx`, `flipy`     | Horizontal / vertical flip.                                              |
| `rot`                | `90`, `180`, or `270` — quarter-turn rotation.                            |
| `blur`               | Gaussian blur sigma (float).                                             |
| `bright`             | Brightness delta (signed int).                                           |
| `contrast`           | Contrast factor (float; 1.0 = unchanged).                                |
| `hue`                | Hue rotation in degrees.                                                 |
| `gray`               | Truthy → grayscale.                                                       |
| `invert`             | Truthy → invert colors.                                                   |
| `fmt`                | Re-encode: `"webp"`, `"png"`, `"jpeg"`.                                   |
| `q`                  | Output quality (int, encoder-specific).                                   |

Pipeline order is fixed: `crop` → `flip`/`rot` → resize (`thumb` / `fit` /
`w+h` / `square`) → effects (`blur`, `bright`, `contrast`, `hue`, `invert`,
`gray`) → encode (`fmt`, `q`). Width/height are clamped server-side to 1000 px
so a hand-crafted URL can't drive a multi-GB allocation.

For ad-hoc image processing outside the uploader pipeline (e.g. a job that
generates a derivative and stores it under another field), use the `Image`
builtin directly:

```soli
img = Image.from_buffer(file["data"])
thumb_b64 = img.thumbnail(200).format("webp").quality(80).to_buffer()
```

`Image` is mostly used in models/jobs, not in views — keep templates thin.

## App-level helpers

Anything you put in `app/helpers/*.sl` is auto-loaded and available inside
every template. No `import` needed. Define a helper as a free-standing
function:

```soli
# app/helpers/markdown_helpers.sl

def render_markdown(text)
  # ... call your markdown engine ...
end
```

Then in a view:

```erb
<%- render_markdown(@post.body) %>
```

Group helpers thematically — `application_helper.sl`, `markdown_helpers.sl`,
`form_helpers.sl` — rather than one mega-file.

## Client interactivity — HTMX + Alpine.js

Every scaffolded app ships with **HTMX** and **Alpine.js** preloaded in
`app/views/layouts/application.html.slv`:

```erb
<script defer src="<%= public_path("js/htmx.min.js") %>"></script>
<script defer src="<%= public_path("js/alpine.min.js") %>"></script>
```

This is Soli's default client stack. Reach for it before introducing React /
Vue / similar — most "make this page interactive" tasks fit it cleanly,
without a build step.

**HTMX** for server-driven partials. Swap a piece of the DOM with the HTML
response of a Soli action — no JSON, no client-side templating:

```erb
<button hx-post="<%= post_path(post) %>/like"
        hx-target="#like-count"
        hx-swap="outerHTML">
  Like
</button>

<span id="like-count"><%= post.likes %></span>
```

Pair with a controller that returns the partial fragment directly:

```soli
def like
  @post = Post.find(params["id"])
  @post.increment("likes")
  render("posts/_like_count")     # tiny partial, no layout
end
```

For HTMX requests, skip the layout by detecting the `HX-Request` header in
the controller (e.g. `if req["headers"]["hx-request"] == "true"`) and pass
`{ "layout": false }` to `render`.

**Alpine.js** for purely-local UI state (open/closed, expanded/collapsed,
in-input validation flash). Keep it scoped — no app-level Alpine stores.

```erb
<div x-data="{ open: false }">
  <button @click="open = !open">Toggle</button>
  <div x-show="open">…content…</div>
</div>
```

Rule of thumb: **server first**. If a piece of state lives on the server
(user data, validations, search results), use HTMX. If it lives only in the
DOM (modals, dropdowns, tabs), use Alpine. Mix freely — they don't conflict.

See `docs/client-interactivity.md` (bundled in every Soli app) for the
deeper end of either library.

## Layouts

Every render runs inside a layout unless explicitly opted out.

- **Default**: `app/views/layouts/application.html.slv`.
- **Per-controller**: set `this.layout = "X"` in the controller's `static { }`
  block to use `app/views/layouts/X.html.slv` instead.
- **Per-render**: pass `"layout"` in the data hash:
  `render("posts/show", { "layout": "minimal" })`.
- **No layout**: `render("posts/show", { "layout": false })`.

The layout calls `<%= yield %>` (or the equivalent `<%= content %>` local)
where the view's content gets inserted.

```erb
<!-- app/views/layouts/application.html.slv -->
<!doctype html>
<html>
  <head>
    <title><%= @title ?? "MyApp" %></title>
    <link rel="stylesheet" href="<%= public_path("css/app.css") %>">
  </head>
  <body>
    <%= yield %>
  </body>
</html>
```

## File layout

```
app/views/
├── layouts/
│   └── application.html.slv      # wrap-around for all renders by default
├── posts/
│   ├── index.html.slv
│   ├── show.html.slv
│   ├── new.html.slv
│   ├── edit.html.slv
│   └── _post.html.slv            # partial — underscore prefix is mandatory
└── shared/
    └── _nav.html.slv             # cross-cutting partials live here
```

`render("posts/show")` → `app/views/posts/show.html.slv`.
`partial("posts/post", ...)` → `app/views/posts/_post.html.slv`.

## Style

- **Indent at 2 spaces** in `.slv` files — matches the docs site convention,
  keeps diffs and partials consistent.
- One template per action; pull cross-cutting markup into a partial.
- Keep logic out of templates. If a `<% %>` block grows past a few lines,
  move it to a helper or a controller `@field`.
- Always close your tags. `<% if %>` needs `<% end %>`; `<% for %>` needs
  `<% end %>`.

## Do / Don't

| Do                                                       | Don't                                                              |
|----------------------------------------------------------|--------------------------------------------------------------------|
| Use `<%= %>` by default                                  | Use `<%- %>` for values that touched user input                     |
| Use `attr(...)` / `j(...)` / `url(...)` for non-body contexts | Trust `h()` to be safe inside attributes — it's HTML-body-only |
| Set `@field` in the controller and read it in the view   | Re-pass `@field` in the `render(...)` data hash                     |
| Use named route helpers — `post_path(post)`              | Hand-build `"/posts/" + str(post.id)`                               |
| Use `for x in xs` for loops                              | Try `<% xs.each do |x| %>` — that doesn't parse inside ERB         |
| Use `#{expr}` for interpolation in strings inside `.sl` blocks | Use `\(expr)` — the lexer rejects that                       |
| Keep templates thin; push logic into helpers              | Embed business rules in `<% %>` blocks                              |
| Put cross-cutting markup in `_partials.html.slv`         | Copy-paste header/nav across every action's view                    |
