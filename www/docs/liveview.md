# Live View

Live View renders components on the server and pushes updates over a WebSocket. Build interactive UIs without writing JavaScript: state lives on the server, events flow over the wire, and the client patches the changed region into the page (a pragmatic line-based diff — see [Current limitations](#current-limitations)).

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
fn counter(event_data: Any) -> Any {
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
| `soli-submit` | Form submission |
| `soli-change` | Input value change |
| `soli-keydown` | Key press |
| `soli-keyup` | Key release |
| `soli-focus` | Element gains focus |
| `soli-blur` | Element loses focus |
| `soli-value-*` | Binds input value into state |
| `soli-target` | Specifies target component for updates |

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

Include the client only on pages that mount a live component — it is ~5 KB gzipped (~21 KB raw) and auto-connects every `[data-liveview-url]` element on `DOMContentLoaded`:

```html
<!-- Include the Live View client (~5 KB gzipped) -->
<script src="/js/live.js"></script>

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

## Lifecycle Events

Two synthetic events are dispatched by the server in addition to user-driven directives:

- `connect` — fired once, immediately after the WebSocket is established and before any client events. Use it to seed initial state and (optionally) start a tick timer.
- `tick` — fired on a recurring interval requested by the handler (see below). Use it for server-pushed updates like dashboards or live charts.

## High-Rate Updates with Ticks

For real-time dashboards, monitoring, and live data feeds, a handler can opt into a per-instance recurring tick. Return the **wrapped form** `{ "state": {...}, "tick_interval": <ms> }` from any handler invocation:

> **Live demo.** A tick-driven server clock runs live on the [LiveView docs page](/docs/core-concepts/liveview) — it's this site's own `live#metrics` handler pushing ~20 diffs a second.

```soli
# app/controllers/live_controller.sl
fn metrics_dashboard(event_data: Any) -> Any {
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

## Current limitations

Live View is early. Server-pushed re-renders work well; the edges are still v1-pragmatic:

- **Diffing is line-based, not DOM-aware.** The server compares the old and new render line-by-line and ships one "replace" patch for the changed region; small components simply get a full re-render. There is no tree diffing or keyed reconciliation, so client-side widget state held *inside* a live region (focus, Alpine state, …) does not survive a patch.
- **The directive set is a subset of Phoenix's.** Click, submit, change, keydown/keyup, focus/blur, `soli-value-*`, and `soli-target` — there is no debounce/throttle, no window-level bindings, no JS commands, no uploads, streams, or nested live components.
- **Reconnects re-mount.** If the socket drops, the client reconnects with backoff, but the component restarts from its initial state — in-flight state is not restored.
- **Per-process.** Instances live in server memory; multi-instance deployments need their own pub/sub layer to coordinate.

## Why Live View?

- **No JavaScript required** — build interactive UIs entirely in server-side code.
- **SEO friendly** — initial HTML is server-rendered.
- **Reduced complexity** — no client-side state management to maintain.
- **Real-time by default** — the WebSocket connection enables instant updates and server-pushed ticks.
