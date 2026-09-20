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

## Serving a page with no session

A socket costs this server a session per reader: the instance, the four
interned tables, and the previous tree they diff against. Measured against the
EUI site, that is **50–60 kB of resident memory for somebody who is only
reading**, linear to four hundred sessions, against 4 799 B of page — about
twelve times the page, held for as long as the window is open. For a
documentation page or a catalogue that is the wrong shape, and the socket buys
nothing there, because nothing on such a page changes unless the reader
changes it.

So a view can be answered as one render instead. The body is the frames a
fresh socket would have sent — a `Welcome` and the batches through the first
`Mount` — with a strong `ETag` over it, so a cache or a CDN answers the second
reader and this server renders once per revalidation rather than once per
person. Six hundred *distinct* renders, each at a different viewport so
nothing could be reused, moved resident memory by four kilobytes.

The usual way is an ordinary route, because a page wants its own URL and its
own params:

```soli
# app/controllers/docs_controller.sl
def show
  page = Doc.find(req["params"]["slug"])
  respond_to(req, fn(format) {
    format.html(fn() render("docs/show", {"page": page}))
    format.eui(fn()  eui_render(doc_view(page)))
  })
end
```

`eui_render(tree)` encodes a view hash into frames and returns an ordinary
response — status, headers, an `ETag`, and a `304` when the caller's
`If-None-Match` matches. The encoder is built, used and dropped inside the
call: no instance, no registry entry and nothing to clean up, because an
action is already running on a worker with an interpreter. A page on a route
is *cheaper* to serve this way than through a component, not dearer. `eui?`
answers the same question `format.eui` asks, for a controller that would
rather branch itself.

A component with no route of its own can be offered the same way, and is then
fetched from `GET /_eui/view/<component>`:

```soli
# config/routes.sl
router_eui("site", "site#site", "site#site_view", {"static": "public, max-age=60"})
```

`{"static": true}` means `no-cache`, which still saves the session and still
revalidates against the `ETag`. It cannot be combined with
`{"session": "required"}` — a static view is rendered for nobody — and saying
both is refused where it is written rather than at request time.

Three things follow from "rendered for nobody", and all three are the
application's to honour:

- The render sees **no session, no cookie and no locale**. A view that greets
  somebody by name does not belong here, and a component that needs a session
  is refused outright.
- A `GET` has **no same-origin check** — a resource a CDN is meant to hold
  cannot have one — so an `<img src>` on any page anywhere reaches it.
  Offering a view this way is promising that rendering it is a *read*.
- Two renders of the same thing must produce **the same bytes**, since the
  `ETag` is the identity of those bytes. A view that reads the clock, a
  random, or a counter that moves per render is still correct — it simply
  caches nothing.

The render is given a nominal viewport, since there is nobody to ask; `?w=`
carries a width when the client knows one. A view that derives its
measurements from the viewport will therefore draw at a size the reader did
not choose, which is a reason to prefer layout the client resolves. In a
browser, an address that serves EUI and nothing else answers with a short page
saying so rather than a 404.

## Islands: a page with one live corner

A whole page becoming a session because one part of it must be live is the
wrong trade: every reader then pays a session's memory for a comment count
that changes twice a day. A node may instead carry an `island` prop — an
absolute path on this same origin — and take its **content** from a session of
its own, while the page around it stays a cached render nothing is held for.
The name is the web's own: a page that is mostly still, with islands in it
that are not.

```soli
{"k": "slot", "island": "/_eui/session/comments?for=" + page["id"], "c": [
  {"k": "text", "t": str(page["comment_count"]) + " comments"}
]}
```

The node's own children are what shows until that session speaks, and they
came from whatever rendered the page. So a client too old to know the prop, a
session that cannot be opened, and a page served from a cache all end in the
same place — content that is out of date rather than missing.

The query is how two islands of one component tell your application which of
them is being rendered: it arrives in `connect` alongside `viewport`, so
`params["for"]` is the row. Islands naming the same path share one session, at
most eight are opened for a page, and an `island` naming another origin is
refused.

## The session that keeps the tree

A page fetched as one render opens no socket until something happens only the
server can answer. When one is dialled, Soli does not simply mount the page
again — which would discard the tree, the layout, focus and every scroll
offset, returning a reader partway down the page to the top.

Instead the client offers the hash of what it holds, Soli renders `connect` as
it would have anyway, and if the two agree it sends a `Welcome` and nothing
else. One render either way, and nothing for the application to do: it is the
same `connect` it always was.

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

The manifest also carries the app's **icon**, which is what a client
installs it as: `eui --install <url>`, or the arrow in the address bar,
writes a launcher entry — a `.desktop` file on Linux, a bundle in
`~/Applications` on macOS, a Start menu shortcut on Windows — that opens
the app in its own window. Drop a PNG at `public/icon.png` and that is the
whole of publishing one; `eui_icon("public/images/logo.png")` names a
different file. The hash rides inside the signed body, so the picture on
the launcher tile is the publisher's and not something picked up on the
way. An app with no icon cannot be installed, because the alternative is
every installed app wearing the same one.


## Opening an application

The standalone client is `eui <wss://host/_eui/session/app>`; a soli built
with `--features eui-desktop` also opens one itself with
`soli eui <url> [--allow clipboard.read]`, and packages an app to open in
its own window with `soli desktop build --eui <component>` (see
[Desktop Apps](/docs/development-tools/desktop)).
