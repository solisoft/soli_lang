# Views

ERB-style templates with the `.html.slv` extension. Path: `app/views/<controller>/<action>.html.slv`.

## Tags

- `<%= expr %>` — evaluate and HTML-escape (default; safe for user input).
- `<%= raw(expr) %>` — evaluate without escaping. **Only** for trusted HTML (markdown render, partials, helper output).
- `<% stmt %>` — evaluate, no output.
- `<%- expr %>` — raw output (legacy alias for `raw`).

Always wrap user content with `h(...)` or rely on `<%= %>`'s default escape. XSS is the default risk.

## Locals

- The second arg to `render("name", { "key": value })` becomes locals.
- Controller assignments to `this.foo = ...` are also auto-exposed — no need to pass them again.
- If a local name collides with a builtin/helper, read it via `locals["key"]`.

## Control flow

```erb
<% for post in posts %>
  <article>
    <h2><%= post.title %></h2>
    <%= raw(post.body_html) %>
  </article>
<% end %>

<% if posts.length() == 0 %>
  <p>No posts yet.</p>
<% end %>
```

`xs.each do |x| %>` does NOT work inside ERB — use `<% for x in xs %>...<% end %>`.

## Helpers to reach for

- `link_to("New post", new_post_path())` — anchor with named-helper URL.
- `public_path("css/application.css")` — cache-busted asset URL.
- `partial("dir/name", { "key": value })` — render `_name.html.slv` from `dir/`.
- `t("posts.title")` — i18n lookup.

## Style

Indent at **2 spaces** inside `.slv`. Match the docs site convention — keeps diffs and partials consistent.
