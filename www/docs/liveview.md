# Live View

Live View renders components on the server and pushes DOM diffs over a WebSocket. Build interactive UIs without writing JavaScript: state lives on the server, events flow over the wire, and the client applies minimal patches.

## How It Works

1. The browser opens a WebSocket to `/live/socket/<component>`.
2. The server renders the initial HTML and sends it down.
3. User interactions (click, submit, change, …) post events back over the socket.
4. The server invokes your handler, computes new state, re-renders, and ships only the diff.

## Creating a Live View Component

### Step 1 — Template

Create a `.sliv` file in `app/views/live/`. Use `@variable` to interpolate reactive state.

```html
<!-- app/views/live/counter.sliv -->
<div class="counter-component">
  <h2>Count: @count</h2>

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

The first argument is the **component name** (used as the `data-live-view` ID and the segment in `/live/socket/<component>`), not a URL path.

### Step 3 — Controller

A Live View handler takes one argument — an event hash with `event`, `params`, and `state` — and returns the new state.

```soli
# app/controllers/live_controller.sl
fn counter(event_data: Any) -> Any {
  let event = event_data["event"];   # e.g. "increment", "connect", "tick"
  let params = event_data["params"]; # client-supplied event params
  let state = event_data["state"];   # current component state
  let count = state["count"];

  if count == null {
    count = 0;
  }

  if event == "increment" {
    return { "count": count + 1 };
  }

  if event == "decrement" {
    return { "count": count - 1 };
  }

  # Return unchanged state for unknown events
  state
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
| `soli-debounce` | Debounces event by N ms |

## Template Variables

State is accessed in templates via `@variable`:

```html
<!-- Simple variable -->
<span>Hello, @username</span>

<!-- Conditional rendering -->
<% if @logged_in %>
  <a href="/logout">Sign Out</a>
<% else %>
  <a href="/login">Sign In</a>
<% end %>

<!-- Iteration -->
<% for item in @items %>
  <li><%= item["name"] %></li>
<% end %>
```

## Client Setup

```html
<!-- Include the Live View client (~2KB gzipped) -->
<script src="/js/live.js"></script>

<!-- Mount a Live View component -->
<div id="counter" data-live-view="counter"></div>

<script>
  // Initialize Live View
  SoliLive.connect();
</script>
```

## Lifecycle Events

Two synthetic events are dispatched by the server in addition to user-driven directives:

- `connect` — fired once, immediately after the WebSocket is established and before any client events. Use it to seed initial state and (optionally) start a tick timer.
- `tick` — fired on a recurring interval requested by the handler (see below). Use it for server-pushed updates like dashboards or live charts.

## High-Rate Updates with Ticks

For real-time dashboards, monitoring, and live data feeds, a handler can opt into a per-instance recurring tick. Return the **wrapped form** `{ "state": {...}, "tick_interval": <ms> }` from any handler invocation:

```soli
# app/controllers/live_controller.sl
fn metrics_dashboard(event_data: Any) -> Any {
  let event = event_data["event"];

  if event == "connect" {
    # Start ticking at 50ms (20 updates/sec)
    return {
      "state": { "cpu": 0, "memory": 0, "requests": 0 },
      "tick_interval": 50
    };
  }

  if event == "tick" {
    # Server pushes fresh data on each tick
    return {
      "state": {
        "time": datetime_now(),
        "cpu": system_cpu_usage(),
        "memory": system_memory_mb(),
        "requests": request_counter
      }
    };
  }

  # Unknown event — leave state and tick interval unchanged
  event_data["state"]
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

## Why Live View?

- **No JavaScript required** — build interactive UIs entirely in server-side code.
- **SEO friendly** — initial HTML is server-rendered.
- **Reduced complexity** — no client-side state management to maintain.
- **Real-time by default** — the WebSocket connection enables instant updates and server-pushed ticks.
