# Live View

Live View renders components on the server and pushes updates over a WebSocket. Build interactive UIs without writing JavaScript: state lives on the server, events flow over the wire, and the client **morphs the DOM in place** to match the new render — nodes are updated, not replaced, so focus, caret position, and client-side widget state survive updates (see [How patches reach the DOM](#how-patches-reach-the-dom)).

**Try it.** [A Live Field Desk](/docs/blog/liveview-desk) is a tutorial with the widget on the page — nested `live_component` assigns, uploads, in-socket tabs, debounce, click-away, hooks, and JS commands.

## How It Works

1. The browser opens a WebSocket to `/live/socket/<component>`.
2. The server renders the initial HTML and sends it down.
3. User interactions (click, submit, change, …) post events back over the socket.
4. The server invokes your handler, computes new state, re-renders, and ships a patch for the changed region.

## Creating a Live View Component

### Step 1 — Template

Create a template in `app/views/live/` (`.html.slv`; `.slv`, `.sliv`, and `.html.erb` also resolve). State is interpolated with standard ERB tags.

```html
<!-- app/views/live/counter.html.slv -->
<div class="counter-component">
  <h2>Count: <%= count %></h2>

  <button soli-click="decrement">-</button>
  <button soli-click="increment">+</button>
</div>
```

### Step 2 — Route

Register the component with `router_live(component_name, controller#action)`:

```soli
# config/routes.sl
router_live("counter", "live#counter");
```

The first argument is the **component name** (the segment in `/live/socket/<component>` and the template filename), not a URL path.

### Step 3 — Controller

A Live View handler takes one argument — an event hash with `event`, `params`, and `state` — and returns the new state.

```soli
# app/controllers/live_controller.sl
def counter(event_data: Any) -> Any {
  event = event_data["event"]   # e.g. "increment", "connect", "tick"
  state = event_data["state"]   # current component state
  count = state["count"] || 0

  if event == "increment"
    { "count": count + 1 }
  elsif event == "decrement"
    { "count": count - 1 }
  else
    state                       # unchanged for unknown events
  end
}
```

## Available Directives

| Directive | Triggers on |
|-----------|------------|
| `soli-click` | Element click |
| `soli-submit` | Form submission (named fields are sent as `params`) |
| `soli-change` | Input value change |
| `soli-keydown` | Key press |
| `soli-keyup` | Key release |
| `soli-window-keydown` | Window-level key press (put on any element in the view) |
| `soli-window-keyup` | Window-level key release |
| `soli-focus` | Element gains focus |
| `soli-blur` | Element loses focus |
| `soli-value-*` | Binds input value into state |
| `soli-target` | Specifies target component for updates |
| `soli-href` | Full-page navigation to the given URL (leaves the socket) |
| `soli-patch` | In-socket navigation: `history.pushState` + `event == "patch"` |
| `soli-live` | Swap the page-root LiveView to `/live/socket/<name>` without a full load |
| `soli-debounce` | Delay the event by N milliseconds; only the last fire is sent |
| `soli-throttle` | Send at most once every N milliseconds |
| `soli-click-away` | Click outside this element |
| `soli-disable-with` | Swap the label and disable the control while the event is in flight |
| `soli-hook` | Attach a named client hook (`mounted` / `updated` / `destroyed`) |
| `soli-upload` | File input: POST `/live/upload`, then `event` with `file` / `files` |
| `soli-upload-max` | Per-file size cap in bytes (default 8 MiB) |

Two more attributes control how the DOM morph treats an element (they trigger nothing on the server):

| Attribute | Effect |
|-----------|--------|
| `soli-key` | Identity for list items: a reordered element with the same key keeps its DOM node (and its focus/widget state) instead of being rebuilt. Falls back to `id` when absent. |
| `soli-ignore` | Marks a subtree as client-owned: the element's own attributes stay server-driven, but its children are never touched by a patch. Put Alpine islands, charts, and other third-party widgets here. |

## Template Variables

State keys are available in the template as plain ERB variables:

```html
<!-- Simple variable -->
<span>Hello, <%= username %></span>

<!-- Conditional rendering -->
<% if logged_in %>
  <a href="/logout">Sign Out</a>
<% else %>
  <a href="/login">Sign In</a>
<% end %>

<!-- Iteration -->
<% for item in items %>
  <li><%= item["name"] %></li>
<% end %>
```

## Client Setup

The client is served by the soli binary itself at `/live/client.js` — no file to vendor, and it is always in sync with the server's patch protocol. Include it only on pages that mount a live component — it is ~7 KB gzipped (~30 KB raw) and auto-connects every `[data-liveview-url]` element on `DOMContentLoaded`:

```html
<!-- Include the Live View client (built into the binary, ~7 KB gzipped) -->
<script src="/live/client.js"></script>

<!-- Mount a Live View component (auto-connects on page load) -->
<div data-live-root data-liveview-url="/live/socket/counter"></div>
```

To control connection timing yourself (e.g. after a client-side navigation that doesn't re-fire `DOMContentLoaded`), add `data-liveview-manual` to skip auto-connect and call `live()` by hand:

```html
<div data-live-root data-liveview-manual data-liveview-url="/live/socket/counter"></div>

<script>
  window.live("wss://example.com/live/socket/counter", { rootElement: document.querySelector("[data-live-root]") });
</script>
```

## How Patches Reach the DOM

When state changes, the server re-renders the template and diffs the new HTML against the previous render, shipping a compact positional patch (just the changed lines) over the socket. The client keeps a shadow copy of the exact HTML it last received, applies the patch to it, then **morphs** the live region's real DOM to match:

- Nodes are mutated in place — attributes synced, text updated — instead of being torn down and rebuilt, so `document.activeElement`, caret/selection position, and scroll state survive.
- Form fields follow a "user wins" rule: a focused field is never clobbered (typing that round-trips through `soli-change` can't lose in-flight keystrokes), and an unfocused field only changes when the server *actually changes* the rendered `value` attribute (checkboxes and selects behave the same way for `checked`/`selected`).
- List items with `soli-key` (or an `id`) keep their DOM node across reorders.
- Subtrees under `soli-ignore` are never touched — the home for Alpine widgets, charts, maps.
- The server owns everything else: DOM your own JS inserts *outside* a `soli-ignore` subtree is removed on the next patch, and `<script>` tags patched into a live region never execute.

If the client ever fails to apply a patch (lost shadow, version skew), it asks the server to replay the last full render — recovery is automatic and keeps server-side state intact.

## Lifecycle Events

Two synthetic events are dispatched by the server in addition to user-driven directives:

- `connect` — fired once, immediately after the WebSocket is established and before any client events. Use it to seed initial state and (optionally) start a tick timer.
- `tick` — fired on a recurring interval requested by the handler (see below). Use it for server-pushed updates like dashboards or live charts.

## When a handler fails

A handler that raises does **not** silently fall back to some other behaviour: the
server logs the failure and pushes an error to the client, leaving the view's state
untouched. The client sees the message itself only when the server runs with
`--dev`; in production it gets a generic `LiveView handler error` so an exception's
text (which may carry paths or query fragments) stays server-side.

Returning nothing at all is legal and means *no state change* — the view re-renders
from the current state, which normally produces an empty patch:

```soli
def toggled(event) {
  Audit.create({ "action": event["event"] })
  # no return — state is unchanged, nothing is patched
}
```

## High-Rate Updates with Ticks

For real-time dashboards, monitoring, and live data feeds, a handler can opt into a per-instance recurring tick. Return the **wrapped form** `{ "state": {...}, "tick_interval": <ms> }` from any handler invocation:

> **Live demo.** A tick-driven server clock runs live on the [LiveView docs page](/docs/core-concepts/liveview) — it's this site's own `live#metrics` handler pushing ~20 diffs a second.

```soli
# app/controllers/live_controller.sl
def metrics_dashboard(event_data: Any) -> Any {
  event = event_data["event"]

  if event == "connect"
    # Start ticking at 50ms (20 updates/sec)
    {
      "state": { "cpu": 0, "memory": 0, "requests": 0 },
      "tick_interval": 50
    }
  elsif event == "tick"
    # Server pushes fresh data on each tick
    {
      "state": {
        "time": datetime_now(),
        "cpu": system_cpu_usage(),
        "memory": system_memory_mb(),
        "requests": request_counter
      }
    }
  else
    # Unknown event — leave state and tick interval unchanged
    event_data["state"]
  end
}
```

### `tick_interval` semantics

| Returned value | Effect |
|----------------|--------|
| key absent | Leave the running tick alone |
| `0` | Stop the tick |
| `> 0` | Start (or replace) the tick at this interval, in milliseconds |

The handler may return either shape on any invocation:

- **Bare:** `{ ...state }` — the whole hash is the new state. Equivalent to `tick_interval` absent.
- **Wrapped:** `{ "state": {...}, "tick_interval": N }` — `state` is the new state; `tick_interval` controls the timer.

If you return the bare form on a tick, the timer keeps running at its previous interval. To stop the timer, return `{ "state": {...}, "tick_interval": 0 }`.

### Recommended intervals

| Interval | Use case |
|----------|----------|
| `1000ms` | Dashboards, status pages |
| `100ms` | Live charts, activity feeds |
| `50ms` (20/s) | Real-time monitoring |
| `16ms` (60/s) | Animations — use sparingly |

If a tick fires while the previous handler call is still running, the tick is dropped (rather than queued) so a slow handler doesn't snowball. Ticks stop automatically when the WebSocket closes.

## Reactive live queries

A tick polls on a timer; a **live query** pushes only when the data actually changes. Call `Model.live_where(filter)` inside a LiveView handler instead of `where(filter).all()`: it runs the same query **and** subscribes this LiveView to the collection. When any request later writes to that collection, the framework re-runs the handler and pushes a patch — no polling, no manual pub/sub.

```soli
# app/controllers/live_controller.sl
def posts_board(event_data: Any) -> Any {
  # Re-queries on connect and on every live_query_changed wake. Because the
  # render is diffed server-side, an unrelated write produces an empty diff
  # and no frame reaches the client.
  { "posts": Post.live_where({ "published": true }) }
}
```

Nothing else is required: writing a `Post` from an ordinary controller (`Post.create(...)`, `post.save()`, `post.destroy()`) wakes every board viewing it. `live_where` returns the same instances as `where(...).all()`, so a template that iterated the old result needs no changes.

Outside a LiveView render `live_where` is exactly `where(...).all()` — the subscription is a no-op — so it's a safe drop-in.

**Semantics:**

- **Per-row matching.** A flat-equality hash filter (`live_where({"published": true})`) is remembered as its field→value map, and a write only wakes subscribers the changed row actually satisfies — so publishing a *draft* post doesn't re-render a board filtered to `published: true`. Numbers compare by value (a stored `5.0` matches a bound `5`); `null` matches a missing field. Filters we can't decompose — the **string form** (`live_where("doc.views > @n", {n: 100})`), **deletes** (no row body), and **transaction commits** (only collection names survive) — wake conservatively (every subscriber), and the diff gate still drops any frame with no visible change.
- **Single-process.** Subscriptions live in server memory, like LiveView instances themselves. A write in one process doesn't wake subscribers in another — multi-process deployments need an external bus. (See [Current limitations](#current-limitations).)
- **Transaction-aware.** Writes inside a `transaction { }` block wake subscribers on **commit**, not per statement, so viewers never see uncommitted rows; a rolled-back transaction wakes no one.

## Streams

A full re-render diffs the whole component; for an **append-only or large list** (a chat log, an activity feed, a leaderboard) that's wasteful — every new row re-diffs every old one. A **stream** instead sends targeted DOM ops (`append` / `prepend` / `insert` / `remove` / `reset`) that the client applies directly to a container, without re-rendering the list.

Render the container **empty** (its items live outside the diff, so patches never fight the streamed nodes), give it a stable `id`, and return a `stream` sub-hash from the handler:

```erb
<!-- app/views/live/feed.html.slv -->
<div data-liveview-id="<%= id %>">
  <ul id="messages"></ul>   <!-- streamed into; stays empty in the render -->
</div>
```

```soli
# app/controllers/live_controller.sl
def feed(event_data) {
  if event_data["event"] == "new_message" {
    msg = event_data["params"]
    # Append one row — no re-render of the existing list.
    return {
      "stream": {
        "container": "messages",
        "ops": [
          { "op": "append", "id": "msg-#{msg["id"]}", "html": "<li id=\"msg-#{msg["id"]}\">#{h(msg["text"])}</li>" }
        ]
      }
    }
  }
  { "count": 0 }   # connect: initial state
}
```

The returned hash may carry `state` **and** `stream` together (update some state *and* stream rows), or `stream` alone (state untouched). Op reference:

| Op | Fields | Effect |
|----|--------|--------|
| `append` | `id`, `html` | Add as the container's last child (re-adding an existing `id` moves it) |
| `prepend` | `id`, `html` | Add as the first child |
| `insert` | `id`, `html`, `before?` | Insert before the child `before` (append if omitted/missing) |
| `remove` | `id` | Remove the element with that id |
| `reset` | — | Clear the container |

`container` is hoisted on the `stream` hash and applies to every op (an op may override it with its own `container`). Rows should carry a stable `id` so re-adds de-dupe and `remove` can find them. Because streamed nodes are outside the diff shadow, a reconnect re-mounts from the initial (empty) render — re-drive the stream on `connect` if the list must survive a reconnect.

## Debounce, throttle, and navigation

Search boxes and sliders should not fire an event on every keystroke. Put `soli-debounce` or `soli-throttle` (milliseconds) on the same element as the directive — `data-soli-debounce` works too:

```html
<input type="search" soli-change="search" soli-debounce="300" name="q">
<input type="range" soli-change="volume" soli-throttle="100" name="vol">
```

Debounce waits until the user pauses; throttle sends at most once per window. They apply to every event type (click, change, keydown, …).

For a full page leave, use `soli-href` on a link or button, or return a `redirect` from the handler:

```html
<a soli-href="/posts">Back to posts</a>
```

```soli
if event == "saved" {
  { "redirect": "/posts/#{post.id}" }
}
```

`redirect` persists any `state` the handler also returned, then the client navigates away. A click on `soli-href` does the same navigation without a round-trip (meta/ctrl-click still opens a new tab).

### In-socket navigation (`soli-patch`)

To change the URL **without** dropping the socket, use `soli-patch` (Phoenix `live_patch`):

```html
<a soli-patch="/posts/<%= id %>?tab=comments">Comments</a>
```

The client pushes the URL and sends `event == "patch"` with:

| Param | Value |
|-------|--------|
| `href` | `/posts/1?tab=comments` |
| `path` | `/posts/1` |
| `query` | `{ "tab": "comments" }` |
| `hash` | `""` |

```soli
if event == "patch" {
  { "state": { "tab": event_data["params"]["query"]["tab"] } }
}
```

The handler can also push a URL itself:

```soli
{ "state": { "tab": "comments" }, "patch": "/posts/1?tab=comments" }
# or replace the current history entry:
{ "patch": { "url": "/posts/1", "replace": true } }
```

Browser back/forward fires the same `patch` event. The JS `patch` command only updates the address bar (it no longer synthesizes `popstate`, which would double-fire).

To swap the **page-root** LiveView to a different component without a full load:

```html
<a soli-live="/live/socket/comments" href="/comments">Comments</a>
```

The client closes this socket, connects `/live/socket/comments` on the same root, and (if `href` or `soli-patch` is set) updates the address bar. A handler can do the same with `{ "live": "/live/socket/comments", "patch": "/comments" }`.

`soli-href` / handler `redirect` / JS `navigate` still do a full page load — use those when the destination is a regular (non-LiveView) page.

## JS commands

A handler can push a small, **eval-free** list of client commands alongside (or instead of) a re-render. Unknown ops are skipped.

```soli
if event == "flash" {
  {
    "state": { "ok": true },
    "js": [
      { "op": "add_class", "to": "#flash", "class": "show" },
      { "op": "focus", "to": "#q" }
    ]
  }
}
```

| `op` | Fields | Effect |
|------|--------|--------|
| `add_class` / `remove_class` / `toggle_class` | `to`, `class` | Space-separated class names |
| `set_attr` / `remove_attr` | `to`, `name`, `value?` | Attribute write / delete |
| `focus` | `to` | Focus the element |
| `dispatch` | `to`, `event`, `detail?` | `CustomEvent` on the target |
| `navigate` | `url` | `window.location` (full load) |
| `patch` | `url` | `history.pushState` + `popstate` |

`to` is a CSS selector, or `window` / `document` / `body`. There is no `eval` path.

The wrapped handler return may carry any combination of `state`, `tick_interval`, `stream`, `redirect`, and `js`. `redirect` is handled first and skips the subsequent patch.

## Loading states

While an event is in flight the triggering element gets `soli-loading` and `soli-<event>-loading` (e.g. `soli-click-loading`). `soli-disable-with` also swaps the label and sets `disabled` until the next patch (or an empty diff) restores the server-rendered markup:

```html
<button soli-click="save" soli-disable-with="Saving…">Save</button>
```

```css
.soli-click-loading { opacity: 0.6; }
```

## Click away

`soli-click-away` fires when a click lands outside the element — the usual pattern for closing a dropdown or popover:

```html
<div class="menu" soli-click-away="close">
  …
</div>
```

## Client hooks

For a chart, map, or other widget that needs a JS constructor, put `soli-hook="Name"` on the element and register the hook before the socket connects. Auto-connect runs on `DOMContentLoaded`, so a script after `/live/client.js` can set `SoliLiveView.hooks` in time:

```html
<script src="/live/client.js"></script>
<script>
SoliLiveView.hooks = {
  Chart: {
    mounted() {
      this.chart = Chart.render(this.el, JSON.parse(this.el.dataset.series));
    },
    updated() {
      this.chart.refresh(JSON.parse(this.el.dataset.series));
    },
    destroyed() { this.chart.teardown(); },
    disconnected() {},
    reconnected() {}
  }
};
</script>

<div soli-hook="Chart" soli-ignore data-series='<%= json_stringify(series) %>'></div>
```

Pair hooks with `soli-ignore` when the widget owns its children. `this.el` is the bound element; `this.pushEvent("name", { … })` sends an event back over the socket (no debounce). `live(url, { hooks: { … } })` merges on top of `SoliLiveView.hooks`.

Callbacks: `mounted` (first seen), `updated` (same node survived a morph), `destroyed` (removed), `disconnected` / `reconnected` (socket).

## Nested LiveViews

A LiveView template may mount another component as a child socket. Put `data-liveview-url` (and usually `data-live-root`) on the slot; after each parent render the client connects any new mounts and disconnects any that disappeared.

```html
<!-- parent: app/views/live/post.html.slv -->
<article>
  <h1><%= title %></h1>
  <div data-liveview-url="/live/socket/comments" data-live-root></div>
</article>
```

```soli
# config/routes.sl
router_live("post", "live#post");
router_live("comments", "live#comments");
```

The child's DOM is treated as a `soli-ignore` island automatically (`data-liveview-url` on the slot), so a parent patch does not wipe the child's render. The two views do not share state — they are independent sockets. `data-liveview-manual` skips auto-connect, same as at the page root.

Independent child sockets still do not share state. For **shared parent assigns**, use a nested live component (same socket, parent state is the source of truth):

```erb
<%- live_component("score", { "score": true }) %>
```

`true` / `from_parent` copies that key from the parent; any other value is a literal override. The helper renders `app/views/live/score.html.slv` and wraps it in `soli-component="score"`.

```erb
<!-- app/views/live/score.html.slv -->
<span id="s"><%= score %></span>
<button soli-click="inc" soli-assign-score="<%= score + 1 %>">+</button>
```

A click inside the wrapper sends `_component: "score"` and `_assigns: { "score": N }`. The runtime merges `_assigns` onto the parent state (coercing types to match) before the handler runs, then the parent re-render fans the new values back into the child. The parent handler can also branch on `params["_component"]`.

This is not Phoenix `live_component` (no `update/2` / `send_update`). Independent `[data-liveview-url]` sockets remain valid when you *want* isolation.

## File uploads

WebSocket frames are capped at 1 MiB, so bytes go over **HTTP** and the socket only carries an id. Put `soli-upload="handler"` on a file input (the page layout should include `csrf_meta_tag()`):

```html
<input type="file" name="avatar" accept="image/*"
       soli-upload="attached" soli-upload-max="2000000">
```

On change the client `POST`s each file to `/live/upload` (multipart + `X-CSRF-Token`), then sends your handler with:

```soli
if event == "attached" {
  file = event_data["params"]["file"]
  # { "filename", "content_type", "size", "data" } — same shape as
  # find_uploaded_file / attach_upload. `data` is base64.
  attach_upload(user, "avatar", file)
  { "state": { "name": file["filename"] } }
}
```

`params["files"]` is the array (use `multiple` on the input). `params["file"]` is the first entry. While the POST is in flight the input gets `soli-upload-loading` and `data-soli-progress` (0–100). Put `[soli-upload-bar]` in the same `<label>` and it fills to that percent (`--soli-progress` is set on the label too). An `img[soli-upload-preview]` in that label shows a local preview for image files as soon as you pick them. A failure adds `soli-upload-error` and sends `{ "error": "…" }` instead of a file.

There is no chunked/resumable protocol — one POST per file, 8 MiB default cap.

An upload belongs to the session that posted it: the id is only redeemable by that
session's LiveView, and each session holds at most **8** pending uploads (they also
expire after 10 minutes). A handler that never consumes its files therefore cannot
fill the server's upload store or lock other users out.

## Current limitations

Live View is young. Server-pushed re-renders and DOM-aware patching work well; some edges remain:

- **The wire format is line-granular, not node-granular.** The server ships the changed lines of the render (the client's morph is what makes the update DOM-aware); Phoenix-style static/dynamic splitting, which ships only the changed *values*, is not implemented. Fine in practice — renders are compared server-side and only the delta travels.
- **Uploads are not chunked or resumable.** `soli-upload` POSTs each file to `/live/upload` (8 MiB default) and then hydrates `params["file"]` for the handler. There is no pause/resume, no multi-part chunk protocol, and no Phoenix `allow_upload` consume pipeline.
- **Independent child sockets still isolate state.** `[data-liveview-url]` mounts remain their own sockets. Shared assigns use `live_component` + `soli-assign-*` on the parent socket — there is no Phoenix `update/2` / `send_update`.
- **Leaving for a regular page is still a full load.** `soli-href`, handler `redirect`, and JS `navigate` drop the socket. Same-app LiveView changes use `soli-patch` (this component) or `soli-live` (another `/live/socket/<name>`).
- **Scripts don't run on patch.** `<script>` tags inside a live region never execute when patched in; put behavior in external JS, a hook, or an Alpine island under `soli-ignore`.
- **Reconnects restore server state, not the client shadow.** A dropped socket reconnects with backoff; the new connection reuses the previous instance state (same `session:component` id) so the connect handler sees in-flight values. Every open tab of that session is attached as another sender, so a click in one tab patches the others. The client still remounts the DOM from a fresh render. Nested child sockets reconnect independently. Once the last socket for an instance closes, its state is held for **two minutes** so a refresh or a network blip reclaims it, then reaped — a ticking view re-arms its timer on reconnect, and the instance's `live_where` subscriptions stop firing as soon as no socket is attached.
- **Per-process.** Instances and `live_where` subscriptions live in server memory; a write in one process does not wake views in another. Multi-instance deployments need their own pub/sub layer.

## Why Live View?

- **No JavaScript required** — build interactive UIs entirely in server-side code.
- **SEO friendly** — initial HTML is server-rendered.
- **Reduced complexity** — no client-side state management to maintain.
- **Real-time by default** — the WebSocket connection enables instant updates and server-pushed ticks.
