# EUI — internal helpers

The functions the catalogue calls on itself. They are documented so the file can be read end to end, not because they are API: copy the file into your application and they are yours to rename.

> **Where these live.** The catalogue is not a shipped Soli library: it is a single file,
> `app/controllers/eui_builders.sl`, generated into new applications by `soli new <app> --eui`
> and present in the `counter-app` example. Copy it, edit it — that is the intended use.

## Internals and helpers

#### `node(kind, style, children)`



Internal to the catalogue — it composes part of a widget rather than being called from a view. Listed for completeness; treat it as implementation detail.

```soli
return node("box", flat, children)
```

<sub>Real use — `app/controllers/tracker_controller.sl:852`.</sub>

#### `editable(kind, value, on_change, o)`

What both of them are. The border is reserved at rest and only coloured later, for the reason `button_variant` gives below: a border that appears when a value goes wrong would shove every field under it sideways.

Internal to the catalogue — it composes part of a widget rather than being called from a view. Listed for completeness; treat it as implementation detail.

```soli
editable("input", value, on_change, o)
```

<sub>Real use — `eui_builders.sl (inside `input`):138`.</sub>

#### `space_px(ix, density)`



Internal to the catalogue — it composes part of a widget rather than being called from a view. Listed for completeness; treat it as implementation detail.

```soli
inner = erp_content_px(lay) - 2 * space_px(lay["wide"] ? 6 : 4, lay["density"]) - 2 * space_px(5, lay["density"])
```

<sub>Real use — `app/controllers/live_controller.sl:808`.</sub>

#### `bp_px(name)`



Internal to the catalogue — it composes part of a widget rather than being called from a view. Listed for completeness; treat it as implementation detail.

```soli
width >= bp_px(name)
```

<sub>Real use — `eui_builders.sl (inside `bp_min`):235`.</sub>

#### `bp_min(width, name)`



Internal to the catalogue — it composes part of a widget rather than being called from a view. Listed for completeness; treat it as implementation detail.

```soli
wide = bp_min(w, "md")
```

<sub>Real use — `app/controllers/tracker_controller.sl:1358`.</sub>

#### `restyle(n, patch)`

Narrowing a widget after it is built only reaches its resting style: the hover and pressed styles are declared inside the local handlers, where a `n["s"]["pad"] = …` cannot see them. The pointer then snaps the node back to the catalogue's geometry, which is the same layout shift as a border that onl

Internal to the catalogue — it composes part of a widget rather than being called from a view. Listed for completeness; treat it as implementation detail.

```soli
return restyle(
```

<sub>Real use — `app/controllers/tracker_controller.sl:928`.</sub>

#### `size_spec(size)`



Internal to the catalogue — it composes part of a widget rather than being called from a view. Listed for completeness; treat it as implementation detail.

```soli
m = size_spec(size)
```

<sub>Real use — `eui_builders.sl (inside `control_metrics`):383`.</sub>

#### `control_metrics(size)`

Box geometry only. The text scale is not here: `size` styles a text node, and only `fg` inherits, so a label takes its size from `control_text_size`.

Internal to the catalogue — it composes part of a widget rather than being called from a view. Listed for completeness; treat it as implementation detail.

```soli
base = control_metrics(size).merge({
```

<sub>Real use — `eui_builders.sl (inside `control`):482`.</sub>

#### `control_text_size(size)`



Internal to the catalogue — it composes part of a widget rather than being called from a view. Listed for completeness; treat it as implementation detail.

```soli
"size": control_text_size(size),
```

<sub>Real use — `eui_builders.sl (inside `checkbox`):634`.</sub>

#### `checkbox_box_px(size)`



Internal to the catalogue — it composes part of a widget rather than being called from a view. Listed for completeness; treat it as implementation detail.

```soli
box = checkbox_box_px(size)
```

<sub>Real use — `eui_builders.sl (inside `checkbox`):592`.</sub>

#### `control_px(size, density)`



Internal to the catalogue — it composes part of a widget rather than being called from a view. Listed for completeness; treat it as implementation detail.

_Defined but never called in the catalogue — dead code as it stands._

#### `tone_resting(tone, lit)`



Internal to the catalogue — it composes part of a widget rather than being called from a view. Listed for completeness; treat it as implementation detail.

```soli
base = base.merge(tone_resting(tone, lit))
```

<sub>Real use — `eui_builders.sl (inside `control`):491`.</sub>

#### `a11y_props(o)`

The semantics a widget declares about itself. Props are a generic bag the client already reads by name, so these cost no wire change and an older client ignores them; what they buy is a checkbox that reaches a screen reader as a checkbox rather than as a button named by its label.

Internal to the catalogue — it composes part of a widget rather than being called from a view. Listed for completeness; treat it as implementation detail.

```soli
props = a11y_props(o)
```

<sub>Real use — `eui_builders.sl (inside `control`):503`.</sub>

#### `stateful(base, tone, on)`

Wire the four pointer events onto an already-final resting style. `base` must be the style the node actually carries — the deltas are merged over it, so whatever the caller changed is already inside every state.

Internal to the catalogue — it composes part of a widget rather than being called from a view. Listed for completeness; treat it as implementation detail.

```soli
built["on"] = here ? {"click": "go_room"} : stateful(resting, {
```

<sub>Real use — `app/controllers/chat_controller.sl:1590`.</sub>

#### `control(o)`

One options hash, so a widget gains a capability without its callers changing:  key       required and unique. 07 §3 names a node by key and the arena's key map is flat, so a duplicate silently restyles someone else. kind      "box" by default. tone      a TONES name; "quiet" by default. size      "

Internal to the catalogue — it composes part of a widget rather than being called from a view. Listed for completeness; treat it as implementation detail.

```soli
control({
```

<sub>Real use — `eui_builders.sl (inside `checkbox`):619`.</sub>

#### `lazy(id, shown, placeholder, make)`

A dialog is a stack: a dimming overlay, then the panel, centred. A region of the tree that is built only when something asks for it.  `make` is a thunk — `fn() { ... }` — and it is not called at all unless `id` is in `shown`. That is the whole mechanism: an unopened modal, an unselected tab and a co

Internal to the catalogue — it composes part of a widget rather than being called from a view. Listed for completeness; treat it as implementation detail.

```soli
[lazy("forecast", shown, placeholder, fn() { erp_forecast_body(state, lay) })]
```

<sub>Real use — `app/controllers/live_controller.sl:1194`.</sub>

#### `chip_remove(on_remove, props, label)`

The × that drops a chip. A button carries its hover and press styles with it, so reusing one here and narrowing only the live style let the × jump back to a button's padding under the pointer. This one is a fixed box: what changes on hover is the colour, which 03 §5 animates without ever running lay

Internal to the catalogue — it composes part of a widget rather than being called from a view. Listed for completeness; treat it as implementation detail.

```soli
parts = parts.concat([chip_remove(on_remove, props, label)]) if on_remove.present?
```

<sub>Real use — `eui_builders.sl (inside `chip`):1742`.</sub>
