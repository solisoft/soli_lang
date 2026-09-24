# EUI — styling

There is no cascade and no selector. A style is a hash of the protocol's own
vocabulary — `display`, `gap`, `pad`, `bg`, `fg`, `size`, `weight`, `radius`,
`width`, `align`, `justify`, `cursor` — and colours are **roles**:
`"accent.base"`, `"text.muted"`, `"surface.raised"`. The client resolves roles
against the viewer's light or dark mode, density and font scale, so the same
view is right in dark mode without the server knowing. A literal `"#RRGGBB"`
is available for a brand mark and wrong for a surface.

## The colour roles

There are **33** of them. A role is resolved by the client against the
viewer's light or dark mode, density and font scale, so the same view is right in
both without the server knowing which one it is.

| Roles | For |
|-------|-----|
| `surface.base` `surface.raised` `surface.sunken` `surface.overlay` | Backgrounds, from the page itself to an overlay. |
| `text.default` `text.muted` `text.inverted` `text.disabled` | Foreground text, including the disabled and inverted cases. |
| `accent.base` `accent.hover` `accent.active` `accent.on` | The application's own colour, and its interaction states. |
| `success.base` `success.subtle` `success.on` | An outcome that went well. |
| `warning.base` `warning.subtle` `warning.on` | Something the viewer should look at. |
| `danger.base` `danger.subtle` `danger.on` | Destruction, or an error. |
| `info.base` `info.subtle` `info.on` | Neutral information. |
| `border.subtle` `border.default` `border.strong` | Rules and outlines, by weight. |
| `focus.ring` | The keyboard focus ring. |
| `series.1` `series.2` `series.3` `series.4` `series.5` | The categorical series of a chart, in fixed order. |

The `series.*` family is the newest: it exists so a chart names a categorical
series rather than hard-coding a hex, and so two charts in the same application
agree on which colour the second series is. A view naming a role that does not
exist is refused — `EUI: unknown colour role 'series.9'` — rather than silently
drawn in a default colour.

```soli
column({"bg": "surface.raised", "pad": 4}, [
  text("Revenue", {"fg": "text.default", "weight": "bold"}),
  chart_line("line", points, 320, 120)      # draws in series.1
])
```

## Gradients

`bg` — and only `bg` — may be a hash naming a linear gradient of two or three
stops, drawn as CSS draws `linear-gradient()`: under the node's corners,
border, shadow and opacity like any fill, its stops mixed in sRGB.

```soli
banner = {"bg": {"gradient": {"to": "right", "stops": ["accent.base", ["#ff80b5", 255]]}}, "pad": 6, "radius": 3}
tilted = {"bg": {"gradient": {"angle": 45, "stops": [ ["accent.base", 26], ["info.base", 128], "danger.base"]}}}
```

| Key | Values |
|-----|--------|
| `to` | `"top"` `"right"` `"bottom"` `"left"`, or a corner — `"top right"` … — whose direction is the box's own diagonal, as in CSS. The default is `"bottom"` |
| `angle` | Whole degrees, `0` to `359`, clockwise from the top: `90` is `"right"`. Give `to` or `angle`, not both |
| `stops` | Two or three: a colour (a role or `"#RRGGBB"`, never `"none"`), or `[colour, at]` with `at` in 255ths of the way along. Stops without a position are spread evenly, and positions go forwards |

Stops that are roles follow the viewer's dark mode like any role. Each
distinct gradient is defined once per session, like a literal colour. A
gradient's colours do not ease: a `transition` into or out of one changes
them at once and fades only the opacity. A gradient is EUI protocol **6**; a
client older than that is sent the first stop as a solid `bg`. (A list of
pairs wants a space after its opening bracket — `[ ["accent.base", 26], …]` —
because `[[` opens a raw string.)

## Transitions

`transition` names how long a node takes to settle into a change of style, in
the client's own scale rather than milliseconds — the viewer's reduced-motion
setting is the client's to honour:

| Value | For |
|-------|-----|
| `"none"` | The default: the change is immediate. |
| `"fast"` | A hover or a press — an answer to something the viewer just did. |
| `"base"` | A panel opening, a row highlighting. |
| `"slow"` | A change the viewer did not ask for and should notice. |
| `"slower"` `"slowest"` | Ambient movement — a level meter settling, a background easing between states. |

```soli
box({"bg": lit ? "accent.base" : "surface.sunken", "transition": "fast"}, [])
```

## Arriving and leaving

`transition` is a duration and never a direction. What a node does when it is
*grafted* or *released* is `animation`, which is a list, and `motion`, which
says which way:

| Key | Values |
|-----|--------|
| `animation` | `"spin"`, `"enter"`, `"exit"`, `"pulse"`, `"bounce"` — a list, so `["enter", "exit"]` is the ordinary spelling of a page |
| `motion` | `"fade"` `"leading"` `"trailing"` `"top"` `"bottom"` `"scale"` `"paired"` |

Only the arriving side names a direction. Whatever is leaving beside it takes
the mirror — `leading` against `trailing`, `top` against `bottom` — so a push
and a pop are one sentence read in the two directions, and "which way is
back" is never asked. A `motion` with neither an entrance nor an exit is
refused, because it is a direction with nothing to direct.

```soli
nav_page("detail", customer_page(state), {"motion": "trailing"})
```

`"pulse"` and `"bounce"` are Tailwind's `animate-pulse` and `animate-bounce`
(EUI protocol 6): the node and everything in it dips to half opacity and back
every two seconds, or is lifted a quarter of its own height and let fall once
a second. Both run on the client's clock like `"spin"` — nothing crosses the
wire while they play, and the node is laid out and pressed where it rests.
A client older than protocol 6 is sent the style without them.

```soli
column({"gap": 2, "animation": "pulse"}, skeleton_lines)          # loading
box({"radius": 4, "bg": "accent.base", "animation": "bounce"}, [icon("arrow_down", {})])
```

### A shared element

`"paired"` is not a direction: it is one thing on two pages. Put it, with the
**same** `key`, on the node that is leaving and on the node taking its place,
and the arriving one flies out of the box its partner had — a row's avatar
becoming a header's avatar, a thumbnail becoming a hero. Both ends are boxes
the client already laid out, so nothing is laid out again for it.

```soli
# In the list, on every row: which row is about to be the one is not
# known until it is tapped.
shared_element("cust:" + one["id"], initial_avatar(one["initial"], one["tone"], 24))

# And in the detail, under the same name, half again as large.
shared_element("cust:" + one["id"], initial_avatar(one["initial"], one["tone"], 36))
```

A name that resolves to nothing is the ordinary case and not an error — a
panel is built and torn down as it opens — so the node simply takes the
motion of the page it is on. Which is also the one way to get this wrong
silently: a name spelt two ways is a page where nothing moves and nothing
complains. Run the client with `EUI_TRACE=1` and it prints a line per pair,
resolved or not, and says why.

## Placement

`position` decides how a node sits in its parent:

| Value | Meaning |
|-------|---------|
| `"flow"` | The default — laid out in the parent's flow. |
| `"stack"` | Positioned within a `stack` parent, so siblings overlap. |
| `"pointer"` | Placed where the pointer is, for a context menu or a tooltip that follows the cursor. |

```soli
stack({}, [
  chart_area("spend", points, 320, 120),
  box({"position": "pointer", "bg": "surface.overlay", "pad": 2}, [text(hover_label, {})])
])
```

## Tailwind classes

The scaffolded catalogue has a sixth file, `eui_builders_tw.sl`, and in it
`tw("...")`: a style written in Tailwind's classes. It is plain Soli and adds
nothing to the wire — every class becomes one of the keys above, a colour
becomes a role, a spacing step becomes an index of the space scale.

```soli
row({"tw": "items-center gap-4 px-4 py-4 border-b border-gray-200 hover:bg-gray-50"}, [
  text(person["name"], tw_style("text-sm font-semibold text-gray-900")),
  text(person["mail"], tw_style("text-xs text-gray-500 truncate"))
])
```

`tw(classes, width = nil)` returns `{"s", "hover", "press", "focus", "disabled",
"props", "gaps", "divide"}`: the resting style and the four states as deltas
over it. A `"tw"` key in any
style given to `node`, `column`, `row` or `stack` is read the same way, and the
states become local handlers, so a hover costs no round trip; `control({"tw":
...})` and `stateful(base, "hover:...", on)` take classes too. `tw_style` is
the resting style alone, for a `text` node.

| Tailwind | EUI |
|----------|-----|
| `p-4`, `px-2`, `gap-3`, `py-1.5` | `pad` / `margin` / `gap` space indices: 0, 0.5, 1, 2, 3, 4, 5, 6, 8, 10, 12, 16, 24 are indices 0 – 12, and 1.5, 2.5, 3.5, 20, 32 are 13 – 17 |
| `w-64`, `w-1/2`, `w-full`, `max-w-md` | px (N × 4), percent, `"100%"`, Tailwind's max widths |
| `text-sm` … `text-4xl`, `font-semibold` | `size` 0 – 7, `weight` |
| `bg-white`, `bg-gray-50`, `bg-gray-100` | `surface.raised`, `surface.base`, `surface.sunken` |
| `text-gray-900`, `text-gray-500`, `text-gray-400` | `text.default`, `text.muted`, `text.disabled` |
| `border-gray-200` / `300` / `400` | `border.subtle` / `default` / `strong` |
| `bg-indigo-600` / `500` / `700` | `accent.base` / `hover` / `active` |
| `red`, `green`, `yellow`, `blue` `-50` / `-600` | `danger`, `success`, `warning`, `info` `.subtle` / `.base` |
| `ring-1 ring-gray-300` | a 1 px border in `border.default` |
| `rounded-md`, `rounded-xl`, `rounded-full` | `radius` 2, 3, 4 |
| `shadow-sm`, `shadow-md`, `shadow-lg` | `shadow` 1, 2, 3 |
| `hover:`, `active:`, `focus:`, `disabled:` | local states; `focus-visible:` is `focus:`, and its `outline-*`/`ring-*` are the client's own keyboard ring |
| `sm:` `md:` `lg:` `xl:` `2xl:` | resolved on the server against the viewport width you pass, mobile first |
| `space-x-4` on a row, `space-y-2` on a column, `gap-x-4` / `gap-y-2` | `gap`, where the two are the same thing |
| `divide-y divide-gray-200`, `divide-x` | a border on every child but the first, laid on by `node()` |
| `uppercase`, `lowercase`, `capitalize` | the string, transformed by `text()` |
| `mx-auto` · `my-auto` · `block` · `relative`, `static` | `self: center` · `self: center` · `display: column` · `position: flow` |
| `bg-gradient-to-r` … `-tl`, `from-*`, `via-*`, `to-*`, `from-10%` … | one `bg` gradient, in any class order; `to-*` is required, since a role has no transparent copy to fade to |
| `animate-spin`, `animate-pulse`, `animate-bounce` | `animation`, and two of them are both: `["spin", "pulse"]` |

Breakpoints need the viewport's width, which the view is given on `connect`
and on every resize and `tw()` is not — so pass it, and a class under a
breakpoint without one raises rather than guessing:

```soli
vw = state["viewport"]["width"]
node("box", {"tw": "flex flex-col gap-4 sm:flex-row sm:items-end", "vw": vw}, [title, actions])
text(label, tw_style("text-sm md:text-base", false, vw))
tw("hidden lg:flex", vw)
```

`tw(classes)` with one argument is unchanged for everything else. A string is
memoised once per breakpoint its width falls in, not per width.

The half steps `1.5`, `2.5` and `3.5`, and `20` and `32`, are space indices
13 – 17, which EUI protocol version 6 added. You write them the same way for
every client: a session with a client older than 6 is sent the nearest step
it has instead, rounding down on a tie — `py-1.5` draws as `py-1` (4 px),
`px-2.5` as `px-2`, `gap-3.5` as `gap-3`, `20` as `16` and `32` as `24`. The
same goes for a raw `{"pad": 13}`.

A class with no equivalent raises, naming the class and the reason, rather
than being dropped: `tracking-*`, `leading-*`, a gradient with no `from-*` or
`to-*`, per-corner radius,
transforms, `italic`, a step the space scale does not have such as `p-7`
(the message names the two nearest), a `space-y` on a row or a gap-x and gap-y that differ where both
axes are spaced, and `dark:` (roles already follow the theme). The whole
table, the approximations and every refusal are in the EUI repository's
`doc/docs/eui/tailwind.md`.

The rest of the catalogue is drawn the way Tailwind UI draws an application:
14 px labels and body text, white fields and secondary buttons inside a
`border.default` hairline, cards at radius 2 with a small shadow, dialogs and
menus a step up.
