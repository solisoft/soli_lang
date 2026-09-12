# EUI — content, data and charts

Typography, media, tables, status and charts.

> **Where these live.** The catalogue is not a shipped Soli library: it is a single file,
> `app/controllers/eui_builders.sl`, generated into new applications by `soli new <app> --eui`
> and present in the `counter-app` example. Copy it, edit it — that is the intended use.

## Typography

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

#### `icon(name, style)`

A named vector icon, stroked by the client from its own table. The name is a prop, not text: an icon is not a character, so it takes `fg` like a label but is never shaped, never falls back to a symbols face, and never reaches a screen reader as the glyph it happens to resemble. With no size of its o

```soli
"c": lit ? [icon(
```

<sub>Real use — `eui_builders.sl (inside `checkbox`):610`.</sub>

#### `h2(content)`

_No description in the source._

```soli
h2("Split panes"),
```

<sub>Real use — `app/controllers/live_controller.sl:823`.</sub>

#### `icon_box_px(size)`

_No description in the source._

```soli
box = icon_box_px(size)
```

<sub>Real use — `eui_builders.sl (inside `icon_button`):2535`.</sub>

#### `text_link(label, on_click, props = {})`

A link is a button that goes somewhere rather than doing something, so it is text in the accent colour and not a box: a row of links must not read as a row of buttons, because what they promise is different. It declares the `link` role, which is what a screen reader announces it by — the colour is n

```soli
[text_link(part["sku"], "inv_pick", {"sku": part["sku"]}), muted(part["warehouse"])]
```

<sub>Real use — `app/controllers/live_controller.sl:1824`.</sub>

#### `code_viewer(code, opts)`

A read-only code viewer with line numbers and scrolling. Displays code in a monospace font with a pinned gutter (line numbers stay visible while scrolling horizontally). Gutter/code lines stay pixel-aligned because both use the same font_size, which determines line-height. `opts` may include {"langu

```soli
out = out.concat([code_viewer(code, {
```

<sub>Real use — `app/controllers/markdown_builders.sl:307`.</sub>

## Media

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

#### `text_interned(content, style)`

A text whose content repeats across many nodes: interned as an atom, so the wire carries it once per session.

```soli
text_interned("☻", {"size": 1}),
```

<sub>Real use — `app/controllers/chat_controller.sl:1754`.</sub>

#### `post_avatar(post, size)`

A face when there is one, a coloured initial when there is not: a real timeline brings pictures, the sample brings letters, and the card treats them the same.

```soli
post_avatar(post, 40),
```

<sub>Real use — `eui_builders.sl (inside `post_card`):3801`.</sub>

#### `post_action(glyph, count, on_click, props, active)`

One action under a post: a glyph and a count, clickable, carrying the post id so one handler serves every post.

```soli
post_action("↩", post["replies"], "noop", {"id": post["id"]}, false),
```

<sub>Real use — `eui_builders.sl (inside `post_card`):3782`.</sub>

#### `media_clock(ms)`

_No description in the source._

```soli
muted(media_clock(state["video_at"] ?? 0) + " / 0:01")
```

<sub>Real use — `app/controllers/live_controller.sl:346`.</sub>

#### `media_button(on, event, props)`

A play/pause button of the size these cards use.

```soli
media_button(playing, "video", {}),
```

<sub>Real use — `app/controllers/live_controller.sl:344`.</sub>

#### `post_media(post, play)`

_No description in the source._

```soli
picture = post_media(post, play)
```

<sub>Real use — `eui_builders.sl (inside `post_card`):3778`.</sub>

#### `post_card(post, liked, play, height)`

_No description in the source._

```soli
built = keyed(i, post_card(post, liked, play, feed_card_height(post)))
```

<sub>Real use — `app/controllers/live_controller.sl:2875`.</sub>

## Data display

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

#### `grid_sort_rows(rows, col, dir)`

_No description in the source._

```soli
state["grid_rows"] = grid_sort_rows(state["grid_rows"], col, dir)
```

<sub>Real use — `app/controllers/live_controller.sl:545`.</sub>

#### `grid_col_editable(col)`

_No description in the source._

```soli
return grid_col_editable(col) if col["id"] == id
```

<sub>Real use — `app/controllers/live_controller.sl:443`.</sub>

#### `grid_col_align(col)`

_No description in the source._

```soli
align = grid_col_align(col)
```

<sub>Real use — `eui_builders.sl (inside `grid_cell`):1402`.</sub>

#### `grid_cell(row_id, col, value, selected, editing, open, on_select, on_change, on_key)`

_No description in the source._

```soli
grid_cell(
```

<sub>Real use — `eui_builders.sl (inside `grid_row`):1516`.</sub>

#### `grid_row(record, columns, selected, editing, on_select, on_change, on_key)`

_No description in the source._

```soli
body = rows.map(fn(r) { grid_row(r, columns, selected, editing, on_select, on_change, on_key) })
```

<sub>Real use — `eui_builders.sl (inside `data_grid`):1599`.</sub>

#### `grid_header(columns, sort, on_sort)`

_No description in the source._

```soli
[grid_header(columns, sort, on_sort), inner]
```

<sub>Real use — `eui_builders.sl (inside `data_grid`):1617`.</sub>

## Feedback and status

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

#### `spinner_sized(size)`

_No description in the source._

```soli
"c": chat_ready ? [text(chat_said, {"size": 0, "clamp": 1, "fg": "text.default"}), text_interned("→", {"size": 1, "fg": "accent.base"})] : [spinner_sized(14), text(chat_said, {"size": 0, "fg": "text.muted"})]
```

<sub>Real use — `app/controllers/chat_controller.sl:2844`.</sub>

#### `alert(title, message, on_close, opts)`

An alert: one thing to say and nothing to decide, so one button. The handler fires on the button, not on the backdrop — a dialog that closes when the pointer slips is a dialog that loses what it was asking.  `opts`: {"ok": "Got it"}

```soli
layers = layers.concat([alert(
```

<sub>Real use — `app/controllers/live_controller.sl:2608`.</sub>

#### `confirm(title, message, on_confirm, on_cancel, opts)`

A confirm: a question with two answers. The affirmative sits last, where the eye ends up, and wears `danger` when it destroys something — the button should say what it will do before the sentence above it is read.  `opts`: {"ok": "Delete", "cancel": "Keep", "danger": true}

```soli
layers = layers.concat([confirm(
```

<sub>Real use — `app/controllers/live_controller.sl:2593`.</sub>

## Charts and canvas

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

#### `chart_max(values)`

_No description in the source._

```soli
top = chart_max(values)
```

<sub>Real use — `eui_builders.sl (inside `chart_points`):3259`.</sub>

#### `chart_points(values, w, h)`

A series scaled into `w × h` with 4 px of breathing room, as [x, y] pairs.

```soli
points = chart_points(values, w, h)
```

<sub>Real use — `eui_builders.sl (inside `chart_line`):3417`.</sub>

#### `chart_grid(w, h)`

Four hairlines, so a series has something to be read against.

```soli
drawing = canvas(w, h, chart_grid(w, h).concat([line]).concat(dots))
```

<sub>Real use — `eui_builders.sl (inside `chart_line`):3426`.</sub>

#### `flatten_points(points)`

_No description in the source._

```soli
line = [0, sounding ? skin["inst"] : skin["dim"], 1].concat(flatten_points(points))
```

<sub>Real use — `app/controllers/tracker_controller.sl:1066`.</sub>

#### `chart_role(i)`

The five roles a chart spends, in order and never cycled. Past the fifth there is no sixth hue to reach for — a generated one is indistinguishable from one already here to a reader with a colour vision deficiency — so the tail goes to the de-emphasis ink and the chart is expected to name it "other",

```soli
swatch = {"k": "box", "s": {"width": 10, "height": 10, "radius": 4, "bg": chart_role(i)}}
```

<sub>Real use — `eui_builders.sl (inside `chart_donut_legend_row`):3474`.</sub>

#### `chart_wash_style(width, lit)`

One column of the plot, behind the drawing. `lit` is the state under the pointer; the width is in the record because the handler has to declare the same box it is repointing, not a narrower one.

```soli
"styles": {"lit": chart_wash_style(width, true), "shown": chart_chip_style(true)}
```

<sub>Real use — `eui_builders.sl (inside `chart_band`):3382`.</sub>

#### `chart_chip_style(shown)`

The tooltip: a chip that is always there and is transparent until the pointer is in its band. Fading one in costs no layout; mounting one would.

```soli
"styles": {"lit": chart_wash_style(width, true), "shown": chart_chip_style(true)}
```

<sub>Real use — `eui_builders.sl (inside `chart_band`):3382`.</sub>

#### `chart_quoted(s)`

A string as the local language's source will read it back: a chunk's `set_text` takes a literal, and a literal wants its quotes.

```soli
says = "dv_" + id + ".text = " + chart_quoted(reading) + "; dl_" + id + ".text = " + chart_quoted(label)
```

<sub>Real use — `eui_builders.sl (inside `chart_donut_legend_row`):3472`.</sub>

#### `chart_spans(centres, w)`

Where the bands meet: the midpoint between neighbouring marks, rounded once so the columns still add up to the plot's width — a band per mark, from the left edge of the plot to its right.

```soli
spans = chart_spans(points.map(fn(p) { p[0] }), w)
```

<sub>Real use — `eui_builders.sl (inside `chart_line`):3427`.</sub>

#### `chart_band(id, i, width, label)`

One band: an invisible box over its share of the plot, carrying the chip and the two handlers that light the pair.

```soli
bands = range(0, count).map(fn(i) { chart_band(id, i, spans[i], labels[i]) })
```

<sub>Real use — `eui_builders.sl (inside `chart_layers`):3401`.</sub>

#### `chart_layers(id, spans, labels, w, h, drawing)`

The three layers, stacked on the plot the drawing was scaled into. The strips are the plot itself — `w − 8` by `h − 8`, centred — so a band sits exactly over the marks `chart_points` placed.

```soli
chart_layers(id, spans, chart_labels(values), w, h, drawing)
```

<sub>Real use — `eui_builders.sl (inside `chart_line`):3428`.</sub>

#### `chart_labels(values)`

_No description in the source._

```soli
chart_layers(id, spans, chart_labels(values), w, h, drawing)
```

<sub>Real use — `eui_builders.sl (inside `chart_line`):3428`.</sub>

#### `chart_donut_legend_row(id, i, part, label, total)`

A donut has no bands: an arc is not a box, and a quadrant is not an arc. Its legend is the thing under the pointer instead, and what it shows is the reading in the hole — one `set_text` for the number, one for the name, which is what a chunk is for (07 §1).

```soli
chart_donut_legend_row(id, j, parts[j], labels[j] ?? ("Part " + str(j + 1)), total)
```

<sub>Real use — `eui_builders.sl (inside `chart_donut`):3523`.</sub>

## Dev bar

#### `dev_figure(label, value, tone)`

_No description in the source._

```soli
dev_figure("event", stats["event"].blank? ? "—" : stats["event"], "accent.base"),
```

<sub>Real use — `eui_builders.sl (inside `dev_bar`):3180`.</sub>

#### `dev_wire_tone(ops)`

How heavy the last patch was, as a colour: a render that sends a few ops is what the design is for, and one that sends the tree is worth noticing.

```soli
dev_figure("ops", str(stats["ops"]), dev_wire_tone(stats["ops"])),
```

<sub>Real use — `eui_builders.sl (inside `dev_bar`):3183`.</sub>

#### `dev_bar(stats, shown = true)`

_No description in the source._

```soli
layers = layers.concat([dev_bar(eui_stats(), state["devbar"] ?? true)])
```

<sub>Real use — `app/controllers/live_controller.sl:2615`.</sub>
