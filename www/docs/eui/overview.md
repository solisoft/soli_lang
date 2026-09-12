# EUI — native interfaces without HTML

EUI is Soli's protocol for delivering an application interface to a native
client without HTML, CSS or JavaScript. The server sends an interface tree
that is already resolved, as compact binary patches; a Rust client applies
them, lays out, and draws on the GPU. A counter costs a few megabytes of RAM
and no CPU at idle, because there is no document engine to keep running.

It is **on by default**: `eui` is part of the default feature set, so a
stock `cargo build --release` / `cargo install --path . --locked` has it.
Drop it with `--no-default-features` when you want a slimmer binary.

The reference is split across a few pages:

| Page | What it covers |
|------|----------------|
| [Styling](/docs/eui/styling) | The style vocabulary and the colour roles |
| [Events](/docs/eui/events) | Every event kind, and waking a session |
| [Assets](/docs/eui/assets) | Images, keyed lists and virtualisation |
| [Widgets — layout](/docs/eui/widgets-layout) | Containers, overlays, navigation, theme |
| [Widgets — input](/docs/eui/widgets-input) | Actions, fields, dates |
| [Widgets — content](/docs/eui/widgets-data) | Typography, media, data, charts |
| [Widgets — internals](/docs/eui/widgets-internals) | The catalogue's own helpers |

## A component is a LiveView component

An EUI component uses the LiveView machinery you already know — the same
route registration, the same `{event, params, state} -> state` handler, the
same registry and worker pool. Two things differ: the view is a function of
state returning a **node tree as plain data** instead of an HTML template, and
the socket is `/_eui/session/<component>` carrying binary frames.

```soli
# config/routes.sl
router_eui("counter", "live#counter", "live#counter_view")
```

```soli
# app/controllers/live_controller.sl

def counter(event_data)
  event = event_data["event"]
  count = event_data["state"]["count"] ?? 0
  if event == "increment"
    {"count": count + 1}
  elsif event == "decrement"
    {"count": count - 1}
  else
    {"count": count}
  end
end

def counter_view(state)
  count = state["count"] ?? 0
  column({"pad": 6, "gap": 4, "align": "start", "bg": "surface.base"}, [
    text("Counter", {"size": 4, "weight": "semibold"}),
    text(count.to_s, {"size": 7, "weight": "bold"}),
    row({"gap": 2}, [button("−", "decrement"), button("+", "increment")]),
    text("Every click is a round trip.", {"fg": "text.muted", "size": 1})
  ])
end
```

`column`, `row`, `text`, `button` and the rest of the catalogue — through
`select`, `slider`, the date pickers and the charts — are ordinary Soli
functions returning hashes — nothing native. Each node is:

```
{"k": kind, "s": style, "t": text, "c": children, "on": handlers, "key": key, "p": props}
```

The server turns that into nodes, interns every atom and every distinct style
once per session, diffs against the tree it last sent, and encodes the patch.

## Who may connect

No middleware runs for a WebSocket upgrade — an `auth` middleware guarding
`/admin/*` says nothing about `/_eui/session/admin_panel`. The socket is
gated in two places, and both are yours to write:

```soli
# config/routes.sl — no session cookie, no socket (401 before any handler runs)
router_eui("admin_panel", "admin#panel", "admin#panel_view", {"session": "required"})
```

```soli
# The handler: `connect` sees the session and may refuse the client.
def panel(event_data)
  user = current_user()
  return {"close": "sign in first"} if user.nil?
  return {"close": "not an admin"} unless user["role"] == "admin"
  ...
end
```

`{"close": reason}` — from `connect` or any later event — sends the client an
`Error` frame (code 403) with the reason and ends the session; nothing is
rendered for it. A `connect` that closed never ran for that client, so no
`disconnect` follows. Without `{"session": "required"}` a cookie-less client
gets a synthetic session and `connect` runs as nobody: fine for a public
board, wrong for anything else.

An EUI component is reachable only over its own socket. `/live/socket/<component>`
answers 404 for it, so the JSON LiveView socket cannot be used to call its
handler with an event name and `params` of the client's choosing.

## Limits, budgets and back-pressure

What the client would refuse, the server refuses first, with a reason the
view's author can act on — a tree nested past 256, more than a million nodes,
a text over 4 KiB, or a session that has interned more atoms, styles, colours
or chunks than the protocol allows (a key or a style derived from data grows
the table with every new value). Such a render fails with `Error` code 400 and
the session ends, since every later render would fail the same way.

Sockets are admitted against `SOLI_WS_MAX_CONNECTIONS` and
`SOLI_WS_MAX_CONNECTIONS_PER_IP`, like every other socket; a client has ten
seconds to say Hello; the server pings every thirty seconds and closes after
two go unanswered. Inbound frames are charged against the `/ws/*` budget
(`SOLI_WS_MAX_MESSAGES_PER_SEC`), a `Resync` twenty at a time — it re-sends
the whole tree. A reader that stops draining its frames is closed after two
seconds rather than have batches skipped: the client reconnects and resyncs,
and a session is never left with a tree the server no longer has.

An `Error` frame ends the session on both sides, so it is sent only when the
session really is over: 503 when the worker queue could not take the event,
504 when the handler did not answer in thirty seconds, 500 when the worker
went away. A handler that raised is none of those — the state is unchanged,
the screen still right — and stays a log line.

Each EUI session is pinned to one realtime worker (see `SOLI_WS_WORKERS`), so
its renders run on the thread that holds its kept subtrees and the
application's own per-worker objects.

## Local-first handlers

A handler can run on the client before the round trip. The counter's `+`:

```soli
local_button("+", "state.count += 1; value.text = str(state.count)", "increment")
```

That string is a small statement language — assignments, arithmetic, `if … else`, `self.style = @hover`, `theme.toggle()` (the viewer's palette, light ⇄ dark; or `theme.mode = "dark"`), `emit` — compiled by Soli to a bytecode chunk, delivered once per
session, verified by the client before it first runs, and executed with a
fuel budget. It reads the root node's props as local state (`with_state`),
rewrites the node keyed `"value"`, then sends `increment`; the next batch
from the server confirms or corrects. Nothing a local handler does is
trusted, and authorisation is never local.

## Running it

```sh
cargo build --release
./target/release/soli serve path/to/app --port 5011
# the client, from the eui repository:
EUI_ALLOW_INSECURE_LOOPBACK=1 eui ws://127.0.0.1:5011/_eui/session/counter
```

A deployment sits behind TLS; the client refuses anything but `wss://` outside
a debug build on loopback.

## Where the rest is

The protocol specification, the client crates, the widget catalogue and the
measured budgets are in the `eui` repository and its documentation site. This
page covers only what changed in Soli.

## The manifest

With the feature on, every app also answers `GET /.well-known/eui` with a
signed manifest. The signing key is generated the first time it is needed,
into `config/eui_publisher.pkcs8`; a client pins the public half on its
first visit and refuses a different key afterwards, so keep the file with
the app's secrets and out of version control. `eui_capabilities("clipboard.read")`
in `config/routes.sl` lists what the manifest asks the client for; nothing
is granted by asking — the person allows each capability on their side.


## Opening an application

The standalone client is `eui <wss://host/_eui/session/app>`; a soli built
with `--features eui-desktop` also opens one itself with
`soli eui <url> [--allow clipboard.read]`, and packages an app to open in
its own window with `soli desktop build --eui <component>` (see
[Desktop Apps](/docs/development-tools/desktop)).
