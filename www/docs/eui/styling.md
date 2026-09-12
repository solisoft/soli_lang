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
