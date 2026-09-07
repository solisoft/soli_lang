# EUI — native interfaces without HTML

EUI is Soli's protocol for delivering an application interface to a native
client without HTML, CSS or JavaScript. The server sends an interface tree
that is already resolved, as compact binary patches; a Rust client applies
them, lays out, and draws on the GPU. A counter costs a few megabytes of RAM
and no CPU at idle, because there is no document engine to keep running.

It is **optional**: a `soli` built with `cargo build --features eui` has it,
the default build does not, and nothing else changes either way.

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

## Style is roles, not CSS

There is no cascade and no selector. A style is a hash of the protocol's own
vocabulary — `display`, `gap`, `pad`, `bg`, `fg`, `size`, `weight`, `radius`,
`width`, `align`, `justify`, `cursor` — and colours are **roles**:
`"accent.base"`, `"text.muted"`, `"surface.raised"`. The client resolves roles
against the viewer's light or dark mode, density and font scale, so the same
view is right in dark mode without the server knowing. A literal `"#RRGGBB"`
is available for a brand mark and wrong for a surface.

## Events carry the node's props

`"on": {"click": "toggle"}` names a server event. When it fires, the handler's
`params` carry the node id, the event kind, its payload, and — because the
server kept the tree — the node's `props`. That is how a row in a list says
which row it is:

```soli
checkbox(item["title"], item["done"], "toggle", {"id": item["id"]})
# … in the handler:
id = params["props"]["id"]
```

An event on a node that has no such handler in the tree the client was sent
is refused and ends the session. Nothing a client sends is trusted before
that check.

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

## Images and assets

```soli
avatar("public/images/avatar.png", 32)
```

The path is a file in the application. Soli hashes it (BLAKE3), sends the
hash in the tree, and serves the bytes at `GET /_eui/asset/<hash>` with a
one-year immutable cache header — the same bytes for every session and every
client, so a CDN can hold them and nothing on the path can substitute them.
A path that resolves outside the application is refused before it is read.

## Keyed lists and virtualisation

Give repeated children a key (`keyed(id, row(...))`) and a re-sort becomes
`MoveChild` ops rather than a rebuild. A `list` with an item height is
virtualised on the client: it lays out only the rows it can see, so ten
thousand rows cost about what fifty do.

**Moving pictures.** `video(path, props, style, handlers)` puts a GIF or
an animated WebP in the tree (EUI spec 03 §8). It sizes itself to its
frames, `playing`, `loop` and `position` say what it should be doing, and
`ended` comes back. The client decodes it in its sandboxed worker and
advances it on its own clock, waking exactly when the next frame is due.

The client's **viewport** — width, height, scale, mode, density, font
scale — reaches the handler as `params["viewport"]` with the `connect`
event and again as a `viewport` event whenever it changes (a resize, a
mode switch). A view that keeps it in the state can lay itself out by
width: `examples/counter-app`'s music player collapses its sidebar to a
rail under 900 px and drops it under 640.

A `list` can be **windowed** (EUI spec 04 §7.1): give it `count`, the
number of rows, `heights`, one integer per row (or rely on `item_height`),
and a `window` handler; hand it only the children in view, each carrying
a `row` prop. The client lays out and scrolls the whole extent, asks
`window` with `[first, last]` when the rows in view change, and your
handler stores that range in the state so the next view builds those rows
and no other. A feed of forty thousand posts then costs the server one
window of cards. `examples/counter-app`'s `feed` is written this way
(`list_window` in its builders).

A keyed child is also what the server memoises. When the view returns, for
a keyed node, **the same hash object it returned last time**, Soli keeps the
converted subtree and skips it in the diff — it is neither walked nor
compared. So build repeated children once, keep them in a cache keyed by
whatever they depend on, and return the cached value; a change to one card
in a feed of ten thousand then costs one card, not ten thousand. The
contract is the usual one for a cache: a value handed back unchanged is
assumed unchanged, so never mutate a node hash after returning it — build
a new one instead. Unkeyed nodes and freshly built ones are converted and
diffed as before.

## Running it

```sh
cargo build --features eui
./target/debug/soli serve path/to/app --port 5011
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
