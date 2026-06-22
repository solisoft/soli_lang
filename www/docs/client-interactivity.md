# Client Interactivity

Soli's default project template ships two complementary client-side libraries that, together, cover the bulk of frontend work without a JavaScript build step:

| Library                                            | Role                                       | Size (vendored) |
|----------------------------------------------------|--------------------------------------------|-----------------|
| [HTMx](https://htmx.org/) `v2.0.10`                | AJAX over HTML attributes (server round-trips) | ~50 kB         |
| [Alpine.js](https://alpinejs.dev/) `v3.14.1`       | Local UI state and behavior                | ~45 kB         |

> **HTMx v2 note:** v2 dropped IE support, removed `hx-vars` / `hx-encoded`, and moved WebSocket/SSE to extensions. If you're porting v1 examples from the web, double-check those edges. The directive set used in this page (`hx-get`, `hx-post`, `hx-target`, `hx-swap`, `hx-trigger`, `hx-push-url`) is identical between v1 and v2.

Both files live in `public/js/` and are loaded with `<script defer>` at the bottom of `app/views/layouts/application.html.slv`. There is no build step, no CDN dependency, and no version drift.

Use this combination when [Live View](/docs/core-concepts/liveview) (real-time, server-stateful, WebSocket) is heavier than you need — most CRUD apps, dashboards, marketing sites, and admin pages.

## When to Use What

```
local UI state            ──►  Alpine          (toggle, tabs, modal, dropdown)
server round-trip         ──►  HTMx            (submit form, swap fragment)
optimistic UI + server    ──►  HTMx + Alpine   (flip locally, reconcile on swap)
real-time / multi-user    ──►  Live View       (WebSocket diffs)
```

A rough decision tree: if the interaction has no server side, use Alpine alone. If it has a server side but doesn't need WebSocket persistence, use HTMx (and add Alpine if you want optimistic feedback). Reach for Live View when you genuinely need server-pushed updates or multi-client collaboration.

## HTMx — AJAX in HTML Attributes

HTMx lets any element fire HTTP requests and swap the response into the DOM. No client-side JavaScript needed:

```html
<button hx-get="/users" hx-target="#user-list" hx-swap="innerHTML">
  Load Users
</button>
<div id="user-list"></div>
```

### Common Directives

- `hx-get="/url"`, `hx-post="/url"`, `hx-put`, `hx-patch`, `hx-delete` — fire the request.
- `hx-target="#selector"` — where the response HTML goes.
- `hx-swap="outerHTML | innerHTML | beforeend | afterbegin | none"` — how the response replaces the target.
- `hx-trigger="click | keyup changed delay:300ms | revealed | every 5s"` — what fires the request.
- `hx-push-url="true"` — update the browser URL on success.

### Server Side

The server returns an HTML fragment, not JSON. Soli's `respond_to` block picks up the `HX-Request` header automatically:

```soli
# app/controllers/posts_controller.sl
class PostsController < Controller
  def create(req)
    let post = Post.create(req["all"])
    respond_to(req, fn(format) {
      format.html(fn() redirect("/posts/#{post["id"]}"))
      format.htmx(fn() render("posts/_show_partial", { "post": post }, { "layout": false }))
    })
  end
end
```

The full request lands in `format.html` (full-page redirect); an HTMx-driven submit lands in `format.htmx` and returns only the partial. See [Controllers — Content Negotiation](/docs/core-concepts/controllers#fn-respond_to) for the full dispatch order.

### Helper Functions (optional)

Long `hx-*` attributes can be wrapped in helpers if you prefer. These are not built in — drop them in `stdlib/` if useful:

```soli
# stdlib/htmx.sl
def hx_get(url)
  "hx-get=\"#{url}\""
end

def hx_target(selector)
  "hx-target=\"#{selector}\""
end
```

```erb
<button <%= hx_get("/users") %> <%= hx_target("#users") %>>Load</button>
```

## Alpine.js — Local State and Behavior

Alpine layers reactive directives directly onto HTML for state that lives only in the browser:

```html
<div x-data="{ open: false }">
  <button @click="open = !open" class="px-3 py-1 bg-indigo-500 text-white rounded">
    Menu
  </button>
  <ul x-show="open" x-cloak class="mt-2 border rounded p-2">
    <li>Profile</li>
    <li>Sign out</li>
  </ul>
</div>
```

Add `[x-cloak] { display: none !important; }` to your CSS and `x-cloak` to anything that starts hidden — Alpine strips the attribute after parsing, eliminating flash-of-unstyled-content.

### Common Directives

- `x-data="{ … }"` — defines a reactive scope (its data).
- `x-show="expr"` — toggles `display: none` on the element.
- `x-if="expr"` — fully mounts/unmounts (use on `<template>`).
- `x-model="varName"` — two-way binding for `<input>` / `<select>` / `<textarea>`.
- `@click="…"`, `@keydown.enter="…"`, `@submit.prevent="…"` — event handlers.
- `:class="…"`, `:disabled="…"` — bind attributes to expressions.
- `x-init="…"` — run JS once on mount (useful for plugging in SortableJS, Chart.js, etc.).
- `x-ref="name"` then `this.$refs.name` — get a DOM handle without `querySelector`.

### Tabs and Modals

Tabs:

```html
<div x-data="{ tab: 'first' }">
  <nav class="flex gap-4 border-b">
    <button @click="tab = 'first'"  :class="tab === 'first'  && 'font-bold'">First</button>
    <button @click="tab = 'second'" :class="tab === 'second' && 'font-bold'">Second</button>
  </nav>
  <section x-show="tab === 'first'"  x-cloak>First panel.</section>
  <section x-show="tab === 'second'" x-cloak>Second panel.</section>
</div>
```

Modal (native `<dialog>`):

```html
<div x-data="{ open: false }">
  <button @click="open = true">Open</button>
  <dialog :open="open" @close="open = false" class="rounded-lg p-4">
    <p>Body text…</p>
    <button @click="open = false">Close</button>
  </dialog>
</div>
```

### Client-Side Validation

Pair `x-model` with derived state to give immediate feedback. Server-side `V.validate(...)` (see [Validation](/docs/builtins/validation)) remains the source of truth — Alpine just fails fast in the UI.

```html
<form x-data="{ email: '', get valid() { return /.+@.+\..+/.test(this.email) } }"
      action="/signup" method="post">
  <input type="email" name="email" x-model="email" class="border rounded px-2 py-1" />
  <p x-show="email && !valid" x-cloak class="text-red-600 text-sm">
    Enter a valid email address.
  </p>
  <button :disabled="!valid"
          class="px-3 py-1 bg-indigo-500 text-white rounded disabled:opacity-50">
    Sign up
  </button>
</form>
```

## Combined Patterns

### Optimistic UI

Wrap an HTMx-driven button in `x-data` and flip local state on click. HTMx then swaps the button with the server's authoritative render:

```html
<button x-data="{ liked: false }"
        @click="liked = true"
        :class="liked && 'text-pink-500'"
        hx-post="/posts/42/like"
        hx-swap="outerHTML">
  ♥ Like
</button>
```

If the request fails, the server's response replaces the optimistic state. For finer error handling, listen for HTMx events:

```html
<div x-data="{ status: 'idle' }"
     @htmx:before-request.window="status = 'loading'"
     @htmx:after-request.window="status = 'idle'"
     @htmx:response-error.window="status = 'failed'">
  <span x-show="status === 'loading'" x-cloak>Saving…</span>
  <span x-show="status === 'failed'"  x-cloak class="text-red-600">Save failed.</span>
</div>
```

### Swap, Then Re-Init

When HTMx swaps content into the page, Alpine's built-in `MutationObserver` re-binds any `x-data` blocks inside the new fragment automatically. You only need to wire something manually when the swapped fragment hosts a third-party widget (e.g. `Sortable`, `Chart`, a datepicker):

```html
<div hx-get="/items" hx-trigger="load" hx-swap="innerHTML"
     @htmx:after-swap="$dispatch('reinit-widgets')">
  …
</div>
```

### Mounting External Libraries

Soli does not bundle SortableJS, Chart.js, or other heavy libraries — drop them in `public/js/` (or a CDN) and let Alpine drive their lifecycle:

```html
<!-- SortableJS — drag to reorder, POST the result via fetch -->
<ul x-data
    x-init="new Sortable($el, { animation: 150,
      onEnd(e) { fetch('/items/reorder', { method: 'POST',
        body: JSON.stringify({ from: e.oldIndex, to: e.newIndex }),
        headers: { 'Content-Type': 'application/json' } }) } })"
    class="space-y-2">
  <% for item in items %>
    <li class="bg-white p-2 rounded shadow cursor-move"><%= item["name"] %></li>
  <% end %>
</ul>
```

```html
<!-- Chart.js inside an x-data component -->
<canvas x-data="{
          init() {
            new Chart(this.$el, { type: 'line', data: <%= chart_data %> })
          }
        }"></canvas>
```

The pattern is the same for any widget library: `x-init` for one-shot mounts, `x-data` with an `init()` method when the widget needs Alpine-driven state.

## Coexistence Notes

- **HTMx + Alpine** share the same DOM and work together cleanly. HTMx uses `hx-*`, Alpine uses `x-*` — no namespace collision.
- **Live View** containers re-render the DOM and reset any Alpine state held inside on each push. Keep local widget state outside Live View regions; for state that must survive a re-render, lift it into Live View.
- **The dev bar** automatically skips itself on HTMx partial responses (it reads the `HX-Request` header) so swaps don't accumulate stacked dev bars on the page. No configuration needed.

## Upgrading or Replacing

Pinned versions live at the top of `public/js/htmx.min.js` and `public/js/alpine.min.js`. To upgrade:

1. Download a newer build (`cdn.jsdelivr.net/npm/htmx.org@<version>/dist/htmx.min.js`, `cdn.jsdelivr.net/npm/alpinejs@<version>/dist/cdn.min.js`).
2. Replace the file. Update the banner comment so future readers know what version is in.
3. Run the smoke test in [Verification](#verification) below.

To opt out of either library entirely, delete the file and remove the matching `<script>` tag from `application.html.slv`. Nothing in the framework requires either of them.

## Verification

After regenerating a project (`soli new myapp`):

1. `app/views/layouts/application.html.slv` contains both `<script defer src="...htmx.min.js">` and `<script defer src="...alpine.min.js">`.
2. `public/js/htmx.min.js` and `public/js/alpine.min.js` exist and start with the version banner.
3. Drop the dropdown snippet from above into any view, reload, click — the panel toggles without a server round-trip.
4. Drop an `hx-get` button + a target div, reload, click — the response HTML appears in the target.
5. Combine them with the optimistic-Like button example — the heart turns pink instantly and the button swaps once the server responds.
