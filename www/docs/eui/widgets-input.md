# EUI — actions, input and dates

Everything the viewer types, picks or presses.

> **Where these live.** The catalogue is not a shipped Soli library: it is a single file,
> `app/controllers/eui_builders.sl`, generated into new applications by `soli new <app> --eui`
> and present in the `counter-app` example. Copy it, edit it — that is the intended use.

## Actions

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

## Input

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

#### `textarea(value, on_change, o = {})`

The multi-line field, and the only editable kind the client has besides `input`. The same node in every respect but that kind, and the kind is what makes the client wrap the text, put the caret on the line the pointer landed on, and keep `Enter` for a newline instead of a submit.  `o["rows"]` is a f

```soli
textarea(state["draft"].to_s, "draft", {
```

<sub>Real use — `app/controllers/chat_controller.sl:2702`.</sub>

#### `radio(label, selected, on_pick, props, o = {})`

A radio is a checkbox that cannot be unticked and knows about its neighbours: choosing one is choosing away from the others. So the *group* owns the value — there is no `checked` argument that a caller could set on two of them at once — and each button carries only what it stands for, in `props`, wh

```soli
radio(opt, opt == value, on_pick, {"value": opt, "group": name}, {"size": size, "disabled": disabled})
```

<sub>Real use — `eui_builders.sl (inside `radio_group`):760`.</sub>

#### `radio_group(options, value, on_pick, o = {})`

The group is what a screen reader is told about — `radio_group` is a role the client knows — and it is what makes the keys unique: every button in a group takes the group's name, so two groups of "Yes"/"No" on one page do not restyle each other. `o["direction"]` is `"column"` unless a row is asked f

```soli
radio_group(
```

<sub>Real use — `app/controllers/live_controller.sl:1312`.</sub>

#### `select_option(label, selected, on_pick, min_width)`

_No description in the source._

```soli
dropdown(anchor, options.map(fn(o) { select_option(o, o == value, on_pick, min_width) }), open, DROPDOWN_MAX_PX)
```

<sub>Real use — `eui_builders.sl (inside `select_sized`):2363`.</sub>

#### `number_stepped(value, delta, o)`

What the − and + buttons mean, for the handler that owns the value. The clamp is the one `number_within?` judges by, so a stepper cannot walk a field into an error nobody typed; an unreadable value steps from the floor, because "" + 1 has to be something.

```soli
set_key(state, key, number_stepped(state[key], props["delta"], o))
```

<sub>Real use — `app/controllers/live_controller.sl:2230`.</sub>

#### `number_text(n)`

A number as a field holds it: "3", not "3.0", unless there is a fraction to keep.

```soli
number_text(n)
```

<sub>Real use — `eui_builders.sl (inside `number_stepped`):2850`.</sub>

#### `field_error(value, judge, complaint, o)`

What is wrong with this value, as a sentence, or "" when nothing is.  An empty required field is not wrong yet. A form that opens already shouting at the person who has not typed in it is a form that has decided they were going to get it wrong; `o["submitted"]` is what says they have had their turn,

```soli
error = field_error(value, fn(said) { true }, "", o)
```

<sub>Real use — `eui_builders.sl (inside `text_field`):2925`.</sub>

#### `field_bad(error, o)`

_No description in the source._

```soli
bad = field_bad(error, o)
```

<sub>Real use — `eui_builders.sl (inside `text_field`):2926`.</sub>

#### `field_note(o)`

The line under a control: what is wrong with the value, or the hint that was there before anything was wrong with it. Never both — a field that explains itself twice is a field nobody reads.

```soli
column({"gap": 1, "width": "100%"}, head.concat([control]).concat(field_note(o)))
```

<sub>Real use — `eui_builders.sl (inside `field_shell`):2896`.</sub>

#### `field_shell(label, control, o)`

_No description in the source._

```soli
field_shell(
```

<sub>Real use — `eui_builders.sl (inside `text_field`):2927`.</sub>

#### `field_style(bad, o)`

The style every field control shares. It fills its column unless a width was asked for, and it goes danger when what is in it is wrong — a *colour*, over a border the resting style already reserved, so a field that turns red does not move the fields under it.

```soli
"style": field_style(bad, o),
```

<sub>Real use — `eui_builders.sl (inside `text_field`):2930`.</sub>

#### `field_props(label, error, bad, o)`

What the field says about itself. The client reads `label`, `description`, `required` and `invalid` by name (03 §4), so a wrong value is announced as wrong rather than only painted that way, and the sentence a sighted person reads under the field is the one a screen reader is given.

```soli
"props": field_props(label, error, bad, o)
```

<sub>Real use — `eui_builders.sl (inside `text_field`):2931`.</sub>

#### `text_field(label, value, on_change, o = {})`

A line of anything. It judges nothing on its own; `o["error"]` and `o["required"]` are the only ways it goes wrong.

```soli
text_field("Reference", state["nf_ref"], "nf_ref", {
```

<sub>Real use — `app/controllers/live_controller.sl:1472`.</sub>

#### `email_field(label, value, on_change, o = {})`

_No description in the source._

```soli
email_field("Contact", state["nf_email"], "nf_email", {
```

<sub>Real use — `app/controllers/live_controller.sl:1478`.</sub>

#### `number_field(label, value, on_change, o = {})`

`o["min"]`, `o["max"]` and `o["step"]` are the bounds and the stride; `o["on_step"]` adds the two buttons and names the event they send, with the direction in `params["props"]["delta"]`. The handler does the arithmetic — `number_stepped` is it — because the value is the server's, and a widget that s

```soli
number_field("Quantity", state["nf_qty"], "nf_qty", {
```

<sub>Real use — `app/controllers/live_controller.sl:1483`.</sub>

#### `number_complaint(o)`

"A number", or the bounds, because "invalid" tells nobody what to type instead.

```soli
error = field_error(value, fn(said) { number_within?(said, o["min"], o["max"]) }, number_complaint(o), o)
```

<sub>Real use — `eui_builders.sl (inside `number_field`):2957`.</sub>

#### `textarea_field(label, value, on_change, o = {})`

The multi-line one. `o["rows"]` is the floor the empty box keeps.

```soli
textarea_field("Notes", state["nf_notes"], "nf_notes", {
```

<sub>Real use — `app/controllers/live_controller.sl:1516`.</sub>

## Dates

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

#### `month_label(month)`

One engine, three pickers. `month` is "YYYY-MM"; days are ISO "YYYY-MM-DD" strings, which compare correctly as strings.

```soli
month_label(month),
```

<sub>Real use — `eui_builders.sl (inside `calendar`):2619`.</sub>

#### `month_shift(month, delta)`

_No description in the source._

```soli
return set_key(state, name + "_month", month_shift(state[name + "_month"], props["delta"])) if event == name + "_nav"
```

<sub>Real use — `app/controllers/live_controller.sl:2217`.</sub>

#### `weekday_index(day)`

_No description in the source._

```soli
blanks = range(0, weekday_index(first_day)).map(fn(i) { day_blank() })
```

<sub>Real use — `eui_builders.sl (inside `calendar`):2608`.</sub>

#### `two_digits(n)`

_No description in the source._

```soli
iso = month + "-" + two_digits(d)
```

<sub>Real use — `eui_builders.sl (inside `calendar`):2610`.</sub>

#### `icon_or_glyph(glyph, name, size)`

An icon button draws a named icon when it is given one, and the character it was handed when it is not. The catalogue is mid-move from the second to the first, and both have to work while it is.

```soli
"c": [icon_or_glyph(glyph, o["icon"], size)]
```

<sub>Real use — `eui_builders.sl (inside `icon_button`):2549`.</sub>

#### `day_cell(iso, label, selected, in_range, on_pick)`

_No description in the source._

```soli
day_cell(iso, str(d), selected.includes?(iso), shaded, on_pick)
```

<sub>Real use — `eui_builders.sl (inside `calendar`):2612`.</sub>

#### `time_select(time, hour_open, min_open, on_hour_toggle, on_min_toggle, on_hour, on_min)`

Two selects, twenty-four hours and sixty minutes, so the clock cannot hold anything but HH:MM — there is no free text to parse and no "25:61" to reject. Both selects are the server's to open, like every other one.

```soli
time_select(time, hour_open, min_open, on_hour_toggle, on_min_toggle, on_hour, on_min)
```

<sub>Real use — `eui_builders.sl (inside `datetime_picker`):2711`.</sub>

#### `picker_panel(children)`

_No description in the source._

```soli
fn() { picker_panel([calendar(o["month"], day.present? ? [day] : [], "", "", o["on_pick"], o["on_nav"])]) }
```

<sub>Real use — `eui_builders.sl (inside `date_field`):3084`.</sub>

#### `picker_field(o, caption, empty, make)`

_No description in the source._

```soli
picker_field(
```

<sub>Real use — `eui_builders.sl (inside `date_field`):3080`.</sub>

#### `date_field(o)`

_No description in the source._

```soli
date_field({
```

<sub>Real use — `app/controllers/live_controller.sl:1490`.</sub>

#### `datetime_field(o)`

_No description in the source._

```soli
datetime_field({
```

<sub>Real use — `app/controllers/live_controller.sl:1500`.</sub>

#### `date_range_field(o)`

_No description in the source._

```soli
[date_range_field({
```

<sub>Real use — `app/controllers/live_controller.sl:1295`.</sub>
