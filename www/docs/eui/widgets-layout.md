# EUI — layout, overlays and navigation

Containers, overlays, navigation and the theme helpers.

> **Where these live.** The catalogue is not a shipped Soli library: it is a single file,
> `app/controllers/eui_builders.sl`, generated into new applications by `soli new <app> --eui`
> and present in the `counter-app` example. Copy it, edit it — that is the intended use.

## Layout and containers

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

#### `pane_px(name)`

_No description in the source._

```soli
px >= pane_px(name)
```

<sub>Real use — `eui_builders.sl (inside `pane_min`):1128`.</sub>

#### `pane_bp(px)`

_No description in the source._

```soli
badge(pane_bp(px), tone)
```

<sub>Real use — `app/controllers/live_controller.sl:864`.</sub>

#### `pane_min(px, name)`

_No description in the source._

```soli
roomy = pane_min(px, "md")
```

<sub>Real use — `app/controllers/live_controller.sl:860`.</sub>

#### `split_span(extent, bar)`

The room the two panels share, once the divider has taken its own.

```soli
span = split_span(extent, bar)
```

<sub>Real use — `eui_builders.sl (inside `split_sizes`):1144`.</sub>

#### `split_sizes(extent, fraction, min_a, min_b, bar)`

`fraction` is per mille — an integer, so it survives a round trip through state and a local handler's props without ever being a float. Both conversions round rather than truncate, which is what makes the trip exact: a divider dropped at a pixel and rebuilt from its fraction lands on the same pixel,

```soli
sizes = split_sizes(extent, fraction, min_a, min_b, bar)
```

<sub>Real use — `eui_builders.sl (inside `split_pane`):1244`.</sub>

#### `split_at(extent, at, min_a, min_b, bar)`

The pointer's position along the axis becomes the fraction the divider sits at. Clamped to both minimums, so a drag that runs past a panel's floor stops there rather than inverting the pair — and the clamp lives here, once, instead of in every application that draws a split.

```soli
state[name] = split_at(extent, at, min_a, min_b, bar)
```

<sub>Real use — `eui_builders.sl (inside `split_drag`):1288`.</sub>

#### `split_panel(build, px, across: Bool, cross)`

_No description in the source._

```soli
split_panel(o["a"], sizes[0], across, cross),
```

<sub>Real use — `eui_builders.sl (inside `split_pane`):1258`.</sub>

#### `split_divider(key, across: Bool, bar, cross, fraction, on_drag, dragging: Bool, label)`

The divider is not a `control`: it is a separator, its press has to reach the server as well as restyle locally, and `control` would put its own `pointer_down` over the top of that. It is keyed so the local chunk can name it, and it holds `key_down`, which is what puts it in the Tab order — so a spl

```soli
split_divider(key + ":bar", across, bar, cross, fraction, on_drag, o["dragging"] == true, o["label"]),
```

<sub>Real use — `eui_builders.sl (inside `split_pane`):1259`.</sub>

#### `split_pane(o)`

key       required; the divider is keyed from it dir       "row" | "column" size      the container's extent along the split axis, in px cross     the extent across it; "100%" if absent fraction  per mille, 0..1000 min_a / min_b   the smallest each panel may become, in px bar       the divider's thi

```soli
split_pane({
```

<sub>Real use — `app/controllers/live_controller.sl:825`.</sub>

#### `split_drag(state, params, name, dir, extent, min_a, min_b, bar)`

The four events a split sends, folded into a component's state. `name` is the state key holding the fraction; `name + "_drag"` holds whether a drag is in flight. An application writes one line in its handler and is done. One `pointer_move` while the divider is held: the point along the axis is where

```soli
state = split_drag(state, params, name, dir, extent, min_a, min_b, bar)
```

<sub>Real use — `eui_builders.sl (inside `split_event`):1319`.</sub>

#### `split_keys(state, params, name, dir, step)`

The keyboard's half of the same divider: an arrow moves it by a step, Home puts it back in the middle, and the result is held inside the thousandths the fraction is measured in.

```soli
state = split_keys(state, params, name, dir, step)
```

<sub>Real use — `eui_builders.sl (inside `split_event`):1321`.</sub>

#### `split_event(state, params, name, dir, extent, min_a, min_b, bar)`

_No description in the source._

```soli
"split_x" => split_event(state, params, "split_x", "row", gallery_split_extent(state), 220, 260, 6),
```

<sub>Real use — `app/controllers/live_controller.sl:2500`.</sub>

#### `table_row(key, values, widths)`

_No description in the source._

```soli
table_row(i, [
```

<sub>Real use — `app/controllers/live_controller.sl:274`.</sub>

## Overlays and structure

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

## Navigation

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

## Theme and responsiveness

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
