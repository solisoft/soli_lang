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
| `animation` | `"spin"`, `"enter"`, `"exit"` — a list, so `["enter", "exit"]` is the ordinary spelling of a page |
| `motion` | `"fade"` `"leading"` `"trailing"` `"top"` `"bottom"` `"scale"` `"paired"` |

Only the arriving side names a direction. Whatever is leaving beside it takes
the mirror — `leading` against `trailing`, `top` against `bottom` — so a push
and a pop are one sentence read in the two directions, and "which way is
back" is never asked. A `motion` with neither an entrance nor an exit is
refused, because it is a direction with nothing to direct.

```soli
nav_page("detail", customer_page(state), {"motion": "trailing"})
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

`tw(classes)` returns `{"s", "hover", "press", "focus", "disabled", "props"}`:
the resting style and the four states as deltas over it. A `"tw"` key in any
style given to `node`, `column`, `row` or `stack` is read the same way, and the
states become local handlers, so a hover costs no round trip; `control({"tw":
...})` and `stateful(base, "hover:...", on)` take classes too. `tw_style` is
the resting style alone, for a `text` node.

| Tailwind | EUI |
|----------|-----|
| `p-4`, `px-2`, `gap-3` | `pad` / `margin` / `gap` space indices: 0, 0.5, 1, 2, 3, 4, 5, 6, 8, 10, 12, 16, 24 are indices 0 – 12 |
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
| `hover:`, `active:`, `focus:`, `disabled:` | local states |

A class with no equivalent raises, naming the class and the reason, rather
than being dropped: `tracking-*`, `leading-*`, gradients, per-corner radius,
`divide-*` and `space-*`, transforms, breakpoints (`md:` — branch on the
viewport instead) and `dark:` (roles already follow the theme). The whole
table, the approximations and every refusal are in the EUI repository's
`doc/docs/eui/tailwind.md`.

The rest of the catalogue is drawn the way Tailwind UI draws an application:
14 px labels and body text, white fields and secondary buttons inside a
`border.default` hairline, cards at radius 2 with a small shadow, dialogs and
menus a step up.
