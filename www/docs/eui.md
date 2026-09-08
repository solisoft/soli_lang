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

## The widget catalogue

There is no widget primitive. The protocol can express **sixteen node kinds** and nothing
else — a button is a `box` with a click handler and a text child. Everything below is
composed from those kinds by ordinary Soli functions returning hashes, which is why adding
a widget is a library change and never a new client.

> **Where these live.** The catalogue is not yet shipped as a Soli library: it is a single
> file, `app/controllers/eui_builders.sl`, in the `counter-app` example of the
> [eui repository](https://github.com/solisoft/eui). Copy it into your application and edit
> it — that is the intended use, and it is why a widget you disagree with is yours to change.

### Layout and containers

Every container takes a style hash and a list of children. `column`, `row` and `stack` are the same `box` primitive with a different `display`.

#### `column(style, children)`

A `box` that stacks children vertically. The workhorse.

```soli
column({"gap": 3, "pad": 4}, [
  text("First", {}),
  text("Second", {})
])
```

#### `row(style, children)`

Horizontal flow. Pair with `spacer()` to push the tail to the right.

```soli
row({"gap": 2, "align": "center"}, [
  text("Label", {}),
  spacer(),
  button("Go", "go")
])
```

#### `stack(style, children)`

Children painted on top of one another, in order. The base for badges on avatars and overlay content.

```soli
stack({}, [
  avatar(src, 40),
  badge("3", "danger")
])
```

#### `card(style, children)`

A raised surface: padding, radius and a border from the theme roles.

```soli
card({}, [
  h2("Billing"),
  muted("Next invoice on the 1st")
])
```

#### `scroll(style, children)`

A clipping viewport. Children lay out fully; only the visible part is painted.

```soli
scroll({"height": 200}, rows)
```

#### `list(style, item_height, children)`

A `scroll` that `virtualises`: with a fixed `item_height` only the visible window is laid out. Use it past a few hundred rows.

```soli
list({"height": 400}, 32, rows)
```

#### `list_window(style, item_height, count, heights, children, on_window)`

Virtualisation where the server holds the data: the client reports the visible range through `on_window` and you send only those rows. `heights` allows variable row heights.

```soli
list_window({"height": 400}, 32, total, heights,
            visible_rows, "window")
```

#### `tile(basis, node)`

Wraps a node with a flex basis, for grid-like rows that wrap.

```soli
row({"gap": 3, "wrap": true}, cards.map(fn(c) tile(240, c)))
```

#### `labelled(title, child)`

A caption above any node. The building block behind `field`.

```soli
labelled("Region", select(regions, value, open, "t", "pick"))
```

#### `spacer()`

Flexible empty space. Inert: no text, props or handlers.

```soli
row({}, [text("Left", {}), spacer(), text("Right", {})])
```

#### `divider()`

A hairline rule. Inert.

```soli
column({"gap": 3}, [h2("Members"), divider(), rows])
```

#### `keyed(key, node)`

Gives a node a stable identity so the diff moves it instead of rebuilding it. Required on any list whose rows reorder.

```soli
items.map(fn(i) keyed(i["id"], row_for(i)))
```

### Typography

Text is a leaf primitive; these are conventional sizes and roles on top of it.

#### `text(content, style)`

A run of text, wrapped to its box. `line_clamp` truncates.

```soli
text("Hello", {"size": 3, "weight": "semibold"})
```

#### `h1(content) · h2(content)`

Heading sizes from the theme scale.

```soli
column({"gap": 2}, [h1("Dashboard"), h2("This week")])
```

#### `muted(content)`

Secondary text in the `text.muted` role.

```soli
muted("Last synced 3 minutes ago")
```

#### `code_block(code)`

Monospaced text on a sunken surface.

```soli
code_block("soli serve . --port 3000")
```

#### `stat(label, value, hint)`

A figure with its caption. The unit of a dashboard.

```soli
stat("Requests", "1.4M", "+12% this week")
```

### Actions

Every button is a `box` with a `click` handler and a text child — there is no button primitive. The variants differ only in their style roles.

#### `button(label, on_click)`

The primary action. `on_click` names a server event.

```soli
button("Save", "save")
```

#### `secondary_button(label, on_click)`

A raised surface rather than the accent role.

```soli
secondary_button("Cancel", "cancel")
```

#### `danger_button(label, on_click)`

The `danger` role, for destructive actions.

```soli
danger_button("Delete account", "destroy")
```

#### `ghost_button(label, on_click)`

Text only until hovered. For tertiary actions in dense toolbars.

```soli
ghost_button("Dismiss", "dismiss")
```

#### `button_variant(label, on_click, bg, fg)`

The builder the others call. Reach for it when you need a role pair the variants do not cover.

```soli
button_variant("Publish", "publish", "success.base", "success.on")
```

#### `icon_button(label, on_click, props)`

A square button holding a single glyph. `props` ride back on the event.

```soli
icon_button("✕", "close", {"id": row["id"]})
```

#### `loading_button(label, on_click, key)`

Swaps to a spinner while the round trip is in flight, keyed so the diff replaces only the label.

```soli
loading_button("Import", "import", "import-btn")
```

#### `local_button(label, program, after)`

Runs a verified bytecode program on the client `before` the round trip, then sends `after`. This is what makes a counter feel instant.

```soli
local_button("+", "state.count += 1", "increment")
```

### Input

Editable fields are the `input` and `textarea` primitives; everything else is composed. A field reports its value through the event it names.

#### `input(value, on_change)`

A single-line field. The event fires per keystroke with the value in `params`.

```soli
input(state["email"], "email_changed")
```

#### `sized_input(value, on_change, width)`

The same field at a fixed width, for inline and grid editing.

```soli
sized_input(cell, "cell_changed", 120)
```

#### `field(label, value, on_change)`

A labelled input — `labelled` plus `input`.

```soli
field("Email", state["email"], "email_changed")
```

#### `form(children, submit_label, on_submit)`

Fields plus a submit button. There is no browser form post — submit is an ordinary event.

```soli
form([
  field("Name", s["name"], "name_changed"),
  field("Email", s["email"], "email_changed")
], "Create", "create")
```

#### `checkbox(label, checked, on_toggle, props)`

A box, a tick and a label. `props` is how a row says which row it is.

```soli
checkbox(item["title"], item["done"], "toggle", {"id": item["id"]})
```

#### `switch(label, on, on_toggle, props)`

A checkbox that reads as a setting rather than a selection.

```soli
switch("Email alerts", s["alerts"], "toggle_alerts", {})
```

#### `slider(value, min, max, on_set)`

A track, a filled portion and a thumb, driven by pointer events.

```soli
slider(s["volume"], 0, 100, "set_volume")
```

#### `select(options, value, open, on_toggle, on_pick)`

A closed trigger plus, when `open`, an `overlay` of options. Open state lives in your state, not the client.

```soli
select(["Draft", "Live"], s["status"], s["open"], "toggle", "pick")
```

#### `select_sized(options, value, open, on_toggle, on_pick, min_width, grow)`

The same control with explicit width behaviour, for toolbars and grids.

```soli
select_sized(cols, v, open, "t", "p", 140, false)
```

#### `dropdown(anchor, content, open)`

Any node as the trigger, any tree as the panel. `select` is one use of it.

```soli
dropdown(icon_button("⋯", "toggle", {}), menu(items, "pick"), s["open"])
```

### Dates

One calendar engine, three selections. The month shown is state you hold, so navigation is an ordinary event.

#### `calendar(month, selected, range_start, range_end, on_pick, on_nav)`

The month grid itself. Pass a range to shade the days between.

```soli
calendar(s["month"], s["day"], null, null, "pick", "nav")
```

#### `date_picker(month, value, on_pick, on_nav)`

A field that opens the calendar.

```soli
date_picker(s["month"], s["due"], "pick", "nav")
```

#### `datetime_picker(month, value, hour, minute, on_pick, on_nav, on_time)`

The calendar plus hour and minute fields, emitting one ISO value.

```soli
datetime_picker(s["month"], s["at"], s["h"], s["m"],
                "pick", "nav", "time")
```

#### `date_range_picker(month, start, finish, on_pick, on_nav)`

The same grid collecting two dates; the days between are shaded as you move.

```soli
date_range_picker(s["month"], s["from"], s["to"], "pick", "nav")
```

### Feedback and status

Small, composed pieces that tell the person what happened. Tones are role names — `info`, `success`, `warning`, `danger` — not colours.

#### `badge(label, tone)`

A small pill for counts and states.

```soli
row({"gap": 2}, [badge("Live", "success"), badge("3", "danger")])
```

#### `chip(label, on_remove, props)`

A removable tag. Omit `on_remove` for a static one.

```soli
chip("rust", "remove_tag", {"tag": "rust"})
```

#### `spinner() · spinner_sized(size)`

An indeterminate wait, animated by the client's `spin` style key — no round trip per frame.

```soli
spinner_sized(20)
```

#### `progress(fraction)`

A determinate bar. `fraction` is 0.0 to 1.0.

```soli
progress(uploaded / total)
```

#### `skeleton(width, height)`

A placeholder block while data loads. Keeps the layout from jumping.

```soli
column({"gap": 2}, [skeleton(180, 12), skeleton(120, 12)])
```

#### `toast(message, tone)`

A transient message in an `overlay`, so it paints above the flow.

```soli
toast("Saved", "success")
```

#### `banner(message, tone, action_label, on_action)`

A persistent strip in the flow, optionally carrying one action.

```soli
banner("Your trial ends Friday", "warning", "Upgrade", "upgrade")
```

#### `empty_state(title, body, action_label, on_action)`

What a list shows when it has nothing. Worth building once.

```soli
empty_state("No invoices", "They will appear here once billed.",
            "Create one", "new_invoice")
```

#### `tooltip(content)`

A hint in an `overlay`. Pair it with a hover style key so it costs no round trip.

```soli
stack({}, [icon_button("?", "noop", {}), tooltip("Read-only")])
```

### Overlays and structure

Anything that floats uses the `overlay` primitive, which paints after its siblings. Open and closed is `your state` — the client holds none of it.

#### `dialog(title, body_children, actions)`

A modal over a scrim. `actions` is a list of buttons.

```soli
dialog("Delete project?",
       [muted("This cannot be undone.")],
       [secondary_button("Cancel", "close"),
        danger_button("Delete", "destroy")])
```

#### `sheet(side, children)`

A panel anchored to an edge — `"left"`, `"right"`, `"bottom"`.

```soli
sheet("right", [h2("Filters"), filter_controls])
```

#### `drawer(children)`

A left sheet, the navigation case, at one argument.

```soli
drawer(sidebar(links, active, "go"))
```

#### `popover(anchor, content, open)`

A panel positioned against a node rather than the viewport.

```soli
popover(button("Share", "toggle"), share_panel, s["open"])
```

#### `menu(items, on_pick)`

A list of choices in an overlay. Each item carries its own id back.

```soli
menu([{"id": "dup", "label": "Duplicate"},
      {"id": "del", "label": "Delete"}], "pick")
```

#### `tabs(names, active, on_select)`

A row of labels with the active one underlined.

```soli
tabs(["Overview", "Usage", "Billing"], s["tab"], "select_tab")
```

#### `accordion(sections, open_id, on_toggle)`

Sections that expand one at a time.

```soli
accordion([{"id": "a", "title": "General", "body": general}],
          s["open"], "toggle_section")
```

#### `stepper(steps, current)`

Progress through a sequence. Display only.

```soli
stepper(["Account", "Plan", "Payment"], 1)
```

#### `toolbar(children)`

A dense row on a raised surface, for actions above a table or canvas.

```soli
toolbar([icon_button("＋", "add", {}), icon_button("⟳", "refresh", {}),
         spacer(), select_sized(views, v, o, "t", "p", 120, false)])
```

### Navigation

Navigation is application state, not a URL: a click is an event and your handler decides what the next tree is.

#### `navbar(brand, links, active, on_go)`

A top bar with a brand and a link row.

```soli
navbar("Acme", ["Home", "Docs", "Pricing"], s["page"], "go")
```

#### `sidebar(links, active, on_go)`

A vertical nav column with the active entry highlighted.

```soli
sidebar(["Inbox", "Sent", "Drafts"], s["box"], "go")
```

#### `breadcrumb(crumbs, on_go)`

A trail. The last entry is the current place and is not a link.

```soli
breadcrumb([{"id": "root", "label": "Projects"},
            {"id": "p1", "label": "Acme"}], "go")
```

#### `pagination(page, pages, on_page)`

Previous, page numbers, next. Emits the page it wants.

```soli
pagination(s["page"], total_pages, "go_page")
```

#### `segmented(options, selected, on_select)`

Mutually exclusive options in one control. Fewer than five, or use `select`.

```soli
segmented(["Day", "Week", "Month"], s["range"], "set_range")
```

#### `tree_view(nodes, open_ids, on_toggle, depth)`

A nested, collapsible tree. Recursive: it calls itself at `depth + 1`.

```soli
tree_view(files, s["open"], "toggle_node", 0)
```

### Data display

Tables are composed rows, not a primitive. Past a few hundred, put them in a `list` so only the visible window lays out.

#### `table_header(labels, widths) · table_row(key, values, widths)`

A fixed-width table. `widths` is shared between the two so columns line up.

```soli
column({}, [
  table_header(["Name", "Plan"], [140, 80]),
  rows.map(fn(r) table_row(r["id"], [r["name"], r["plan"]], [140, 80]))
])
```

#### `data_grid(columns, rows, selected, editing, sort, on_select, on_sort, on_change, on_key)`

A sortable, inline-editable grid: cell selection, keyboard movement and per-column editability. The largest thing in the catalogue, and still plain Soli.

```soli
data_grid(columns, rows, s["sel"], s["edit"], s["sort"],
          "select", "sort", "change", "key")
```

#### `avatar(src, size)`

A circular image by content hash.

```soli
avatar(user["photo"], 40)
```

#### `initial_avatar(letter, tone, size)`

The fallback when there is no photo: a letter on a toned disc, no asset to fetch.

```soli
initial_avatar("A", "accent", 40)
```

### Charts and canvas

All four charts are one `canvas` node whose `paths` prop is a list of `[kind, colour, numbers…]`. The server resolves the colour before encoding, so the client never parses a string while painting.

#### `canvas(width, height, paths)`

The primitive underneath. Path kinds cover polylines, rectangles, areas, circles and arcs.

```soli
canvas(200, 60, [["line", "accent.base", 0,50, 40,20, 80,35]])
```

#### `chart_line(values, w, h)`

A polyline over a faint grid, scaled to the maximum.

```soli
chart_line([4, 9, 6, 12, 8, 15], 200, 60)
```

#### `chart_area(values, w, h)`

The same line, closed to the baseline and filled.

```soli
chart_area([4, 9, 6, 12, 8, 15], 200, 60)
```

#### `chart_bar(values, w, h)`

One rectangle per value, gap derived from the count.

```soli
chart_bar([4, 9, 6, 12, 8, 15], 200, 60)
```

#### `chart_donut(parts, w, h)`

Arcs from a list of `[label, value]` pairs, each in its own role.

```soli
chart_donut([["Pro", 60], ["Free", 30], ["Trial", 10]], 80, 80)
```

### Media

Images, sound and video are referenced by `BLAKE3 hash`, never by path. The client asks for a hash it does not have; an asset is immutable, so it is fetched once and cached forever.

#### `image(src, width, height)`

A raster asset by content hash.

```soli
image(asset("logo.png"), 120, 40)
```

#### `audio(src, props, on)`

Draws nothing, plays. Control it with props and events.

```soli
audio(asset("chime.ogg"), {"autoplay": true}, {})
```

#### `video(src, props, style, on)`

Decoded in the sandboxed worker. GIF and animated WebP arrive through the same node.

```soli
video(asset("demo.webm"), {"loop": true}, {"radius": 2}, {})
```

#### `media_scrubber(width, at, duration, on_seek, props)`

A seek bar for an audio or video node.

```soli
media_scrubber(200, s["at"], s["len"], "seek", {})
```

### Theme and responsiveness

The client resolves roles against light or dark mode, density and font scale. The server never learns which.

#### `theme_toggle()`

Flips the client's mode locally — `theme.toggle()` in a local handler, so no round trip and no server state.

```soli
row({}, [spacer(), theme_toggle()])
```

#### `bp(width) · bp_min(width, name) · bp_px(name)`

Breakpoint helpers. The viewport reaches the handler, so the tree you build `is` the responsive answer — there is no media query.

```soli
cols = bp(params["viewport"]["w"]) == "sm" ? 1 : 3
```

#### `with_state(state, root)`

Attaches the state a local handler may read and write, at the root of the tree.

```soli
with_state({"count": s["count"]}, column({}, children))
```

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
