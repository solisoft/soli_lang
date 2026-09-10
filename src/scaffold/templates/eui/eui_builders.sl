# EUI view builders. Each returns a plain hash; nothing here is native.
# Lives in app/controllers/ so it loads with the handlers (one namespace).
#
# This is the reference catalogue spec/03-widgets.md §4 names. It is the
# copy `soli new <app> --eui` writes, vendored in the language repository
# as `src/scaffold/templates/eui/eui_builders.sl`; the two are kept byte
# for byte identical by `scripts/sync-catalogue.sh`.
#
#   {"k": kind, "s": style, "t": text, "c": children, "on": handlers, "key": key, "p": props}
#
# Style keys are the spec's vocabulary: display, gap, pad, margin, bg, fg,
# border, border_color, radius, size, weight, width, height, align, justify,
# wrap, grow, cursor… Colours are role names ("accent.base") or "#RRGGBB".

def node(kind, style, children)
  {
    "k": kind,
    "s": style,
    "c": children
  }
end

def column(style, children)
  style["display"] = "column"
  node("box", style, children)
end

def row(style, children)
  style["display"] = "row"
  node("box", style, children)
end

def stack(style, children)
  style["display"] = "stack"
  node("box", style, children)
end

def text(content, style)
  {
    "k": "text",
    "t": content,
    "s": style
  }
end

def spacer
  {"k": "spacer", "s": {"grow": 1}}
end

# A named vector icon, stroked by the client from its own table. The name is a
# prop, not text: an icon is not a character, so it takes `fg` like a label but
# is never shaped, never falls back to a symbols face, and never reaches a
# screen reader as the glyph it happens to resemble. With no size of its own it
# takes a square from the text beside it.
def icon(name, style)
  {
    "k": "icon",
    "s": style ?? {},
    "p": {"name": name}
  }
end

def divider
  {"k": "divider"}
end

def scroll(style, children)
  style["display"] = "column"
  node("scroll", style, children)
end

# A virtualised list: `item_height` lets the client skip rows it cannot see.
def list(style, item_height, children)
  style["display"] = "column"
  {
    "k": "list",
    "s": style,
    "c": children,
    "p": {"item_height": item_height}
  }
end

# A windowed list (spec 04 §7.1): `count` rows of which only `children`
# are present, each carrying its `row`; `heights` gives every row's height
# so the scroll extent is exact. `on_window` receives `[first, last]`
# when the rows in view change.
def list_window(style, item_height, count, heights, children, on_window)
  style["display"] = "column"
  {
    "k": "list",
    "s": style,
    "c": children,
    "p": {
      "item_height": item_height,
      "count": count,
      "heights": heights
    },
    "on": {"window": on_window}
  }
end

# A sound (EUI spec 03 §7). It draws nothing; it plays. `src` is a file in
# the application, hashed and served like a picture. `props` may carry
# "playing", "volume" (0..100), "loop" and "position" (ms — the client
# seeks when the number changes), and `on` may carry "ended" and
# "time_update" handlers.
def audio(src, props, on)
  # `sound`, not `node`: a bare assignment to a builder's name rebinds it.
  sound = {"k": "audio", "p": props.merge({"src": src})}
  sound["on"] = on unless on.nil?
  sound
end

# A moving picture (EUI spec 03 §8). It sizes itself to its frames unless
# a style says otherwise. `props` may carry "playing", "loop" and
# "position" (ms), and `on` may carry "ended". GIF and animated WebP: the
# client decodes them in Rust, in its sandboxed worker.
def video(src, props, style, on)
  picture = {
    "k": "video",
    "s": style ?? {},
    "p": props.merge({"src": src})
  }
  picture["on"] = on unless on.nil?
  picture
end

def input(value, on_change)
  {
    "k": "input",
    "t": value,
    "s": {
      "pad": [2, 3, 2, 3],
      "border": 1,
      "border_color": "border.default",
      "radius": 2
    },
    "on": {"change": on_change}
  }
end

# The primary button: accent roles, so it follows the viewer into dark mode
# without the server knowing.
def button(label, on_click)
  button_variant(label, on_click, "accent.base", "accent.on")
end

def keyed(key, n)
  n["key"] = key
  n
end

# Viewport breakpoints, same rungs as Tailwind: the view branches on these
# because the client has no media-query engine. `width` is
# `state["viewport"]["width"]`, sent on connect and every resize.
BP = {
  "xs": 0,
  "sm": 640,
  "md": 768,
  "lg": 1024,
  "xl": 1280,
  "2xl": 1536
}

# The spacing scale, 05 §2, in px at cozy density — the same numbers the
# client resolves `pad`, `gap` and `margin` against. A view needs them
# when it has to predict a box's width instead of being told: a canvas is
# drawn at a size the server chooses, so it can only fill its parent if
# the server can work out what the parent will give it.
SPACE = [0, 2, 4, 8, 12, 16, 20, 24, 32, 40, 48, 64, 96]

def space_px(ix, density)
  factor = 1.0
  factor = 0.8 if density == "compact"
  factor = 1.25 if density == "comfortable"
  int((SPACE[ix] * factor).round())
end

def bp_px(name)
  BP[name] ?? 0
end

def bp(width)
  return "2xl" if width >= BP["2xl"]
  return "xl" if width >= BP["xl"]
  return "lg" if width >= BP["lg"]
  return "md" if width >= BP["md"]
  return "sm" if width >= BP["sm"]

  "xs"
end

def bp_min(width, name)
  width >= bp_px(name)
end

# ---------------------------------------------------------------- catalogue
# Composed widgets. Every one is a plain function over the primitives above;
# state and events belong to the handler, so a checkbox carries the id of what
# it toggles as a prop, and the server reads it back from params["props"].

def h1(content)
  text(
    content,
    {"size": 5, "weight": "bold"}
  )
end

def h2(content)
  text(
    content,
    {"size": 4, "weight": "semibold"}
  )
end

def muted(content)
  text(
    content,
    {"fg": "text.muted", "size": 1}
  )
end

# Narrowing a widget after it is built only reaches its resting style: the
# hover and pressed styles are declared inside the local handlers, where a
# `n["s"]["pad"] = …` cannot see them. The pointer then snaps the node back to
# the catalogue's geometry, which is the same layout shift as a border that
# only exists on hover, several times larger. This applies one patch to every
# style a node declares, so all of its states keep the same shape.
def restyle(n, patch)
  n["s"] = n["s"].merge(patch)
  handlers = n["on"]
  return n if handlers.nil?

  names = ["pointer_enter", "pointer_leave", "pointer_down", "pointer_up"]
  for name in names
    handler = handlers[name]
    unless handler.nil?
      styles = handler["styles"] ?? {}
      for key in styles.keys()
        styles[key] = styles[key].merge(patch)
      end
      handler["styles"] = styles
      handlers[name] = handler
    end
  end
  n["on"] = handlers
  n
end


# ------------------------------------------------------------- control base
# Every interactive widget below is `control` plus a body. It exists because
# twenty-four widgets in this file answered the pointer with `cursor: pointer`
# and nothing else, and because not one of them could be disabled: the word
# did not appear in two and a half thousand lines.
#
# A tone is a resting colour set and the two deltas the pointer applies to it.
# Deltas, not whole styles — merged over whatever base the caller ended up
# with, they keep every geometry choice inside all three states, so a size or
# a selection or the caller's own patch cannot go missing under the pointer.
# That is `restyle`'s invariant made structural instead of repaired
# afterwards; `restyle` stays, for narrowing a widget from outside.
#
# `selected` folds into the *resting* colours before the hover delta is taken,
# which is why hovering an already-selected row does not look broken.
TONES = {
  "accent": {
    "bg": "accent.base", "fg": "accent.on", "border_color": "none",
    "hover": {"bg": "accent.hover", "border_color": "border.strong"},
    "press": {"bg": "accent.active"},
    "selected": {}
  },
  "neutral": {
    "bg": "surface.sunken", "fg": "text.default", "border_color": "border.default",
    "hover": {"bg": "surface.raised", "border_color": "border.strong"},
    "press": {"bg": "surface.sunken"},
    "selected": {"bg": "info.subtle"}
  },
  "ghost": {
    "bg": "none", "fg": "accent.base", "border_color": "none",
    "hover": {"bg": "surface.sunken"},
    "press": {"bg": "surface.sunken", "fg": "accent.active"},
    "selected": {"bg": "surface.sunken"}
  },
  # The theme gives `accent` a hover and an active offset and gives the status
  # roles neither — there is no `danger.active` to reach for. So danger presses
  # to a sunken surface and keeps its own colour in the label, which is the
  # precedent `button_variant` already set, and `danger.base` carries a
  # guaranteed 3:1 against a surface.
  "danger": {
    "bg": "danger.base", "fg": "danger.on", "border_color": "none",
    "hover": {"border_color": "border.strong"},
    "press": {"bg": "surface.sunken", "fg": "danger.base"},
    "selected": {}
  },
  "quiet": {
    "bg": "none", "fg": "text.default", "border_color": "none",
    "hover": {"bg": "surface.sunken"},
    "press": {"bg": "surface.sunken"},
    "selected": {"bg": "surface.sunken"}
  }
}

# Disabled is one patch over the resting style, and there are no other styles
# left once the handlers are gone. `text.disabled` and `border.subtle` carry
# their own contrast guarantee (05 §4), so this stays legible in every mode.
DISABLED = {
  "fg": "text.disabled",
  "bg": "none",
  "border_color": "border.subtle",
  "cursor": "not_allowed",
  "transition": "none"
}

# What a size decides. `pad` and `gap` are space *indices*, not pixels: the
# client multiplies them by the viewer's density (05 §5), so a server that
# resolved them here would scale them twice. A data-dense application asks for
# `size: "sm"`; it does not get its own density, because density is the
# viewer's to choose and not the application's.
#
# There is deliberately no height. Height is the text line plus the padding,
# which the client scales on both axes for free; a height pinned in pixels
# clips its own label the moment the viewer raises their font scale.
SIZES = {
  "sm": {"text": 1, "pad": [1, 3, 1, 3], "gap": 2, "min_width": 32, "icon": 24, "mark": 16},
  "md": {"text": 2, "pad": [2, 4, 2, 4], "gap": 3, "min_width": 44, "icon": 28, "mark": 18},
  "lg": {"text": 3, "pad": [3, 5, 3, 5], "gap": 3, "min_width": 56, "icon": 36, "mark": 22}
}

# The `control` scale of 05 §2, in px at cozy density. Only for the places
# where a fixed box is the point — an icon button, a day cell, a row height —
# never for anything that holds a label.
CONTROL = [28, 36, 44]

def size_spec(size)
  SIZES[size] ?? SIZES["md"]
end

# Box geometry only. The text scale is not here: `size` styles a text node,
# and only `fg` inherits, so a label takes its size from `control_text_size`.
def control_metrics(size)
  m = size_spec(size)
  {"pad": m["pad"], "gap": m["gap"], "min_width": m["min_width"]}
end

def control_text_size(size)
  size_spec(size)["text"]
end

def icon_box_px(size)
  size_spec(size)["icon"]
end

def checkbox_box_px(size)
  size_spec(size)["mark"]
end

def control_px(size, density)
  ix = size == "sm" ? 0 : (size == "lg" ? 2 : 1)
  factor = 1.0
  factor = 0.8 if density == "compact"
  factor = 1.25 if density == "comfortable"
  int((CONTROL[ix] * factor).round())
end

def tone_resting(tone, lit)
  base = {"bg": tone["bg"], "fg": tone["fg"], "border_color": tone["border_color"]}
  return base unless lit

  base.merge(tone["selected"] ?? {})
end

# The semantics a widget declares about itself. Props are a generic bag the
# client already reads by name, so these cost no wire change and an older
# client ignores them; what they buy is a checkbox that reaches a screen
# reader as a checkbox rather than as a button named by its label.
def a11y_props(o)
  props = (o["props"] ?? {}).merge({})
  semantics = o["a11y"] ?? {}
  for name in semantics.keys()
    value = semantics[name]
    props[name] = value unless value.nil?
  end
  props["disabled"] = true if o["disabled"] == true
  props["busy"] = true if o["loading"] == true
  props["read_only"] = true if o["read_only"] == true
  props
end

# Wire the four pointer events onto an already-final resting style. `base`
# must be the style the node actually carries — the deltas are merged over
# it, so whatever the caller changed is already inside every state.
def stateful(base, tone, on)
  hover = base.merge(tone["hover"] ?? {})
  active = base.merge(tone["press"] ?? {})
  on.merge({
    "pointer_enter": {"local": "self.style = @hover", "styles": {"hover": hover}},
    "pointer_leave": {"local": "self.style = @base", "styles": {"base": base}},
    "pointer_down": {"local": "self.style = @active", "styles": {"active": active}},
    "pointer_up": {"local": "self.style = @hover", "styles": {"hover": hover}}
  })
end

# One options hash, so a widget gains a capability without its callers
# changing:
#
#   key       required and unique. 07 §3 names a node by key and the arena's
#             key map is flat, so a duplicate silently restyles someone else.
#   kind      "box" by default.
#   tone      a TONES name; "quiet" by default.
#   size      "sm" | "md" | "lg"; "md" by default.
#   shape     extra resting style — radius, width, justify. The caller wins.
#   on        the handler map, usually just {"click": event}.
#   props     the identity the handler reads back (03 §4).
#   a11y      what this control *is*, per a11y_props above.
#   selected / checked / expanded   server state, folded into the resting look.
#   disabled / read_only / loading  the terminal states.
#
# Precedence: disabled > loading > read_only > active > hover > selected >
# resting. The first two are terminal — they replace the resting style and
# take the handlers with them, so hover and pressed cannot be reached.
#
# Dropping the handler map is the whole of what `disabled` means, and it is
# right in three places at once: no click reaches the server, because dispatch
# finds nothing on the path; the node leaves the Tab order, because focus
# order is exactly the nodes holding those handlers; and the cursor and the
# colours come from DISABLED. It is wrong in a fourth, which is why `disabled`
# is also a prop: with no click handler the client's accessibility mapping
# sees no button, and a disabled control would decay into an unnamed group.
def control(o)
  key = o["key"]
  throw "control: every control needs a unique key" if key.nil?

  size = o["size"] ?? "md"
  tone = TONES[o["tone"] ?? "quiet"] ?? TONES["quiet"]
  disabled = o["disabled"] == true
  loading = o["loading"] == true
  inert = disabled || loading
  lit = o["selected"] == true || o["checked"] == true

  base = control_metrics(size).merge({
    "display": "row",
    "align": "center",
    "justify": "center",
    "radius": 2,
    "border": 1,
    "border_color": "none",
    "transition": "fast"
  })
  base = base.merge(tone_resting(tone, lit))
  base = base.merge(o["shape"] ?? {})
  base = base.merge(DISABLED) if disabled
  base = base.merge({"cursor": "wait"}) if loading && !disabled
  base["cursor"] = base["cursor"] ?? "pointer"

  n = {
    "k": o["kind"] ?? "box",
    "key": key,
    "s": base,
    "c": o["c"] ?? []
  }
  props = a11y_props(o)
  n["p"] = props if props.keys().length() > 0
  n["on"] = stateful(base, tone, o["on"] ?? {}) unless inert
  n
end
# A button: a box with a click handler, and hover/pressed states that run
# locally — the client repoints the node at a declared style on pointer
# enter/down/up/leave, so feedback never waits for the network. The node is
# keyed so the local handler can name it (`self`).
#
# The border is in the resting style, uncoloured. A border in EUI is part of
# the box — layout adds its widths to the frame — so a border that appears on
# hover makes the button 2 px wider and taller and shoves its whole row
# sideways under the pointer. Reserving the width at rest and colouring it on
# hover leaves nothing for layout to do: what changes is a colour, which 03 §5
# animates on its own. Same rule as `chip_remove` below.
def button_variant(label, on_click, bg, fg)
  base = {
    "display": "row",
    "justify": "center",
    "align": "center",
    "pad": [2, 4, 2, 4],
    "min_width": 44,
    "bg": bg,
    "fg": fg,
    "border": 1,
    "radius": 2,
    "cursor": "pointer",
    "transition": "fast"
  }
  hover = base.merge({"bg": bg == "accent.base" ? "accent.hover" : bg, "border_color": "border.strong"})
  active = base.merge({"bg": bg == "accent.base" ? "accent.active" : "surface.sunken"})
  {
    "k": "box",
    "key": "btn:" + on_click + ":" + label,
    "s": base,
    "on": {
      "click": on_click,
      "pointer_enter": {"local": "self.style = @hover", "styles": {"hover": hover}},
      "pointer_leave": {"local": "self.style = @base", "styles": {"base": base}},
      "pointer_down": {"local": "self.style = @active", "styles": {"active": active}},
      "pointer_up": {"local": "self.style = @hover", "styles": {"hover": hover}}
    },
    "c": [text(label, {"weight": "semibold"})]
  }
end

def secondary_button(label, on_click)
  button_variant(label, on_click, "surface.sunken", "text.default")
end

def danger_button(label, on_click)
  button_variant(label, on_click, "danger.base", "danger.on")
end

def ghost_button(label, on_click)
  button_variant(label, on_click, "none", "accent.base")
end

# A checkbox is a small box whose fill says its state, plus a label. `props`
# travel back with the click so the handler knows which item it was.
#
# The row is the hit target and the mark is a child of it: a mark that changed
# size under the pointer would shove its own label sideways, so the pointer
# washes the row and leaves the mark alone. `checked` is not handed to
# `control` as selection — the mark already says the state, and a second
# background saying it as well reads as a bug.
def checkbox(label, checked, on_toggle, props, o = {})
  size = o["size"] ?? "md"
  box = checkbox_box_px(size)
  disabled = o["disabled"] == true
  mixed = o["indeterminate"] == true
  lit = checked || mixed
  mark = {
    "k": "box",
    "s": {
      "width": box,
      "height": box,
      "radius": 1,
      "border": 2,
      "border_color": disabled ? "border.subtle" : (lit ? "accent.base" : "border.strong"),
      "bg": (lit && !disabled) ? "accent.base" : "none",
      "display": "row",
      "justify": "center",
      "align": "center",
      "transition": "fast"
    },
    "c": lit ? [icon(
      mixed ? "minus" : "check",
      {
        "fg": disabled ? "text.disabled" : "accent.on",
        "width": box - 6,
        "height": box - 6
      }
    )] : []
  }
  control({
    "key": o["key"] ?? ("cb:" + (props["id"] ?? label).to_s),
    "size": size,
    "shape": {"justify": "start", "border": 0, "radius": 1, "min_width": 0, "pad": [1, 2, 1, 2]},
    "on": {"click": on_toggle},
    "props": props,
    "disabled": disabled,
    "a11y": {
      "role": "check_box",
      "checked": mixed ? "mixed" : checked,
      "label": o["name"] ?? label
    },
    "c": [
      mark,
      text(label, {
        "size": control_text_size(size),
        "fg": disabled ? "text.disabled" : (checked ? "text.muted" : "text.default")
      })
    ]
  })
end

# A switch is a track the knob slides along. Both track and knob carry a
# transition, so the colour change eases; the knob's travel is a layout
# change and does not animate, which 03 §5 is explicit about.
def switch(label, on, on_toggle, props, o = {})
  size = o["size"] ?? "md"
  disabled = o["disabled"] == true
  knob_px = checkbox_box_px(size) - 2
  knob = {"k": "box", "s": {
    "width": knob_px,
    "height": knob_px,
    "radius": 4,
    "bg": (on && !disabled) ? "accent.on" : "surface.raised",
    "self": on ? "end" : "start",
    "transition": "fast"
  }}
  track = {
    "k": "box",
    "s": {
      "display": "row",
      "width": knob_px * 2 + 4,
      "height": knob_px + 4,
      "radius": 4,
      "pad": 1,
      "bg": disabled ? "border.subtle" : (on ? "accent.base" : "border.strong"),
      "justify": on ? "end" : "start",
      "align": "center",
      "transition": "fast"
    },
    "c": [knob]
  }
  control({
    "key": o["key"] ?? ("sw:" + (props["id"] ?? label).to_s),
    "size": size,
    "shape": {"justify": "start", "border": 0, "radius": 1, "min_width": 0, "pad": [1, 2, 1, 2]},
    "on": {"click": on_toggle},
    "props": props,
    "disabled": disabled,
    "a11y": {
      "role": "switch",
      "checked": on,
      "label": o["name"] ?? label
    },
    "c": [
      track,
      text(label, {"size": control_text_size(size), "fg": disabled ? "text.disabled" : "text.default"})
    ]
  })
end

def badge(label, tone)
  {
    "k": "box",
    "s": {
      "display": "row",
      "pad": [0, 2, 0, 2],
      "radius": 4,
      "bg": tone + ".subtle",
      "shrink": 0
    },
    "c": [text(
      label,
      {
        "fg": tone + ".base",
        "size": 0,
        "weight": "semibold"
      }
    )]
  }
end

def card(style, children)
  style["bg"] = "surface.raised"
  style["radius"] = 3
  style["border"] = 1
  style["border_color"] = "border.subtle"
  style["shadow"] = style["shadow"] ?? 1
  style["pad"] = style["pad"] ?? 5
  style["display"] = "column"
  node("box", style, children)
end

# Tabs: a row of labels, the active one underlined in accent. `props` carry
# the tab name so one handler serves every tab.
#
# The active tab is said by its underline alone — it is not
# handed to `control` as selection, because a background behind the active
# tab as well would say the same thing twice. The strip is keyed and carries
# its role, so the set reaches an assistive technology as a set and each tab
# knows its place in it.
def tabs(names, active, on_select, o = {})
  size = o["size"] ?? "md"
  off_list = o["disabled"] ?? []
  count = names.length()
  cells = range(0, count).map(fn(i) {
    name = names[i]
    is_active = name == active
    off = off_list.includes?(name)
    control({
      "key": (o["key"] ?? "tabs") + ":" + name,
      "size": size,
      "shape": {
        "pad": [2, 1, 2, 1],
        "min_width": 0,
        "radius": 0,
        "border": [0, 0, 2, 0],
        "border_color": is_active ? "accent.base" : "none"
      },
      "on": {"click": on_select},
      "props": {"tab": name},
      "disabled": off,
      "a11y": {
        "role": "tab",
        "selected": is_active,
        "label": name,
        "pos_in_set": i + 1,
        "set_size": count
      },
      "c": [text(name, {
        "size": control_text_size(size),
        "weight": is_active ? "semibold" : "regular",
        "fg": off ? "text.disabled" : (is_active ? "text.default" : "text.muted")
      })]
    })
  })
  strip = row({"gap": 5, "border": [0, 0, 1, 0], "border_color": "border.subtle"}, cells)
  strip["key"] = o["key"] ?? "tabs"
  strip["p"] = {"role": "tab_list", "orientation": "horizontal"}
  strip
end

# A spinner: a three-quarter arc on a canvas that the client spins.
def spinner
  spinner_sized(18)
end

def spinner_sized(size)
  half = size / 2
  {
    "k": "canvas",
    "s": {
      "width": size,
      "height": size,
      "animation": "spin",
      "shrink": 0
    },
    "p": {"paths": [ [
      4,
      "accent.base",
      2,
      half,
      half,
      half - 2,
      0,
      4.71
    ]]}
  }
end

# A button that shows it is working the instant it is pressed: a local
# handler reveals the spinner and changes the label, the server answers,
# and the client puts the button back the moment that answer arrives.
# A light/dark switch: the viewer's choice, made on the client by a local
# handler (`theme.toggle()`), so it costs no round trip and the server
# learns of it only as the next viewport. Not shown in the feed: a client
# that follows the desktop's theme has no use for it.
def theme_toggle
  {
    "k": "box",
    "s": {
      "display": "row",
      "align": "center",
      "justify": "center",
      "pad": [2, 3, 2, 3],
      "min_width": 36,
      "bg": "surface.sunken",
      "fg": "text.default",
      "radius": 2,
      "cursor": "pointer",
      "transition": "fast"
    },
    "on": {"click": {"local": "theme.toggle()"}},
    "c": [text("☀/☾", {"weight": "semibold"})]
  }
end

def loading_button(label, on_click, key)
  spin = spinner_sized(14)
  spin["s"]["display"] = "none"
  spin["key"] = key + "_spin"
  caption = text(label, {"weight": "semibold"})
  caption["key"] = key + "_label"
  showing = spinner_sized(14)["s"]
  showing["display"] = "row"
  {
    "k": "box",
    "s": {
      "display": "row",
      "gap": 2,
      "align": "center",
      "justify": "center",
      "pad": [2, 4, 2, 4],
      "min_width": 44,
      "bg": "surface.sunken",
      "fg": "text.default",
      "radius": 2,
      "cursor": "pointer",
      "transition": "fast"
    },
    "on": {"click": {
      "local": key + "_spin.style = @showing; " + key + "_label.text = \"Loading…\"",
      "styles": {"showing": showing},
      "then": on_click
    }},
    "c": [spin, caption]
  }
end

def toast(message, tone)
  {
    "k": "box",
    "p": {
      "role": "status",
      "live": tone == "danger" ? "assertive" : "polite"
    },
    "s": {
      "display": "row",
      "gap": 3,
      "pad": [3, 4, 3, 4],
      "radius": 2,
      "shadow": 2,
      "bg": tone + ".subtle",
      "border": 1,
      "border_color": tone + ".base"
    },
    "c": [text(
      message,
      {
        "fg": tone + ".base",
        "weight": "semibold"
      }
    )]
  }
end

# A dialog is a stack: a dimming overlay, then the panel, centred.
# A region of the tree that is built only when something asks for it.
#
# `make` is a thunk — `fn() { ... }` — and it is not called at all unless
# `id` is in `shown`. That is the whole mechanism: an unopened modal, an
# unselected tab and a collapsed section cost nothing to build, nothing to
# encode and nothing to send, because their content never exists.
#
# This is lazy *evaluation*, not a loading spinner. Every EUI frame answers
# a client event and a handler is synchronous, so the thunk runs during the
# very event that reveals the region and its content lands in that same
# frame — `placeholder` is what stands there while the region is closed,
# not something the viewer watches being replaced.
#
# Nothing is cached here on purpose. `lazy` cannot know what a subtree
# reads, and a cached panel that depends on state would serve stale nodes.
# A builder whose input really is fixed — a document at a path — memoises
# on that input itself.
def lazy(id, shown, placeholder, make)
  return placeholder unless (shown ?? []).includes?(id)

  make()
end

def dialog(title, body_children, actions, opts = {})
  panel = card(
    {
      "width": opts["width"] ?? 360,
      "height": opts["height"] ?? "auto",
      "gap": 4,
      "self": "center"
    },
    [h2(title)].concat(body_children, [row(
      {"gap": 2, "justify": "end"},
      actions
    )])
  )
  n = {
    "k": "overlay",
    "key": opts["key"] ?? ("dialog:" + title),
    "s": {
      "display": "stack",
      "justify": "center",
      "align": "center",
      "blur": 16,
      "bg": "#00000073",
      # 03 §5: an `enter` fades the scrim in and frosts the blur in with it,
      # from a radius of zero, over the `transition` duration. Without it the
      # page went from sharp to fully frosted between two frames, which reads
      # as a flash rather than as something arriving. `slow` is the longest
      # the motion scale offers -- 320 ms.
      "animation": "enter",
      "transition": opts["transition"] ?? "slow"
    },
    # `modal` keeps Tab inside the dialog, and puts it there when it opens:
    # a server cannot do either, because it does not own Tab. `keys` claims
    # Escape alone, so the buttons inside keep Enter as the press they stand
    # for — and Escape now reaches the server, which is what closes it.
    "p": {
      "role": opts["alert"] == true ? "alert_dialog" : "dialog",
      "label": title,
      "modal": true,
      "autofocus": true,
      "keys": ["Escape"]
    },
    "c": [panel]
  }
  n["on"] = {"key_down": opts["on_close"]} unless opts["on_close"].nil?
  n
end
# The page behind is put out of play twice over: blurred, so nothing
# on it is legible enough to invite a click, and dimmed, so the panel
# is plainly the brighter thing. The blur does most of the work, which
# is why the scrim is a third lighter than it was when it did all of
# it — the page should still read as present, just out of reach.

# An alert: one thing to say and nothing to decide, so one button. The
# handler fires on the button, not on the backdrop — a dialog that closes
# when the pointer slips is a dialog that loses what it was asking.
#
# `opts`: {"ok": "Got it"}
def alert(title, message, on_close, opts)
  opts = opts ?? {}
  dialog(title, [text(message, {})], [button(opts["ok"] ?? "OK", on_close)], opts.merge({
    "alert": true,
    "on_close": on_close
  }))
end

# A confirm: a question with two answers. The affirmative sits last, where
# the eye ends up, and wears `danger` when it destroys something — the
# button should say what it will do before the sentence above it is read.
#
# `opts`: {"ok": "Delete", "cancel": "Keep", "danger": true}
def confirm(title, message, on_confirm, on_cancel, opts)
  opts = opts ?? {}
  ok_label = opts["ok"] ?? "Confirm"
  destructive = opts["danger"] ?? false
  ok = destructive ? danger_button(ok_label, on_confirm) : button(ok_label, on_confirm)
  dialog(title, [text(message, {})], [secondary_button(opts["cancel"] ?? "Cancel", on_cancel), ok], opts.merge({
    "alert": true,
    "on_close": on_cancel
  }))
end

def field(label, value, on_change)
  column({"gap": 1}, [muted(label), input(value, on_change)])
end

def form(children, submit_label, on_submit)
  column({"gap": 4}, children.concat([row({"justify": "end"}, [button(submit_label, on_submit)])]))
end

# A table: header row plus keyed body rows; `columns` is a list of widths.

# --------------------------------------------------------------- split panes
# Two panels and a divider that can be dragged. `dir` is `"row"` for a
# vertical divider with the panels side by side, `"column"` for a horizontal
# one with them stacked.
#
# The drag is server-driven, and that is a smaller concession than it sounds.
# A press captures the pointer, so a move that leaves the divider still
# reaches it; moves are coalesced to one per frame rather than one per sample
# the mouse sends; and the payload of a `pointer_move` is measured against the
# node whose handler catches it — so the handlers sit on the container, and
# the number arriving at the server is already the position of the divider
# inside it, needing no arithmetic and no memory of where the drag began.
# What a local chunk would save is one round trip per frame, and a chunk
# cannot read its own event yet, so it could not do this at all.
#
# Where a panel's own size comes in: `a` and `b` are functions of one
# argument, the panel's extent along the split axis in pixels. A panel is
# built knowing how much room it has, which is what lets its content answer
# the panel instead of the window — `bp(px)` and `bp_min(px, "md")` take a
# width, and nothing about them says that width has to be the viewport's.

# The breakpoint rungs above are Tailwind's, and they are a *window's*. Handed
# a panel they say almost nothing: a pane of 309 px and one of 505 px are both
# "xs", so a view that branches on `bp` inside a split never branches at all.
# These are the same idea at the scale a panel actually lives at, and they are
# what a split's content should ask.
PANE = {
  "xs": 0,
  "sm": 200,
  "md": 320,
  "lg": 480,
  "xl": 720
}

def pane_px(name)
  PANE[name] ?? 0
end

def pane_bp(px)
  return "xl" if px >= PANE["xl"]
  return "lg" if px >= PANE["lg"]
  return "md" if px >= PANE["md"]
  return "sm" if px >= PANE["sm"]

  "xs"
end

def pane_min(px, name)
  px >= pane_px(name)
end


# The room the two panels share, once the divider has taken its own.
def split_span(extent, bar)
  span = extent - bar
  span < 0 ? 0 : span
end

# `fraction` is per mille — an integer, so it survives a round trip through
# state and a local handler's props without ever being a float. Both
# conversions round rather than truncate, which is what makes the trip exact:
# a divider dropped at a pixel and rebuilt from its fraction lands on the same
# pixel, where truncating at both ends lost one on the way.
def split_sizes(extent, fraction, min_a, min_b, bar)
  span = split_span(extent, bar)
  return [0, 0] if span <= 0

  a = int((span * fraction / 1000.0).round())
  room = span - min_b
  a = room if a > room
  a = min_a if a < min_a
  a = 0 if a < 0
  a = span if a > span
  [a, span - a]
end

# The pointer's position along the axis becomes the fraction the divider sits
# at. Clamped to both minimums, so a drag that runs past a panel's floor stops
# there rather than inverting the pair — and the clamp lives here, once,
# instead of in every application that draws a split.
def split_at(extent, at, min_a, min_b, bar)
  span = split_span(extent, bar)
  return 500 if span <= 0

  a = int(at) - int(bar / 2)
  room = span - min_b
  a = room if a > room
  a = min_a if a < min_a
  a = 0 if a < 0
  a = span if a > span
  int((a * 1000.0 / span).round())
end

def split_panel(build, px, across: Bool, cross)
  {
    "k": "box",
    "s": {
      "display": "column",
      "width": across ? px : cross,
      "height": across ? cross : px,
      "overflow": "clip"
    },
    "c": [build(px)]
  }
end

# The divider is not a `control`: it is a separator, its press has to reach
# the server as well as restyle locally, and `control` would put its own
# `pointer_down` over the top of that. It is keyed so the local chunk can
# name it, and it holds `key_down`, which is what puts it in the Tab order —
# so a split can be moved without a pointer at all.
def split_divider(key, across: Bool, bar, cross, fraction, on_drag, dragging: Bool, label)
  base = {
    "width": across ? bar : cross,
    "height": across ? cross : bar,
    "bg": dragging ? "accent.base" : "border.subtle",
    "cursor": across ? "resize_h" : "resize_v",
    "transition": "fast"
  }
  hot = base.merge({"bg": "accent.base"})
  {
    "k": "box",
    "key": key,
    "s": base,
    "p": {
      "role": "separator",
      "orientation": across ? "vertical" : "horizontal",
      "label": label ?? "Resize panels",
      "value_now": fraction,
      "value_min": 0,
      "value_max": 1000
    },
    "on": {
      "pointer_enter": {"local": "self.style = @hot", "styles": {"hot": hot}},
      "pointer_leave": {"local": "self.style = @base", "styles": {"base": base}},
      "pointer_down": {"local": "self.style = @hot", "styles": {"hot": hot}, "then": on_drag},
      "key_down": on_drag
    }
  }
end

#   key       required; the divider is keyed from it
#   dir       "row" | "column"
#   size      the container's extent along the split axis, in px
#   cross     the extent across it; "100%" if absent
#   fraction  per mille, 0..1000
#   min_a / min_b   the smallest each panel may become, in px
#   bar       the divider's thickness; 6 by default
#   on_drag   the event the divider and the container both send
#   dragging  true while a drag is in flight, so the divider stays lit
#   a / b     fn(px) -> node
def split_pane(o)
  key = o["key"]
  throw "split_pane: every split needs a key" if key.nil?

  dir = o["dir"] ?? "row"
  across = dir == "row"
  bar = o["bar"] ?? 6
  extent = o["size"] ?? 0
  cross = o["cross"] ?? "100%"
  fraction = o["fraction"] ?? 500
  min_a = o["min_a"] ?? 80
  min_b = o["min_b"] ?? 80
  on_drag = o["on_drag"]
  sizes = split_sizes(extent, fraction, min_a, min_b, bar)

  n = {
    "k": "box",
    "key": key,
    "s": {
      "display": across ? "row" : "column",
      "gap": 0,
      "align": "stretch",
      "width": across ? extent : cross,
      "height": across ? cross : extent,
      "overflow": "clip"
    },
    "c": [
      split_panel(o["a"], sizes[0], across, cross),
      split_divider(key + ":bar", across, bar, cross, fraction, on_drag, o["dragging"] == true, o["label"]),
      split_panel(o["b"], sizes[1], across, cross)
    ]
  }
  # The move and the release belong to the container, not to the divider: that
  # is what makes the payload container-relative, and what lets the pointer
  # leave the divider mid-drag without the drag ending.
  #
  # `drag_only` tells the client that this node wants the move only while a
  # button is held. The handler itself never leaves the tree -- taking it
  # off between drags looks equivalent and is not: an event already in
  # flight then names a handler the server has just removed, and every one
  # of them is refused. Measured here, that was 167 of 429 events dropped
  # and a drag that could not follow the hand.
  unless on_drag.nil?
    n["on"] = {"pointer_move": on_drag, "pointer_up": on_drag}
    n["p"] = (n["p"] ?? {}).merge({"drag_only": true})
  end
  n
end

# The four events a split sends, folded into a component's state. `name` is
# the state key holding the fraction; `name + "_drag"` holds whether a drag is
# in flight. An application writes one line in its handler and is done.
# One `pointer_move` while the divider is held: the point along the axis
# is where the divider now is, in thousandths.
def split_drag(state, params, name, dir, extent, min_a, min_b, bar)
  payload = params["payload"] ?? [0, 0]
  at = dir == "row" ? payload[0] : payload[1]
  state[name] = split_at(extent, at, min_a, min_b, bar)
  state
end

# The keyboard's half of the same divider: an arrow moves it by a step,
# Home puts it back in the middle, and the result is held inside the
# thousandths the fraction is measured in.
def split_keys(state, params, name, dir, step)
  payload = params["payload"] ?? [""]
  pressed_key = payload[0]
  back = dir == "row" ? "ArrowLeft" : "ArrowUp"
  fwd = dir == "row" ? "ArrowRight" : "ArrowDown"
  current = state[name] ?? 500
  state[name] = current - step if pressed_key == back
  state[name] = current + step if pressed_key == fwd
  state[name] = 500 if pressed_key == "Home"
  state[name] = 0 if state[name] < 0
  state[name] = 1000 if state[name] > 1000
  state
end

def split_event(state, params, name, dir, extent, min_a, min_b, bar)
  kind = params["kind"]
  drag = name + "_drag"
  step = 25

  if kind == "pointer_down"
    state[drag] = true
  elsif kind == "pointer_up"
    state[drag] = false
  elsif kind == "pointer_move" && (state[drag] ?? false)
    state = split_drag(state, params, name, dir, extent, min_a, min_b, bar)
  elsif kind == "key_down"
    state = split_keys(state, params, name, dir, step)
  end
  state
end

def table_header(labels, widths)
  cells = range(0, labels.length()).map(fn(i) {
    {
      "k": "text",
      "t": labels[i],
      "s": {
        "width": widths[i],
        "weight": "semibold",
        "size": 1,
        "fg": "text.muted"
      }
    }
  })
  row(
    {
      "gap": 4,
      "pad": [2, 3, 2, 3],
      "border": [0, 0, 1, 0],
      "border_color": "border.default"
    },
    cells
  )
end

def table_row(key, values, widths)
  cells = range(0, values.length()).map(fn(i) { {
    "k": "text",
    "t": values[i],
    "s": {"width": widths[i]}
  } })
  keyed(key, row(
    {
      "gap": 4,
      "pad": [1, 3, 1, 3],
      "border": [0, 0, 1, 0],
      "border_color": "border.subtle"
    },
    cells
  ))
end

# ---- Data grid -------------------------------------------------------------
# An editable, sortable grid. The header sits outside the list — sticky
# without sticky — and rows are keyed so a sort is MoveChild and a cell
# edit is set_text. `columns` are `{id, label, width, editable, options,
# align}`; `rows` are hashes keyed by those ids plus `id`. A column is
# editable unless `editable` is `false`. A non-empty `options` list makes
# the editor a compact select. `align` is `start`, `center` or `end`
# (default `start`). `selected` / `editing` / `sort` are `{row, col}`,
# `{row, col, open}`, `{col, dir}` — empty hashes when none. Rows are an
# equal-column `grid` (`1fr` each), so two columns are 50 %, three 33 %,
# filling the parent. The handler owns all three.

def grid_sort_rows(rows, col, dir)
  sorted = rows.sort_by(col)
  dir == "desc" ? sorted.reverse() : sorted
end

def grid_col_editable(col)
  return false if col["editable"] == false

  true
end

def grid_col_align(col)
  a = col["align"] ?? "start"
  return a if a == "center" || a == "end"

  "start"
end

def grid_cell(row_id, col, value, selected, editing, open, on_select, on_change, on_key)
  col_id = col["id"]
  props = {"row": row_id, "col": col_id}
  key = "cell:" + str(row_id) + ":" + col_id
  editable = grid_col_editable(col)
  align = grid_col_align(col)
  choices = col["options"] ?? []
  # The keyed node is always a box. Swapping it for an `input` is a kind
  # change, which replaces the node id — and the next event in the same
  # gesture (Enter's key_down, or change-then-submit) misses the tree.
  inner = text(
    value,
    {
      "size": 1,
      "clamp": 1,
      "text_align": align,
      "fg": selected == true ? "info.base" : "text.default"
    }
  )
  if choices.length() > 0 && !(editing == true)
    tone = value == "Paid" ? "success" : (value == "Open" ? "warning" : "info")
    inner = badge(value, tone)
  end
  if editing == true && editable == true && choices.length() > 0
    head = row(
      {
        "align": "center",
        "justify": align,
        "gap": 1,
        "grow": 1
      },
      [text(
        value,
        {
          "size": 1,
          "grow": 1,
          "clamp": 1
        }
      ), icon(
        open == true ? "chevron_up" : "chevron_down",
        {"width": 14, "height": 14, "fg": "text.muted"}
      )]
    )
    picks = choices.map(fn(o) {
      {
        "k": "box",
        "s": {
          "pad": [1, 2, 1, 2],
          "radius": 1,
          "bg": o == value ? "surface.sunken" : "none",
          "cursor": "pointer"
        },
        "p": props.merge({"value": o}),
        "on": {"click": on_select},
        "c": [text(
          o,
          {"size": 1, "weight": o == value ? "bold" : "regular"}
        )]
      }
    })
    inner = open == true ? column(
      {"gap": 0, "grow": 1},
      [head].concat(picks)
    ) : head
  end
  if editing == true && editable == true && choices.length() == 0
    inner = {
      "k": "input",
      "t": value,
      "s": {
        "grow": 1,
        "pad": [1, 2, 1, 2],
        "border": 1,
        "border_color": "focus.ring",
        "radius": 1,
        "bg": "surface.raised",
        "text_align": align
      },
      "p": props,
      "on": {
        "change": on_change,
        "submit": on_select,
        "blur": on_select
      }
    }
  end
  cursor = "pointer"
  cursor = "text" if editable == true && choices.length() == 0
  base = {
    "display": "row",
    "justify": align,
    "align": "center",
    "pad": [1, 2, 1, 2],
    "radius": 1,
    "cursor": cursor,
    "bg": selected == true ? "info.subtle" : "none",
    "transition": "fast"
  }
  hover = base.merge({"bg": selected == true ? "info.subtle" : "surface.sunken"})
  {
    "k": "box",
    "key": key,
    "s": base,
    "p": props,
    "on": {
      "click": on_select,
      "key_down": on_key,
      "pointer_enter": {"local": "self.style = @hover", "styles": {"hover": hover}},
      "pointer_leave": {"local": "self.style = @base", "styles": {"base": base}}
    },
    "c": [inner]
  }
end

def grid_row(record, columns, selected, editing, on_select, on_change, on_key)
  row_id = record["id"]
  cells = columns.map(fn(col) {
    id = col["id"]
    is_edit = editing["row"] == row_id && editing["col"] == id
    grid_cell(
      row_id,
      col,
      str(record[id] ?? ""),
      selected["row"] == row_id && selected["col"] == id,
      is_edit,
      is_edit && editing["open"] == true,
      on_select,
      on_change,
      on_key
    )
  })
  n = columns.length()
  n = 1 if n < 1
  keyed(
    row_id,
    {
      "k": "box",
      "s": {
        "display": "grid",
        "width": "100%",
        "gap": 4,
        "align": "center",
        "border": [0, 0, 1, 0],
        "border_color": "border.subtle"
      },
      "p": {"columns": n},
      "c": cells
    }
  )
end

def grid_header(columns, sort, on_sort)
  cells = columns.map(fn(col) {
    id = col["id"]
    active = sort["col"] == id
    mark = ""
    mark = sort["dir"] == "desc" ? " ↓" : " ↑" if active
    align = grid_col_align(col)
    hs = {
      "display": "row",
      "justify": align,
      "align": "center",
      "pad": [2, 2, 2, 2],
      "cursor": "pointer"
    }
    {
      "k": "box",
      "key": "grid-h:" + id,
      "s": hs,
      "p": {"col": id},
      "on": {"click": on_sort},
      "c": [text(
        col["label"] + mark,
        {
          "weight": "semibold",
          "size": 1,
          "text_align": align,
          "fg": active == true ? "accent.base" : "text.muted"
        }
      )]
    }
  })
  n = columns.length()
  n = 1 if n < 1
  {
    "k": "box",
    "s": {
      "display": "grid",
      "width": "100%",
      "gap": 4,
      "pad": [1, 2, 1, 2],
      "align": "center",
      "border": [0, 0, 1, 0],
      "border_color": "border.default",
      "bg": "surface.sunken"
    },
    "p": {"columns": n},
    "c": cells
  }
end

def data_grid(columns, rows, selected, editing, sort, on_select, on_sort, on_change, on_key)
  body = rows.map(fn(r) { grid_row(r, columns, selected, editing, on_select, on_change, on_key) })
  n = rows.length()
  h = n * 32
  # A list that fits its rows still eats the wheel. Only virtualise when
  # the body is taller than the cap; otherwise a column lets the page scroll.
  inner = h > 256 ? list({"height": 256, "width": "100%"}, 32, body) : column(
    {"gap": 0, "width": "100%"},
    body
  )
  column(
    {
      "gap": 0,
      "width": "100%",
      "border": 1,
      "border_color": "border.subtle",
      "radius": 2,
      "bg": "surface.raised"
    },
    [grid_header(columns, sort, on_sort), inner]
  )
end

# A button whose click runs a local chunk first, then a server event.
# `program` is the assembly list of spec/07; node targets are keys.
def local_button(label, program, after)
  b = button(label, after)
  b["on"]["click"] = {"local": program, "then": after}
  b
end

# The root node's props are the component's local state — merged into
# whatever props the root already carries, since a root may also ask the
# client for something (a `wake`, 06 §1.1).
def with_state(state, root)
  root["p"] = (root["p"] ?? {}).merge(state)
  root
end

# An image asset from the application, by path. The server hashes the file
# and the client fetches it once, by content, and caches it forever.
def image(src, width, height)
  {
    "k": "image",
    "p": {"src": src},
    "s": {"width": width, "height": height}
  }
end

def avatar(src, size)
  {
    "k": "image",
    "p": {"src": src},
    "s": {
      "width": size,
      "height": size,
      "radius": 4
    }
  }
end

# ------------------------------------------------------- catalogue, part 2
# Everything below composes from the same primitives. A widget that needs
# a state (open/closed, selected, page) keeps it in the handler; the widget
# only draws what it is told and carries the identity a handler needs in
# its props.

def progress(fraction)
  filled = (fraction * 100).to_i
  filled = 100 if filled > 100
  filled = 0 if filled < 0
  {
    "k": "box",
    "p": {
      "role": "progress",
      "label": "Progress",
      "value_now": filled,
      "value_min": 0,
      "value_max": 100
    },
    "s": {
      "display": "row",
      "height": 6,
      "radius": 4,
      "bg": "surface.sunken",
      "overflow": "clip",
      "grow": 1,
      "width": "100%"
    },
    "c": [{"k": "box", "s": {
      "width": filled.to_s + "%",
      "bg": "accent.base",
      "radius": 4
    }}]
  }
end

def skeleton(width, height)
  {"k": "box", "s": {
    "width": width,
    "height": height,
    "radius": 2,
    "bg": "surface.sunken"
  }}
end

# The × that drops a chip. A button carries its hover and press styles with
# it, so reusing one here and narrowing only the live style let the × jump
# back to a button's padding under the pointer. This one is a fixed box:
# what changes on hover is the colour, which 03 §5 animates without ever
# running layout again.
def chip_remove(on_remove, props, label)
  base = {
    "display": "row",
    "justify": "center",
    "align": "center",
    "width": 16,
    "height": 16,
    "radius": 4,
    "bg": "none",
    "fg": "text.muted",
    "cursor": "pointer",
    "transition": "fast"
  }
  hover = base.merge({"fg": "danger.base"})
  active = base.merge({"bg": "danger.subtle", "fg": "danger.base"})
  {
    "k": "box",
    "key": "chip-x:" + on_remove + ":" + str(props["id"] ?? ""),
    "s": base,
    "p": props.merge({"role": "button", "label": "Remove " + label.to_s}),
    "on": {
      "click": on_remove,
      "pointer_enter": {"local": "self.style = @hover", "styles": {"hover": hover}},
      "pointer_leave": {"local": "self.style = @base", "styles": {"base": base}},
      "pointer_down": {"local": "self.style = @active", "styles": {"active": active}},
      "pointer_up": {"local": "self.style = @hover", "styles": {"hover": hover}}
    },
    "c": [icon("close", {"width": 10, "height": 10})]
  }
end

def chip(label, on_remove, props)
  parts = [text(label, {"size": 1})]
  parts = parts.concat([chip_remove(on_remove, props, label)]) if on_remove.present?
  {
    "k": "box",
    "s": {
      "display": "row",
      "align": "center",
      "gap": 1,
      "pad": [0, 2, 0, 3],
      "radius": 4,
      "bg": "surface.sunken",
      "border": 1,
      "border_color": "border.subtle",
      "shrink": 0
    },
    "c": parts
  }
end

# A card that asks a wrapping row for `basis` pixels and takes an equal
# share of whatever the line has left: three tiles across on a desktop,
# two on a tablet, one on a phone, decided by the width itself rather
# than by a breakpoint, and with no hole at the end of the last line.
def tile(basis, node)
  node["s"]["width"] = "auto"
  node["s"]["basis"] = basis
  node["s"]["grow"] = 1
  node["s"]["shrink"] = 1
  node
end

def stat(label, value, hint)
  card(
    {"gap": 1, "width": "100%"},
    [muted(label), text(
      value,
      {"size": 6, "weight": "bold"}
    ), text(
      hint,
      {"fg": "text.muted", "size": 0}
    )]
  )
end

def empty_state(title, body, action_label, on_action)
  column(
    {
      "align": "center",
      "gap": 3,
      "pad": 8
    },
    [
      {"k": "box", "s": {
        "width": 48,
        "height": 48,
        "radius": 4,
        "bg": "surface.sunken"
      }},
      h2(title),
      text(
        body,
        {"fg": "text.muted", "text_align": "center"}
      ),
      button(action_label, on_action)
    ]
  )
end

def banner(message, tone, action_label, on_action)
  row(
    {
      "gap": 3,
      "align": "center",
      "pad": [2, 4, 2, 4],
      "radius": 2,
      "bg": tone + ".subtle",
      "border": [0, 0, 0, 3],
      "border_color": tone + ".base"
    },
    [text(message, {"fg": "text.default"}), spacer(), ghost_button(action_label, on_action)]
  )
end

# Breadcrumb: every crumb but the last is a link carrying its path.
def breadcrumb(crumbs, on_go)
  parts = []
  i = 0
  for crumb in crumbs
    parts = parts.concat([muted("/")]) if i > 0
    if i == crumbs.length() - 1
      parts = parts.concat([text(crumb["label"], {"weight": "semibold"})])
    else
      link = {
        "k": "text",
        "t": crumb["label"],
        "s": {"fg": "accent.base", "cursor": "pointer"},
        "on": {"click": on_go},
        "p": {"path": crumb["path"]}
      }
      parts = parts.concat([link])
    end
    i = i + 1
  end
  row(
    {
      "gap": 2,
      "align": "center",
      "shrink": 0
    },
    parts
  )
end

def pagination(page, pages, on_page)
  prev = icon_button("‹", on_page, {"page": page - 1}, {
    "tone": "neutral",
    "icon": "chevron_left",
    "name": "Previous page",
    "disabled": page <= 1
  })
  nxt = icon_button("›", on_page, {"page": page + 1}, {
    "tone": "neutral",
    "icon": "chevron_right",
    "name": "Next page",
    "disabled": page >= pages
  })
  row(
    {
      "gap": 2,
      "align": "center",
      "shrink": 0
    },
    [prev, muted(page.to_s + " / " + pages.to_s), nxt]
  )
end

# Segmented control: one row of options, the selected one raised.
def segmented(options, selected, on_select)
  count = options.length()
  cells = range(0, count).map(fn(i) {
    opt = options[i]
    is_sel = opt == selected
    {
      "k": "box",
      "p": {
        "option": opt,
        "role": "tab",
        "label": opt,
        "selected": is_sel,
        "pos_in_set": i + 1,
        "set_size": count
      },
      "s": {
        "pad": [1, 3, 1, 3],
        "radius": 1,
        "bg": is_sel ? "surface.raised" : "none",
        "cursor": "pointer",
        "border": is_sel ? 1 : 0,
        "border_color": "border.subtle"
      },
      "on": {"click": on_select},
      "c": [text(opt, is_sel ? {"weight": "semibold"} : {"fg": "text.muted"})]
    }
  })
  strip = row(
    {
      "gap": 1,
      "pad": 1,
      "radius": 2,
      "bg": "surface.sunken",
      "shrink": 0
    },
    cells
  )
  strip["p"] = {"role": "tab_list", "orientation": "horizontal"}
  strip
end

# Accordion: sections with a header that toggles by id; the open one shows its body.
def accordion(sections, open_id, on_toggle)
  column(
    {
      "gap": 0,
      "border": 1,
      "border_color": "border.subtle",
      "radius": 2
    },
    sections.map(fn(sec) {
      is_open = sec["id"] == open_id
      header = {
        "k": "box",
        "s": {
          "display": "row",
          "align": "center",
          "gap": 2,
          "pad": [2, 3, 2, 3],
          "cursor": "pointer",
          "border": [0, 0, 1, 0],
          "border_color": "border.subtle"
        },
        "on": {"click": on_toggle},
        "p": {"id": sec["id"]},
        "c": [icon(
          is_open ? "chevron_down" : "chevron_right",
          {"fg": "text.muted", "width": 14, "height": 14}
        ), text(sec["title"], {"weight": "semibold"})]
      }
      body = is_open ? [column({"pad": [
        2,
        3,
        3,
        3
      ]}, [text(sec["body"], {"fg": "text.muted"})])] : []
      column({"gap": 0}, [header].concat(body))
    })
  )
end

# Stepper: numbered steps, done ones filled, the current one ringed.
def stepper(steps, current)
  cells = []
  i = 0
  for label in steps
    tone = i < current ? "accent.base" : (i == current ? "surface.raised" : "surface.sunken")
    dot = {
      "k": "box",
      "s": {
        "width": 24,
        "height": 24,
        "radius": 4,
        "bg": tone,
        "display": "row",
        "justify": "center",
        "align": "center",
        "border": i == current ? 2 : 0,
        "border_color": "accent.base"
      },
      "c": [text(
        (i + 1).to_s,
        {
          "size": 0,
          "weight": "semibold",
          "fg": i < current ? "accent.on" : "text.default"
        }
      )]
    }
    cells = cells.concat([row(
      {"gap": 2, "align": "center"},
      [dot, text(label, i == current ? {"weight": "semibold"} : {"fg": "text.muted"})]
    )])
    if i < steps.length() - 1
      cells = cells.concat([{"k": "box", "s": {
        "width": 24,
        "height": 1,
        "bg": "border.default"
      }}])
    end
    i = i + 1
  end
  row(
    {"gap": 2, "align": "center"},
    cells
  )
end

# A menu is an overlay anchored by the caller: a raised column of items.
def menu(items, on_pick)
  column(
    {
      "gap": 0,
      "pad": 1,
      "radius": 2,
      "shadow": 2,
      "bg": "surface.overlay",
      "border": 1,
      "border_color": "border.subtle",
      "min_width": 160
    },
    items.map(fn(it) {
      {
        "k": "box",
        "s": {
          "pad": [1, 3, 1, 3],
          "radius": 1,
          "cursor": "pointer"
        },
        "on": {"click": on_pick},
        "p": {"item": it},
        "c": [text(it, {})]
      }
    })
  )
end

def tooltip(content)
  {
    "k": "box",
    "s": {
      "pad": [1, 2, 1, 2],
      "radius": 1,
      "bg": "text.default"
    },
    "c": [text(
      content,
      {"fg": "text.inverted", "size": 0}
    )]
  }
end

# A sheet slides from an edge over the page: overlay, dim, panel at the edge.
def sheet(side, children, opts = {})
  panel = column(
    {
      "gap": 4,
      "pad": 5,
      "bg": "surface.raised",
      "width": 320,
      "self": "stretch"
    },
    children
  )
  n = {
    "k": "overlay",
    "key": opts["key"] ?? ("sheet:" + side),
    "s": {
      "display": "row",
      "justify": side == "left" ? "start" : "end",
      "align": "stretch",
      "blur": 10,
      "bg": "#00000047"
    },
    "p": {
      "role": "dialog",
      "label": opts["label"] ?? "Panel",
      "modal": true,
      "autofocus": true,
      "keys": ["Escape"]
    },
    "c": [panel]
  }
  n["on"] = {"key_down": opts["on_close"]} unless opts["on_close"].nil?
  n
end
# Lighter than a dialog's, and blurred less: a sheet is somewhere you
# went, not a question you have to answer, and the page it slid over
# should stay recognisable behind it.

def drawer(children, opts = {})
  sheet("left", children, opts)
end

def popover(anchor, content, open)
  return anchor unless open

  stack({"gap": 0}, [
    anchor,
    {
      "k": "overlay",
      "s": {
        "position": "absolute",
        "margin": [2, 0, 0, 0],
        "pad": 3,
        "radius": 2,
        "shadow": 2,
        "bg": "surface.overlay",
        "border": 1,
        "border_color": "border.subtle",
        "z": 5
      },
      "c": content
    }
  ])
end

def toolbar(children)
  row(
    {
      "gap": 2,
      "align": "center",
      "pad": [1, 2, 1, 2],
      "bg": "surface.raised",
      "border": [0, 0, 1, 0],
      "border_color": "border.subtle"
    },
    children
  )
end

def navbar(brand, links, active, on_go)
  items = links.map(fn(l) {
    {
      "k": "text",
      "t": l,
      "s": l == active ? {"weight": "semibold"} : {
        "fg": "text.muted",
        "cursor": "pointer"
      },
      "on": {"click": on_go},
      "p": {"path": l}
    }
  })
  row(
    {
      "gap": 5,
      "align": "center",
      "pad": [2, 4, 2, 4],
      "bg": "surface.raised",
      "border": [0, 0, 1, 0],
      "border_color": "border.subtle"
    },
    [text(brand, {"weight": "bold"})].concat(items)
  )
end

def sidebar(links, active, on_go)
  column(
    {
      "gap": 1,
      "pad": 3,
      "width": 200,
      "bg": "surface.raised",
      "border": [0, 1, 0, 0],
      "border_color": "border.subtle"
    },
    links.map(fn(l) {
      {
        "k": "box",
        "s": {
          "pad": [1, 2, 1, 2],
          "radius": 1,
          "bg": l == active ? "surface.sunken" : "none",
          "cursor": "pointer"
        },
        "on": {"click": on_go},
        "p": {"path": l},
        "c": [text(l, l == active ? {
          "weight": "semibold",
          "fg": "accent.base"
        } : {})]
      }
    })
  )
end

def code_block(code)
  {
    "k": "box",
    "s": {
      "pad": 3,
      "radius": 2,
      "bg": "surface.sunken",
      "overflow": "clip"
    },
    "c": [text(
      code,
      {"font": "mono", "size": 1}
    )]
  }
end

# A read-only code viewer with line numbers and scrolling.
# Displays code in a monospace font with a pinned gutter (line numbers stay
# visible while scrolling horizontally). Gutter/code lines stay pixel-aligned
# because both use the same font_size, which determines line-height.
# `opts` may include {"language": "...", "line_numbers": true/false, "spans": [...]}.
# `spans` is an optional array of [start_byte, len_byte, color] triples for syntax highlighting.
def code_viewer(code, opts)
  opts = opts ?? {}
  show_numbers = opts["line_numbers"] ?? true

  # Split code by newlines to get line count; build gutter numbers.
  lines = code.split("\n")
  line_count = lines.length()
  digit_width = line_count.to_s.length()

  # Build line number strings: "   1", "   2", etc., right-aligned.
  # Include trailing newline to match code's line structure.
  numbers_text = range(0, line_count).map(fn(i) { (i + 1).to_s.rjust(digit_width) }).join("\n")

  # Code styling: monospace, compact size.
  code_style = {"font": "mono", "size": 1}
  gutter_style = {
    "font": "mono",
    "size": 1,
    "text_align": "end",
    "fg": "text.muted",
    "pad": [0, 2, 0, 2]
  }

  # Wrap code text if large (>2KB) to use interning. `if` is a statement
  # in Soli, not an expression, so the choice is a ternary.
  code_text_node = code.length() > 2000 ? text_interned(code, code_style) : text(code, code_style)

  # Apply syntax highlighting spans if provided. Assigned in one step
  # through `merge`: a nested `node["p"]["spans"] = …` writes into whatever
  # `node["p"]` evaluates to, which is not necessarily the hash still held
  # by the node.
  code_text_node["p"] = (code_text_node["p"] ?? {}).merge({"spans": opts["spans"]}) unless opts["spans"].nil?

  gutter_box = {
    "k": "box",
    "s": {
      "display": "column",
      "width": (digit_width * 8) + 8,
      "bg": "surface.raised",
      "overflow": "clip",
      "shrink": 0
    },
    "c": [text(numbers_text, gutter_style)]
  }
  # ~8px per digit + padding

  # Inner scroll holds the code; overflow: "scroll" gives unbounded width
  # so the code Text node doesn't wrap.
  code_scroll = {
    "k": "scroll",
    "s": {
      "display": "column",
      "overflow": "scroll",
      "grow": 1,
      "pad": [0, 0, 0, 3]
    },
    "c": [code_text_node]
  }
  # The code needs air on its left or the first character sits against
  # the gutter and the two columns read as one. The padding goes on the
  # scroller, not the text: padding the text node would move the run
  # the spans are measured against.

  # Content row: gutter + code, both inside.
  content_row = row(
    {"gap": 0, "align": "start"},
    show_numbers ? [gutter_box, code_scroll] : [code_scroll]
  )

  # The frame. Two cases, and they want two different kinds of box.
  #
  # With no ceiling asked for, the viewer shows the whole file and ends at
  # the last line — so it is a plain `box`, which a column sizes to its
  # content. A `scroll` cannot do that job: the engine gives a scroll its
  # bound rather than its content (04, `place_scroll`), so a scroll in a
  # column that has room takes all of it, and the only way to stop it is to
  # predict the content's height and hand it back as `max_height`. That
  # prediction was a line of mono at this size being 18 px, which is true
  # at font scale 1 and false at every other, and what it left under the
  # last line was a strip of sunken background with no gutter beside it.
  # Nothing here computes a height any more; the engine measures the text
  # that is actually there.
  #
  # With a ceiling, a scroll is exactly right: `max_height` is the bound,
  # the viewer stops there, and the code scrolls inside it.
  frame_style = {
    "radius": 2,
    "bg": "surface.sunken",
    "pad": 1,
    "overflow": "clip"
  }
  ceiling = opts["max_height"]
  ceiling.nil? ? node("box", frame_style.merge({"display": "column"}), [content_row]) : scroll(
    frame_style.merge({"max_height": ceiling}),
    [content_row]
  )
end

def tree_view(nodes, open_ids, on_toggle, depth)
  column({"gap": 0}, nodes.map(fn(n) {
    is_open = open_ids.includes?(n["id"])
    has_kids = n["children"].length() > 0
    rowv = {
      "k": "box",
      "s": {
        "display": "row",
        "gap": 1,
        "align": "center",
        "pad": [0, 1, 0, 1],
        "margin": [0, 0, 0, depth * 3],
        "cursor": has_kids ? "pointer" : "default"
      },
      "on": has_kids ? {"click": on_toggle} : {},
      "p": {"id": n["id"]},
      "c": [icon(
        has_kids ? (is_open ? "chevron_down" : "chevron_right") : "dot",
        {"fg": "text.muted", "width": 14, "height": 14}
      ), text(n["label"], {})]
    }
    kids = is_open && has_kids ? [tree_view(n["children"], open_ids, on_toggle, depth + 1)] : []
    column({"gap": 0}, [rowv].concat(kids))
  }))
end

# ---- Select ----------------------------------------------------------------

# A closed select is its anchor; open, a dropdown lists the options below it.
# The server owns `open`: the anchor toggles it, an option picks and closes.
# The anchor has a click handler, so Tab reaches it and Enter opens it.
def select(options, value, open, on_toggle, on_pick)
  select_sized(options, value, open, on_toggle, on_pick, 160, false)
end

def select_sized(options, value, open, on_toggle, on_pick, min_width, grow)
  s = {
    "display": "row",
    "align": "center",
    "gap": 2,
    "pad": [2, 3, 2, 3],
    "min_width": min_width,
    "border": 1,
    "border_color": "border.default",
    "radius": 2,
    "bg": "surface.raised",
    "cursor": "pointer"
  }
  s["grow"] = 1 if grow
  anchor = {
    "k": "box",
    "s": s,
    "on": {"click": on_toggle},
    "c": [text(value, {"grow": 1}), icon(
      "chevron_down",
      {"fg": "text.muted", "width": 14, "height": 14}
    )]
  }
  dropdown(anchor, options.map(fn(o) { select_option(o, o == value, on_pick, min_width) }), open)
end

def select_option(label, selected, on_pick, min_width)
  {
    "k": "box",
    "s": {
      "pad": [1, 3, 1, 3],
      "radius": 1,
      "min_width": min_width,
      "bg": selected ? "surface.sunken" : "none",
      "cursor": "pointer"
    },
    "p": {"value": label},
    "on": {"click": on_pick},
    "c": [text(label, {"weight": selected ? "bold" : "regular"})]
  }
end

# A popover that opens under its anchor rather than over it.
# An open list floats: it is an `overlay`, so it paints in the top layer
# and no card or scroller clips it, and it is `absolute` in a `stack`, so
# it neither grows the box it hangs off nor pushes the page open. Where it
# lands is the client's business (04 §5): under the anchor when the window
# has room, over it when it has not, and never past an edge. The top
# margin is the gap it keeps.
def dropdown(anchor, content, open)
  return anchor unless open

  stack({"gap": 0}, [
    anchor,
    {
      "k": "overlay",
      "s": {
        "position": "absolute",
        "margin": [2, 0, 0, 0],
        "pad": 1,
        "radius": 2,
        "shadow": 2,
        "bg": "surface.overlay",
        "border": 1,
        "border_color": "border.subtle",
        "display": "column",
        "z": 5
      },
      "c": content
    }
  ])
end

# ---- Slider ----------------------------------------------------------------

# A 240 px track. A press sets the value from the pointer x and a drag
# follows it; once the track has focus (Tab reaches it through its click
# handler) the arrow keys nudge it. The server owns the value: `on_set`
# receives `params["kind"]` — "click", "pointer_down", "pointer_move",
# "pointer_up", or "key_down" — and `params["payload"]`.
def slider(value, min, max, on_set)
  width = 240
  span = max - min
  span = 1 if span == 0
  filled = (value - min) * width / span
  lead = filled > 8 ? filled - 8 : 0
  {
    "k": "box",
    "s": {
      "display": "row",
      "align": "center",
      "width": width,
      "height": 24,
      "cursor": "grab"
    },
    "p": {
      "min": min,
      "max": max,
      "width": width,
      "role": "slider",
      "label": "Value",
      "value_now": value,
      "value_min": min,
      "value_max": max,
      "orientation": "horizontal"
    },
    "on": {
      "click": on_set,
      "key_down": on_set,
      "pointer_down": on_set,
      "pointer_move": on_set,
      "pointer_up": on_set
    },
    "c": [
      node("box", {
        "width": lead,
        "height": 4,
        "bg": "accent.base",
        "radius": 4
      }, []),
      node("box", {
        "width": 16,
        "height": 16,
        "radius": 4,
        "bg": "accent.base",
        "border": 2,
        "border_color": "surface.base"
      }, []),
      node("box", {
        "grow": 1,
        "height": 4,
        "bg": "surface.sunken",
        "radius": 4
      }, [])
    ]
  }
end

# ---- Calendar engine -------------------------------------------------------

# One engine, three pickers. `month` is "YYYY-MM"; days are ISO "YYYY-MM-DD"
# strings, which compare correctly as strings.
def month_label(month)
  DateTime.parse(month + "-01").format("%B %Y")
end

def month_shift(month, delta)
  first_day = DateTime.parse(month + "-01")
  moved = delta > 0 ? first_day.end_of_month().add_days(1) : first_day.add_days(-1)
  moved.format("%Y-%m")
end

def weekday_index(day)
  {
    "Monday": 0,
    "Tuesday": 1,
    "Wednesday": 2,
    "Thursday": 3,
    "Friday": 4,
    "Saturday": 5,
    "Sunday": 6
  }[day.weekday()]
end

def two_digits(n)
  n < 10 ? "0" + str(n) : str(n)
end

# `glyph` is what is drawn; `name` is what it is called. A control whose only
# text is "×" is announced as "×", which is the reason a name is worth giving
# even where the argument has a default.
def icon_button(glyph, on_click, props, o = {})
  size = o["size"] ?? "md"
  box = icon_box_px(size)
  control({
    "key": o["key"] ?? ("ib:" + on_click.to_s + ":" + glyph),
    "tone": o["tone"] ?? "quiet",
    "size": size,
    "shape": {"width": box, "height": box, "radius": 1, "pad": 0, "min_width": box},
    "on": {"click": on_click},
    "props": props,
    "disabled": o["disabled"] == true,
    "a11y": {
      "role": "button",
      "label": o["name"] ?? glyph,
      "expanded": o["expanded"]
    },
    "c": [icon_or_glyph(glyph, o["icon"], size)]
  })
end

# An icon button draws a named icon when it is given one, and the character it
# was handed when it is not. The catalogue is mid-move from the second to the
# first, and both have to work while it is.
def icon_or_glyph(glyph, name, size)
  return text(glyph, {"size": control_text_size(size), "weight": "bold"}) if name.nil?

  side = int(icon_box_px(size) * 0.6)
  icon(name, {"width": side, "height": side})
end

def day_cell(iso, label, selected, in_range, on_pick)
  {
    "k": "box",
    "s": {
      "height": 32,
      "radius": 1,
      "display": "row",
      "justify": "center",
      "align": "center",
      "bg": selected ? "accent.base" : (in_range ? "info.subtle" : "none"),
      "cursor": "pointer"
    },
    "p": {"date": iso},
    "on": {"click": on_pick},
    "c": [text(
      label,
      {"fg": selected ? "accent.on" : "text.default", "size": 1}
    )]
  }
end

def day_blank
  node("box", {"height": 32}, [])
end

def weekday_header
  cells = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"].map(fn(w) {
    node("box", {"display": "row", "justify": "center"}, [muted(w)])
  })
  {
    "k": "box",
    "s": {
      "display": "grid",
      "gap": 0,
      "width": "100%"
    },
    "p": {"columns": 7},
    "c": cells
  }
end

# The month grid: navigation, weekday header, seven columns of days.
# `selected` is a list of ISO days; `range_start`/`range_end` shade between.
def calendar(month, selected, range_start, range_end, on_pick, on_nav)
  first_day = DateTime.parse(month + "-01")
  blanks = range(0, weekday_index(first_day)).map(fn(i) { day_blank() })
  cells = range(1, first_day.end_of_month().day() + 1).map(fn(d) {
    iso = month + "-" + two_digits(d)
    shaded = range_start.present? && range_end.present? && iso >= range_start && iso <= range_end
    day_cell(iso, str(d), selected.includes?(iso), shaded, on_pick)
  })
  header = row(
    {"align": "center", "gap": 1},
    [
      icon_button("‹", on_nav, {"delta": -1}, {"icon": "chevron_left", "name": "Previous month"}),
      text(
        month_label(month),
        {
          "weight": "semibold",
          "grow": 1,
          "text_align": "center"
        }
      ),
      icon_button("›", on_nav, {"delta": 1}, {"icon": "chevron_right", "name": "Next month"})
    ]
  )
  grid = {
    "k": "box",
    "s": {
      "display": "grid",
      "gap": 0,
      "width": "100%"
    },
    "p": {"columns": 7},
    "c": blanks.concat(cells)
  }
  # A month is a panel of its own: a surface a shade lighter than the card
  # it sits on, padded and rounded, so the days read as one block rather
  # than as text loose on the card.
  column(
    {
      "gap": 1,
      "width": "100%",
      "pad": 3,
      "radius": 2,
      "bg": "surface.overlay",
      "border": 1,
      "border_color": "border.subtle"
    },
    [header, weekday_header(), grid]
  )
end

# ---- Pickers ---------------------------------------------------------------

def date_picker(month, value, on_pick, on_nav)
  column(
    {"gap": 2, "width": "100%"},
    [
      calendar(month, value.present? ? [value] : [], "", "", on_pick, on_nav),
      muted(value.present? ? value : "Pick a day")
    ]
  )
end

# A date and a time: the calendar plus hour and minute selects, so the
# clock cannot hold anything but HH:MM.
def datetime_picker(
  month,
  date,
  time,
  hour_open,
  min_open,
  on_pick,
  on_nav,
  on_hour_toggle,
  on_min_toggle,
  on_hour,
  on_min
)
  bits = (time ?? "00:00").split(":")
  hour = bits[0]
  minute = "00"
  minute = bits[1] if bits.length() > 1
  hours = range(0, 24).map(fn(h) { two_digits(h) })
  minutes = range(0, 60).map(fn(m) { two_digits(m) })
  clock_column = column(
    {"gap": 1, "width": "100%"},
    [
      muted("Time"),
      row(
        {
          "gap": 2,
          "align": "center",
          "width": "100%"
        },
        [
          select_sized(hours, hour, hour_open, on_hour_toggle, on_hour, 64, true),
          text(":", {"weight": "bold"}),
          select_sized(minutes, minute, min_open, on_min_toggle, on_min, 64, true)
        ]
      )
    ]
  )
  column(
    {"gap": 2, "width": "100%"},
    [calendar(month, date.present? ? [date] : [], "", "", on_pick, on_nav), clock_column, muted(date + " " + time)]
  )
end

def sized_input(value, on_change, width)
  box = input(value, on_change)
  box["s"]["width"] = width
  box
end

# Two selections on one calendar: the first click starts, the second ends,
# the third starts over. The server keeps the two ends ordered.
def date_range_picker(month, start, finish, on_pick, on_nav)
  ends = [start, finish].filter(fn(d) { d.present? })
  caption = finish.present? ? start + " → " + finish : (start.present? ? start + " → …" : "Pick a start day")
  column(
    {"gap": 2, "width": "100%"},
    [calendar(month, ends, start, finish, on_pick, on_nav), muted(caption)]
  )
end

# A titled card, so a picker reads as one thing.
def labelled(title, child)
  card({"gap": 3}, [text(title, {"weight": "bold"}), child])
end

# ---- The dev bar --------------------------------------------------------
#
# A dev bar in HTML is spliced into the document on its way out. EUI has no
# document: the server composes the tree, so the bar is a widget the
# application places itself — an `overlay`, which paints in the top layer and
# is clipped by nothing (03 §2.4), pinned to the bottom of the window.
#
# What it reports is what EUI costs: the view and the encode in milliseconds,
# the ops and the bytes that went on the wire, and the four tables a session
# interns once. `eui_stats()` gives the *previous* render — the work behind
# what is on the screen — and gives nothing at all outside `--dev`, so a view
# can compose `dev_bar(eui_stats())` and ship it: with no numbers there is no
# bar, and the node never reaches the client.
#
# Nothing the window knows is in here — frame time, quads, memory. Spec 08
# says the client reports nothing about the machine beyond its viewport, and
# a dev bar is not a reason to change that.
#
# It sits over the page rather than beside it, and EUI has no way to make a
# node transparent to the pointer, so it takes the clicks that land on it.
# That is why it is one line at the very bottom and not a panel.

def dev_figure(label, value, tone)
  row(
    {"gap": 2, "align": "baseline"},
    [
      text(value, {"font": "mono", "size": 0, "weight": "semibold", "fg": tone}),
      text(label, {"font": "mono", "size": 0, "fg": "text.muted"})
    ]
  )
end

# How heavy the last patch was, as a colour: a render that sends a few ops is
# what the design is for, and one that sends the tree is worth noticing.
def dev_wire_tone(ops)
  return "success.base" if ops <= 24
  return "warning.base" if ops <= 200

  "danger.base"
end

def dev_bar(stats, shown = true)
  return {"k": "box", "s": {"display": "none"}} if (stats ?? {})["renders"].nil? || !shown

  ms = fn(v) { str((v * 10).round() / 10.0) + " ms" }
  figures = [
    dev_figure("event", stats["event"].blank? ? "—" : stats["event"], "accent.base"),
    dev_figure("view", ms(stats["view_ms"]), "text.default"),
    dev_figure("encode", ms(stats["encode_ms"]), "text.default"),
    dev_figure("ops", str(stats["ops"]), dev_wire_tone(stats["ops"])),
    dev_figure("B", str(stats["bytes"]), dev_wire_tone(stats["ops"])),
    dev_figure("nodes", str(stats["nodes"]), "text.default"),
    dev_figure("seq", str(stats["seq"]), "text.muted"),
    dev_figure("renders", str(stats["renders"]), "text.muted"),
    dev_figure(
      "interned",
      [stats["atoms"], stats["styles"], stats["colors"], stats["chunks"]].map(fn(n) { str(n) }).join("/"),
      "text.muted"
    )
  ]
  {
    "k": "overlay",
    "s": {
      "position": "absolute",
      "align": "end",
      "justify": "center",
      "width": "100%"
    },
    "c": [row(
      {
        "gap": 4,
        "align": "center",
        "wrap": "wrap",
        "pad": [1, 3, 1, 3],
        "margin": [0, 0, 2, 0],
        "radius": 2,
        "bg": "surface.overlay",
        "border": 1,
        "border_color": "border.subtle",
        "shadow": 2
      },
      [dev_bar_tag()].concat(figures)
    )]
  }
end

# The tag at the head of the bar is also its switch: a click hides the bar
# for the rest of the session. It asks the server rather than repointing a
# style locally, because the next render would draw the bar again.
def dev_bar_tag
  {
    "k": "box",
    "s": {"cursor": "pointer", "pad": [0, 1, 0, 1], "radius": 1},
    "on": {"click": "dev_bar_toggle"},
    "c": [text("EUI", {"font": "mono", "size": 0, "weight": "bold", "fg": "accent.base"})]
  }
end

# ---- Charts ----------------------------------------------------------------

# A chart is a `canvas` with a `paths` prop, spec 03 §1.1: each path is a
# list — kind, colour, then numbers in logical px from the content box.
#   [0, colour, width, x0, y0, x1, y1, …]   polyline, round caps and joins
#   [1, colour, x, y, w, h, radius]          filled rectangle
#   [2, colour, base_y, x0, y0, x1, y1, …]   area between a polyline and base_y
#   [3, colour, cx, cy, r]                   filled circle
#   [4, colour, width, cx, cy, r, a0, a1]    arc, radians, clockwise from +x
def canvas(width, height, paths)
  {
    "k": "canvas",
    "s": {"width": width, "height": height},
    "p": {"paths": paths}
  }
end

def chart_max(values)
  top = 0
  for v in values
    top = v if v > top
  end
  top > 0 ? top : 1
end

# A series scaled into `w × h` with 4 px of breathing room, as [x, y] pairs.
def chart_points(values, w, h)
  top = chart_max(values)
  count = values.length()
  step = count > 1 ? (w - 8) / (count - 1) : 0
  range(0, count).map(fn(i) { [4 + i * step, 4 + (h - 8) * (top - values[i]) / top] })
end

# Four hairlines, so a series has something to be read against.
def chart_grid(w, h)
  range(0, 4).map(fn(i) { [
    0,
    "border.subtle",
    1,
    4,
    4 + (h - 8) * i / 3,
    w - 4,
    4 + (h - 8) * i / 3
  ] })
end

def flatten_points(points)
  flat = []
  for p in points
    flat = flat.concat(p)
  end
  flat
end

# The four roles a chart spends, in order.
def chart_role(i)
  roles = ["accent.base", "info.base", "success.base", "warning.base"]
  roles[i % 4]
end

# ---- Answering the pointer -------------------------------------------------
#
# The client hit-tests boxes, not paths: a canvas is one node, so a chart that
# answers the pointer needs boxes over it. Every chart is a `stack` of three
# layers — a row of wash columns behind the drawing, the drawing itself, and a
# row of invisible bands in front holding the handlers and the value chips.
# Entering a band repoints two nodes at styles the handler declared, its wash
# and its chip, and leaving puts them back; the keys are how a chunk names a
# node it does not carry (07 §1), so they are identifiers: `cw_line_3`.
#
# Nothing here waits for the network, and nothing here moves: what the new
# records change is a colour and an opacity, which 03 §5 animates on its own
# while layout stays exactly where it was.

# One column of the plot, behind the drawing. `lit` is the state under the
# pointer; the width is in the record because the handler has to declare the
# same box it is repointing, not a narrower one.
def chart_wash_style(width, lit)
  {
    "width": width,
    "height": "100%",
    "radius": 2,
    "bg": lit ? "surface.sunken" : "none",
    "transition": "fast"
  }
end

# The tooltip: a chip that is always there and is transparent until the
# pointer is in its band. Fading one in costs no layout; mounting one would.
def chart_chip_style(shown)
  {
    "bg": "surface.overlay",
    "fg": "text.default",
    "border": 1,
    "border_color": "border.subtle",
    "radius": 2,
    "shadow": 1,
    "pad": [1, 2, 1, 2],
    "size": 0,
    "weight": "semibold",
    "opacity": shown ? 255 : 0,
    "transition": "fast"
  }
end

# A string as the local language's source will read it back: a chunk's
# `set_text` takes a literal, and a literal wants its quotes.
def chart_quoted(s)
  "\"" + s + "\""
end

# Where the bands meet: the midpoint between neighbouring marks, rounded once
# so the columns still add up to the plot's width — a band per mark, from the
# left edge of the plot to its right.
def chart_spans(centres, w)
  count = centres.length()
  edges = [4]
  for i in range(1, count)
    edges = edges.concat([int(((centres[i - 1] + centres[i]) / 2) + 0.5)])
  end
  edges = edges.concat([int(w - 4)])
  range(0, count).map(fn(i) { edges[i + 1] - edges[i] })
end

# One band: an invisible box over its share of the plot, carrying the chip and
# the two handlers that light the pair.
def chart_band(id, i, width, label)
  wash_key = "cw_" + id + "_" + str(i)
  chip_key = "ct_" + id + "_" + str(i)
  {
    "k": "box",
    "s": {
      "display": "column",
      "justify": "start",
      "align": "center",
      "pad": [1, 0, 0, 0],
      "width": width,
      "height": "100%"
    },
    "on": {
      "pointer_enter": {
        "local": wash_key + ".style = @lit; " + chip_key + ".style = @shown",
        "styles": {"lit": chart_wash_style(width, true), "shown": chart_chip_style(true)}
      },
      "pointer_leave": {
        "local": wash_key + ".style = @rest; " + chip_key + ".style = @hidden",
        "styles": {"rest": chart_wash_style(width, false), "hidden": chart_chip_style(false)}
      }
    },
    "c": [keyed(chip_key, text(label, chart_chip_style(false)))]
  }
end

# The three layers, stacked on the plot the drawing was scaled into. The
# strips are the plot itself — `w − 8` by `h − 8`, centred — so a band sits
# exactly over the marks `chart_points` placed.
def chart_layers(id, spans, labels, w, h, drawing)
  count = spans.length()
  washes = range(0, count).map(fn(i) {
    keyed("cw_" + id + "_" + str(i), {"k": "box", "s": chart_wash_style(spans[i], false)})
  })
  bands = range(0, count).map(fn(i) { chart_band(id, i, spans[i], labels[i]) })
  stack(
    {"width": w, "height": h, "justify": "center", "align": "center"},
    [
      row({"width": int(w - 8), "height": int(h - 8)}, washes),
      drawing,
      row({"width": int(w - 8), "height": int(h - 8)}, bands)
    ]
  )
end

def chart_labels(values)
  values.map(fn(v) { str(v) })
end

def chart_line(id, values, w, h)
  points = chart_points(values, w, h)
  line = [0, "accent.base", 2].concat(flatten_points(points))
  dots = points.map(fn(p) { [
    3,
    "accent.base",
    p[0],
    p[1],
    3
  ] })
  drawing = canvas(w, h, chart_grid(w, h).concat([line]).concat(dots))
  spans = chart_spans(points.map(fn(p) { p[0] }), w)
  chart_layers(id, spans, chart_labels(values), w, h, drawing)
end

def chart_area(id, values, w, h)
  points = chart_points(values, w, h)
  flat = flatten_points(points)
  area = [2, "info.subtle", h - 4].concat(flat)
  line = [0, "info.base", 2].concat(flat)
  drawing = canvas(w, h, chart_grid(w, h).concat([area, line]))
  spans = chart_spans(points.map(fn(p) { p[0] }), w)
  chart_layers(id, spans, chart_labels(values), w, h, drawing)
end

def chart_bar(id, values, w, h)
  top = chart_max(values)
  count = values.length()
  slot = (w - 8) / count
  bars = range(0, count).map(fn(i) {
    bar_h = (h - 8) * values[i] / top;
    [1, "accent.base", 4 + i * slot + slot / 8, h - 4 - bar_h, slot - slot / 4, bar_h, 1]
  })
  drawing = canvas(w, h, chart_grid(w, h).concat(bars))
  centres = range(0, count).map(fn(i) { 4 + i * slot + slot / 2 })
  chart_layers(id, chart_spans(centres, w), chart_labels(values), w, h, drawing)
end

# A donut has no bands: an arc is not a box, and a quadrant is not an arc. Its
# legend is the thing under the pointer instead, and what it shows is the
# reading in the hole — one `set_text` for the number, one for the name, which
# is what a chunk is for (07 §1).
def chart_donut_legend_row(id, i, part, label, total)
  rest = {
    "display": "row",
    "align": "center",
    "gap": 2,
    "pad": [1, 1, 1, 1],
    "radius": 2,
    "bg": "none",
    "width": "100%",
    "transition": "fast"
  }
  lit = rest.merge({"bg": "surface.sunken"})
  share = total > 0 ? int(part * 100 / total) : 0
  reading = str(part) + " · " + str(share) + "%"
  says = "dv_" + id + ".text = " + chart_quoted(reading) + "; dl_" + id + ".text = " + chart_quoted(label)
  rests = "dv_" + id + ".text = " + chart_quoted(str(total)) + "; dl_" + id + ".text = " + chart_quoted("Total")
  swatch = {"k": "box", "s": {"width": 10, "height": 10, "radius": 4, "bg": chart_role(i)}}
  {
    "k": "box",
    "key": "dr_" + id + "_" + str(i),
    "s": rest,
    "on": {
      "pointer_enter": {"local": "self.style = @lit; " + says, "styles": {"lit": lit}},
      "pointer_leave": {"local": "self.style = @rest; " + rests, "styles": {"rest": rest}}
    },
    "c": [swatch, text(label, {"size": 1}), spacer(), text(str(part), {"size": 1, "fg": "text.muted"})]
  }
end

# A donut: one arc per part, in the four "base" roles, a small gap between,
# and the total in the hole until a legend row says otherwise.
def chart_donut(id, parts, labels, w, h)
  total = int(parts.sum())
  scale = total > 0 ? total : 1
  radius = (w < h ? w : h) / 2 - 10
  arcs = []
  start = -1.5707963
  i = 0
  for p in parts
    sweep = 6.2831853 * p / scale
    arcs = arcs.concat([ [
      4,
      chart_role(i),
      14,
      w / 2,
      h / 2,
      radius,
      start,
      start + sweep - 0.04
    ]])
    start = start + sweep
    i = i + 1
  end
  hole = column(
    {"gap": 0, "align": "center", "justify": "center"},
    [
      keyed("dv_" + id, text(str(total), {"size": 3, "weight": "bold"})),
      keyed("dl_" + id, text("Total", {"size": 0, "fg": "text.muted"}))
    ]
  )
  # As wide as the wheel, so the two read as one block whatever the cell
  # around them is doing.
  legend = column(
    {"gap": 1, "width": w},
    range(0, parts.length()).map(fn(j) {
      chart_donut_legend_row(id, j, parts[j], labels[j] ?? ("Part " + str(j + 1)), total)
    })
  )
  wheel = stack(
    {"width": w, "height": h, "justify": "center", "align": "center"},
    [canvas(w, h, arcs), hole]
  )
  column({"gap": 3, "width": "100%", "align": "center"}, [wheel, legend])
end

# ---- Feed ------------------------------------------------------------------

# A text whose content repeats across many nodes: interned as an atom, so
# the wire carries it once per session.
def text_interned(content, style)
  interned = text(content, style)
  interned["intern"] = true
  interned
end

# A face when there is one, a coloured initial when there is not: a real
# timeline brings pictures, the sample brings letters, and the card treats
# them the same.
def post_avatar(post, size)
  face = post["avatar"]
  return initial_avatar(post["initial"], post["tone"], size) if face.blank?

  built = avatar(face, size)
  built["s"]["shrink"] = 0
  built
end

# An initial in a coloured disc, in place of a fetched avatar.

def initial_avatar(letter, tone, size)
  {
    "k": "box",
    "s": {
      "width": size,
      "height": size,
      "radius": 4,
      "bg": tone,
      "display": "row",
      "justify": "center",
      "align": "center"
    },
    "c": [text_interned(
      letter,
      {
        "fg": "accent.on",
        "weight": "bold",
        "size": 3
      }
    )]
  }
end

# One action under a post: a glyph and a count, clickable, carrying the
# post id so one handler serves every post.
def post_action(glyph, count, on_click, props, active)
  {
    "k": "box",
    "s": {
      "display": "row",
      "gap": 1,
      "align": "center",
      "pad": [1, 2, 1, 2],
      "radius": 2,
      "cursor": "pointer",
      "fg": active ? "accent.base" : "text.muted"
    },
    "p": props,
    "on": {"click": on_click},
    "c": [text_interned(glyph, {"size": 1}), text(str(count), {"size": 1})]
  }
end

# A post card of fixed height, so a feed of thousands can be virtualised:
# the client lays out only the cards it can see.
# What a card carries under its text: a picture, a moving picture, or a
# sound with a button to start it. The sound node exists only while that
# card is the one playing — the client holds a few sources, not a feed of
# them.
# A bar showing where a picture or a sound is, clickable to seek. The
# width is fixed so the click's x maps straight onto the position.
def media_scrubber(width, at, duration, on_seek, props)
  filled = duration > 0 ? int(width * at / duration) : 0
  filled = width if filled > width
  {
    "k": "box",
    "s": {
      "display": "row",
      "align": "center",
      "width": width,
      "height": 6,
      "radius": 4,
      "bg": "surface.sunken",
      "cursor": "pointer"
    },
    "p": props,
    "on": {"click": on_seek},
    "c": [{"k": "box", "s": {
      "width": filled,
      "height": 6,
      "radius": 4,
      "bg": "accent.base"
    }}]
  }
end

def media_clock(ms)
  seconds = ms / 1000
  str(seconds / 60) + ":" + (seconds % 60 < 10 ? "0" : "") + str(seconds % 60)
end

# A play/pause button of the size these cards use.
def media_button(on, event, props)
  {
    "k": "box",
    "s": {
      "display": "row",
      "align": "center",
      "justify": "center",
      "width": 28,
      "height": 28,
      "radius": 4,
      "bg": on ? "accent.base" : "surface.sunken",
      "fg": on ? "accent.on" : "text.default",
      "cursor": "pointer",
      "shrink": 0
    },
    "p": props,
    "on": {"click": event},
    "c": [text(on ? "▮▮" : "▶", {"size": 1})]
  }
end

def post_media(post, play)
  playing = play["sound"] ?? false
  kind = post["media"]
  if kind == "image"
    return [{
      "k": "image",
      "p": {"src": post["image"]},
      "s": {
        "width": "100%",
        "max_width": 480,
        "height": 180,
        "radius": 2
      }
    }]
  end

  if kind == "video"
    on = play["video"] ?? false
    at = play["at"] ?? 0
    duration = play["duration"] ?? 0
    props = {"id": post["id"]}
    return [
      video("public/video/pulse.gif", {
        "playing": on,
        "loop": false,
        "position": play["seek"] ?? 0
      }, {
        "width": "100%",
        "max_width": 480,
        "height": 180,
        "radius": 2
      }, {"time_update": "video_time", "ended": "video_ended"}),
      row(
        {
          "gap": 3,
          "align": "center",
          "width": "100%",
          "max_width": 480
        },
        [
          media_button(on, "video_play", props),
          media_scrubber(300, at, duration > 0 ? duration : 1440, "video_seek", props.merge({"w": 300})),
          muted(media_clock(at) + " / " + media_clock(duration > 0 ? duration : 1440))
        ]
      )
    ]
  end
  # Full width, like a video on a timeline anywhere else.

  if kind == "audio"
    controls = row(
      {"gap": 3, "align": "center"},
      [
        {
          "k": "box",
          "s": {
            "display": "row",
            "gap": 2,
            "align": "center",
            "pad": [2, 3, 2, 3],
            "radius": 4,
            "bg": playing ? "accent.base" : "surface.sunken",
            "fg": playing ? "accent.on" : "text.default",
            "cursor": "pointer"
          },
          "p": {"id": post["id"]},
          "on": {"click": "play"},
          "c": [text(
            playing ? "▮▮  Playing" : "▶  Play the chime",
            {"size": 1, "weight": "semibold"}
          )]
        },
        muted("1.6 s")
      ]
    )
    parts = [controls]
    if playing
      parts = parts.concat([audio("public/sounds/chime.wav", {
        "playing": true,
        "volume": 80
      }, {"ended": "sound_ended"})])
    end
    return parts
  end

  []
end

def post_card(post, liked, play, height)
  header = row(
    {
      "gap": 2,
      "align": "center",
      "wrap": "wrap"
    },
    [
      text(
        "#" + str(post["n"]),
        {
          "fg": "text.muted",
          "size": 1,
          "font": "mono"
        }
      ),
      text_interned(post["name"], {"weight": "semibold"}),
      text_interned(
        post["handle"],
        {"fg": "text.muted", "size": 1}
      ),
      text_interned(
        "· " + post["when"],
        {"fg": "text.muted", "size": 1}
      )
    ]
  )
  # The card's number, so a scroll through a hundred thousand of them
  # can be checked by eye: nothing skipped, nothing repeated.
  body = text(post["text"], {"clamp": 2})
  picture = post_media(post, play)
  actions = row(
    {"gap": 4, "align": "center"},
    [
      post_action("↩", post["replies"], "noop", {"id": post["id"]}, false),
      post_action("⟳", post["reposts"], "noop", {"id": post["id"]}, false),
      post_action("♥", post["likes"] + (liked ? 1 : 0), "like", {"id": post["id"]}, liked)
    ]
  )
  {
    "k": "box",
    "s": {
      "display": "row",
      "gap": 3,
      "pad": [3, 4, 3, 4],
      "height": height,
      "overflow": "clip",
      "border": [0, 0, 1, 0],
      "border_color": "border.subtle",
      "align": "start"
    },
    "p": {"item_height": height},
    "c": [
      post_avatar(post, 40),
      column(
        {
          "gap": 1,
          "grow": 1,
          "shrink": 1,
          "min_width": 0
        },
        [header, body].concat(picture).concat([actions])
      )
    ]
  }
end
