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

# `o` narrows a field without its caller having to reach into the hash
# afterwards: `style` is merged over the resting style, `props` is the
# identity and the semantics the handler and the screen reader read back,
# `key` names the node, and `on` adds handlers beside `change`.
#
# `change` is not one per keystroke. The client sends it when the field is
# left or `Enter` is pressed, and only when the value differs from the one
# the server sent, so a field is never judged while it is still being typed
# into. A view that does want every letter asks for `text_input` in `o`.
def input(value, on_change, o = {})
  editable("input", value, on_change, o)
end

# One line of body text at cozy density, in px. It is a floor for an empty
# textarea and nothing else: the client scales text with the viewer's font
# scale and the box grows with its content, so nothing is laid out from this.
TEXT_LINE_PX = 22

# The multi-line field, and the only editable kind the client has besides
# `input`. The same node in every respect but that kind, and the kind is what
# makes the client wrap the text, put the caret on the line the pointer
# landed on, and keep `Enter` for a newline instead of a submit.
#
# `o["rows"]` is a floor, not a ceiling: the box is at least that many lines
# tall and grows with what is typed into it, because a textarea that starts
# scrolling at three lines hides the paragraph it was asked to hold.
def textarea(value, on_change, o = {})
  rows = o["rows"] ?? 3
  editable("textarea", value, on_change, o.merge({
    "style": {"min_height": rows * TEXT_LINE_PX}.merge(o["style"] ?? {})
  }))
end

# What both of them are. The border is reserved at rest and only coloured
# later, for the reason `button_variant` gives below: a border that appears
# when a value goes wrong would shove every field under it sideways.
#
# The background is `surface.sunken` because a field with none of its own is
# the colour of the card it sits on — nothing says where the box is until
# something has been typed into it. `sunken` is the role that means inset,
# and 05 §3 resolves it away from the surface in **both** palettes (0.955
# against 0.985 in light, 0.15 against 0.19 in dark), so one word here is a
# correct contrast in every mode and the server still sends no colour.
def editable(kind, value, on_change, o)
  base = {
    "pad": [2, 3, 2, 3],
    "border": 1,
    "border_color": "border.default",
    "radius": 2,
    "bg": "surface.sunken",
    "transition": "fast"
  }
  style = base.merge(o["style"] ?? {})
  key = o["key"].to_s
  key = editable_key(on_change, o) if key.blank?
  n = {
    "k": kind,
    "t": value ?? "",
    "s": style,
    "on": {"change": on_change}.merge(o["on"] ?? {})
  }
  unless key.blank?
    n["key"] = key
    n["on"] = editable_states(style, key, n["on"])
  end
  props = o["props"] ?? {}
  n["p"] = props if props.keys().length() > 0
  n
end

# A name for a field nobody named. A local handler reaches its own node by
# key (07 §1), so a field without one can have no states at all; the event it
# sends and the label it carries are what tell two fields apart, and a field
# with neither is one this cannot help.
def editable_key(on_change, o)
  label = (o["props"] ?? {})["label"].to_s
  said = on_change.to_s
  return "" if said.blank? && label.blank?

  "ed:" + said + ":" + label
end

# Hover and focus, both local (07 §6): the client repoints the node at a
# style the session already holds, so neither waits for a round trip.
#
# Focus earns its keep more than hover does. The client draws its ring for
# *keyboard* focus alone (03 §3), so a field clicked into had nothing to say
# it was the one taking the keystrokes, and a form of eight fields looked the
# same whichever one was live.
#
# `state.field_focus` is why leaving is a question and not a reset: a pointer
# that wanders off a field someone is still typing into must not take the
# focused look with it. Focus writes this field's key there and blur clears
# it, and a chunk may read a root prop — so `pointer_leave` asks who holds
# focus before it decides what to go back to.
#
# A handler the caller already put on one of these four is kept, and runs
# *after* the chunk (`then`, 07 §6): a combobox needs its `blur` and its look
# at once. One that is itself local is left alone — two chunks on one event
# is the caller's business, not this function's.
def editable_states(style, key, on)
  bad = style["border_color"] == "danger.base"
  hover = style.merge({"bg": "surface.base"})
  focus = style.merge({"bg": "surface.base"})
  hover["border_color"] = "border.strong" unless bad
  focus["border_color"] = "accent.base" unless bad
  styles = {"base": style, "hover": hover, "focus": focus}
  mine = "\"" + key + "\""
  held = "if state.field_focus == " + mine
  states = {
    "pointer_enter": {"local": held + " { self.style = @focus } else { self.style = @hover }", "styles": styles},
    "pointer_leave": {"local": held + " { self.style = @focus } else { self.style = @base }", "styles": styles},
    "focus": {"local": "state.field_focus = " + mine + "; self.style = @focus", "styles": styles},
    "blur": {"local": "state.field_focus = \"\"; self.style = @base", "styles": styles}
  }
  out = on.merge({})
  for name in states.keys()
    said = out[name]
    if said.nil?
      out[name] = states[name]
    elsif said.to_s == said
      out[name] = states[name].merge({"then": said})
    end
  end
  out
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

# What a field stands at. The client gives a real `input` a floor of one
# control (04, `place`: "an editable field is at least one control tall"), and
# gives a box built to look like one no way to ask for the same — so a
# `select`, a date field and a `multi_select` all came out four pixels under
# the text field beside them. This is that floor, said on the server.
#
# It is exact rather than approximate: the theme scales `control` by density
# and by nothing else, which is what `control_px` already reproduces. Density
# reaches the view in `params["viewport"]`, and a caller that does not pass it
# gets the cozy default, which is what it was before.
def field_height(o)
  control_px("md", o["density"] ?? "cozy")
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

# A link is a button that goes somewhere rather than doing something, so it
# is text in the accent colour and not a box: a row of links must not read
# as a row of buttons, because what they promise is different. It declares
# the `link` role, which is what a screen reader announces it by — the
# colour is not available to everyone, and on its own it says nothing.
#
# It is `text_link` and not `link` because `breadcrumb` below keeps a local
# of that name, and a local in this language is visible to what comes after
# it: defining `link` as a function meant the first breadcrumb drawn
# replaced it with a hash, and every later call said so.
def text_link(label, on_click, props = {})
  {
    "k": "text",
    "t": label,
    "s": {"fg": "accent.base", "cursor": "pointer"},
    "on": {"click": on_click},
    "p": {"role": "link", "label": label}.merge(props)
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

# The tick itself, without the row around it. It draws and nothing else: no
# handler, no key, no role. That is what lets it sit inside a node whose own
# role is a leaf — `option`, say — where a real `checkbox` would be dropped
# from the accessibility tree while still taking the click (06 §2 gives the
# nearest handler on the path the event, and that would be the mark, not the
# row). Whatever holds it says the state; this only shows it.
def check_mark(checked, mixed, disabled, size)
  box = checkbox_box_px(size)
  lit = checked || mixed
  {
    "k": "box",
    "s": {
      "width": box,
      "height": box,
      "shrink": 0,
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
  disabled = o["disabled"] == true
  mixed = o["indeterminate"] == true
  mark = check_mark(checked, mixed, disabled, size)
  # A checkbox with no label is a bare tick — a header's select-all, say, where
  # the count beside it is a node of its own and must not be swallowed into the
  # tick's name. Don't leave an empty text node for the gap to push away from.
  kids = [mark]
  kids = kids.concat([text(label, {
    "size": control_text_size(size),
    "fg": disabled ? "text.disabled" : (checked ? "text.muted" : "text.default")
  })]) unless label.blank?
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
    "c": kids
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

# A radio is a checkbox that cannot be unticked and knows about its
# neighbours: choosing one is choosing away from the others. So the *group*
# owns the value — there is no `checked` argument that a caller could set on
# two of them at once — and each button carries only what it stands for, in
# `props`, which is what comes back as `params["props"]`.
#
# The mark is a ring with a dot in it rather than a box with a tick, because
# the shape is the only thing that says "one of these" before the pointer is
# anywhere near it.
def radio(label, selected, on_pick, props, o = {})
  size = o["size"] ?? "md"
  box = checkbox_box_px(size)
  disabled = o["disabled"] == true
  mark = {
    "k": "box",
    "s": {
      "width": box,
      "height": box,
      "radius": 4,
      "border": 2,
      "border_color": disabled ? "border.subtle" : (selected ? "accent.base" : "border.strong"),
      "bg": "none",
      "display": "row",
      "justify": "center",
      "align": "center",
      "transition": "fast"
    },
    "c": selected ? [node(
      "box",
      {
        "width": box - 8,
        "height": box - 8,
        "radius": 4,
        "bg": disabled ? "text.disabled" : "accent.base"
      },
      []
    )] : []
  }
  control({
    "key": o["key"] ?? ("rd:" + (props["group"] ?? "").to_s + ":" + (props["value"] ?? label).to_s),
    "size": size,
    "shape": {"justify": "start", "border": 0, "radius": 1, "min_width": 0, "pad": [1, 2, 1, 2]},
    "on": {"click": on_pick},
    "props": props,
    "disabled": disabled,
    "a11y": {
      "role": "radio",
      "checked": selected,
      "label": o["name"] ?? label
    },
    "c": [
      mark,
      text(label, {
        "size": control_text_size(size),
        "fg": disabled ? "text.disabled" : "text.default"
      })
    ]
  })
end

# The group is what a screen reader is told about — `radio_group` is a role
# the client knows — and it is what makes the keys unique: every button in a
# group takes the group's name, so two groups of "Yes"/"No" on one page do
# not restyle each other. `o["direction"]` is `"column"` unless a row is
# asked for; three short options read better on one line.
def radio_group(options, value, on_pick, o = {})
  name = o["name"] ?? "radio"
  size = o["size"] ?? "md"
  disabled = o["disabled"] == true
  buttons = options.map(fn(opt) {
    radio(opt, opt == value, on_pick, {"value": opt, "group": name}, {"size": size, "disabled": disabled})
  })
  group = (o["direction"] ?? "column") == "row" ? row(
    {"gap": 4, "align": "center", "wrap": "wrap"},
    buttons
  ) : column({"gap": 1}, buttons)
  group["p"] = {"role": "radio_group", "label": o["label"] ?? name}
  group
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
  can_edit = grid_col_editable(col)
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
  if editing == true && can_edit == true && choices.length() > 0
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
  if editing == true && can_edit == true && choices.length() == 0
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
  cursor = "text" if can_edit == true && choices.length() == 0
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

# A sidebar: one row per destination, and the one you are on marked.
#
# `icons` names an icon per link — `{"Orders": "doc"}` — and a link without
# one still gets the width, so the labels of a part-iconed menu line up with
# each other instead of stepping in and out.
#
# The row you are on is marked three ways, because one is not enough: a
# filled ground for the eye scanning the column, a weight and a colour for
# the eye reading it, and a bar down the leading edge that survives both a
# colour blindness and the high-contrast palette. The bar is a box inside
# the row rather than a border on it, so nothing shifts by two pixels as the
# selection moves.
def sidebar(links, active, on_go, icons = {})
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
      here = l == active
      mark = {"k": "box", "s": {
        "width": 2,
        "height": 16,
        "radius": 1,
        "shrink": 0,
        "bg": here ? "accent.base" : "none"
      }}
      glyph = icon(icons[l] ?? "dot", {
        "width": 16,
        "height": 16,
        "shrink": 0,
        "fg": here ? "accent.base" : "text.muted"
      })
      {
        "k": "box",
        "s": {
          "display": "row",
          "align": "center",
          "gap": 2,
          "pad": [2, 3, 2, 2],
          "radius": 2,
          "bg": here ? "surface.sunken" : "none",
          "cursor": "pointer",
          "transition": "fast"
        },
        "on": {"click": on_go},
        "p": {"path": l, "label": l, "current": here},
        "c": [mark, glyph, text(l, here ? {
          "weight": "semibold",
          "fg": "accent.base"
        } : {"fg": "text.default"})]
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
def select(options, value, open, on_toggle, on_pick, o = {})
  select_sized(options, value, open, on_toggle, on_pick, 160, false, o)
end

def select_sized(options, value, open, on_toggle, on_pick, min_width, grow, o = {})
  # The same surface as the box you type into, for the same reason: a select
  # the colour of the card it sits on reads as a label until it is clicked.
  # The hover is the neutral tone's, so a select and a button answer the
  # pointer with the same two colours.
  s = {
    "display": "row",
    "align": "center",
    "gap": 2,
    "pad": [2, 3, 2, 3],
    "min_width": min_width,
    "min_height": field_height(o),
    "border": 1,
    "border_color": "border.default",
    "radius": 2,
    "bg": "surface.sunken",
    "cursor": "pointer",
    "transition": "fast"
  }
  s["grow"] = 1 if grow
  anchor = {
    "k": "box",
    "key": "sel:" + on_toggle.to_s,
    "s": s,
    "on": stateful(s, TONES["neutral"], {"click": on_toggle}),
    "c": [text(value, {"grow": 1}), icon(
      "chevron_down",
      {"fg": "text.muted", "width": 14, "height": 14}
    )]
  }
  dropdown(anchor, options.map(fn(o) { select_option(o, o == value, on_pick, min_width) }), open, DROPDOWN_MAX_PX)
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

# How tall a list of options is allowed to get before it scrolls instead of
# running on. Eight rows or so: enough that a short list never scrolls, few
# enough that a long one does not bury the page it hangs over. A panel that
# is not a list of options — a calendar, say — passes no ceiling and is
# bounded by the window alone.
DROPDOWN_MAX_PX = 280

# A popover that opens under its anchor rather than over it.
# An open list floats: it is an `overlay`, so it paints in the top layer
# and no card or scroller clips it, and it is `absolute` in a `stack`, so
# it neither grows the box it hangs off nor pushes the page open. Where it
# lands is the client's business (04 §5): under the anchor when the window
# has room, over it when it has not, and never past an edge. The top
# margin is the gap it keeps.
#
# The content lives in a `scroll`, which is what makes a long panel usable.
# A scroll is as tall as its content when the room is indefinite and no
# taller than the room when it is not (04 §7), and the client measures a
# popover against the window (04 §5) — so three options still make a
# three-option panel, and a panel that would not fit the window scrolls
# inside it instead. `max_px` lowers that ceiling further, which a list of
# options wants and a calendar does not.
#
# Without the scroll a long panel simply grew past the bottom of the
# window. The client clamps it back inside, so the options that did not fit
# were unreachable, and the wheel over them found the page's scroller and
# moved the page behind instead.
def dropdown(anchor, content, open, max_px = 0)
  return anchor unless open

  pane = {"gap": 0}
  pane["max_height"] = max_px if max_px > 0
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
      "c": [scroll(pane, content)]
    }
  ])
end

# ---- Multi-selection --------------------------------------------------------
# A selection is `{"ids": [...], "all": Bool, "scope": Str}`, and the flag is
# the whole design:
#
#   all == false   `ids` are what is chosen.
#   all == true    everything is chosen *except* `ids`.
#
# The second reading is what makes "select all" free over a windowed list. A
# selection held as a list of ids would put ten thousand strings into the
# session state and re-serialise them on every event; held as one boolean plus
# the handful of rows someone unticked afterwards, it costs nothing and answers
# for rows the server has never sent. An application spends it the same way:
# `all: true` goes into the query as `NOT IN (ids)`, not as an enumeration.
#
# `selection_toggle` is one body with two meanings — add-or-remove in `ids` is
# "choose" under the first reading and "except" under the second — and that
# symmetry is the reason for the shape rather than a happy accident.
#
# `scope` is what stops the flag lying. "All" is always relative to the query
# that was on screen when it was clicked; select every unpaid order, clear the
# filter, and without a scope token "all" silently means every order there is.
# The caller puts whatever names its query in there and compares it.
#
# Every field is read with `??` and `== true`, never `||`: in Soli `false` and
# `0` are truthy, so `sel["all"] || false` is a bug that survives testing.
def selection(ids = [], scope = "")
  {"ids": ids, "all": false, "scope": scope}
end

def selection_scope(sel)
  (sel ?? {})["scope"] ?? ""
end

# Everything, as one boolean. The exception list starts empty.
def selection_all(sel)
  {"ids": [], "all": true, "scope": selection_scope(sel)}
end

def selection_none(sel)
  {"ids": [], "all": false, "scope": selection_scope(sel)}
end

# A selection is only meaningful for the query it was made in. Hand this the
# token naming the current query and it answers with the selection, or with an
# empty one when the query has moved on underneath it.
def selection_scoped(sel, scope)
  return selection([], scope) if sel.nil? || selection_scope(sel) != scope

  sel
end

def selection_ids_of(sel)
  (sel ?? {})["ids"] ?? []
end

def selection_all?(sel)
  (sel ?? {})["all"] == true
end

def selection_has?(sel, id)
  inside = selection_ids_of(sel).includes?(id)
  return !inside if selection_all?(sel)

  inside
end

# `concat` appends to the array it is called on and hands it back, so
# `ids.concat([id])` would grow the selection this one was derived from — the
# caller's, and anything else still holding that array. `kept` is fresh out of
# `filter`, so appending to it touches nobody, and the filter is doing double
# duty as the copy.
def selection_toggle(sel, id)
  ids = selection_ids_of(sel)
  kept = ids.filter(fn(x) { x != id })
  return {"ids": kept, "all": selection_all?(sel), "scope": selection_scope(sel)} if kept.length() < ids.length()

  {"ids": kept.concat([id]), "all": selection_all?(sel), "scope": selection_scope(sel)}
end

def selection_count(sel, total)
  held = selection_ids_of(sel).length()
  return held unless selection_all?(sel)
  return 0 if held > total

  total - held
end

def selection_empty?(sel, total)
  selection_count(sel, total) == 0
end

# What the header's tick should say: "none", "all", or "some" — which is the
# `"mixed"` third state of 03 §6.1, and which `checkbox` already draws as a
# minus when it is given `indeterminate`.
def selection_mark(sel, total)
  count = selection_count(sel, total)
  return "none" if count == 0
  return "all" if count >= total

  "some"
end

# `selection_has?` walks the id list, which is right for a list of twenty and
# wrong for a window of thirty rows re-asked on every scroll. Build the index
# once per render and read it per row.
#
# The index stores `true` and holds nothing else. A `false` entry would read
# correctly through `selection_in?` and still be counted by `.keys().length()`,
# so a count taken from the hash would start lying the first time someone
# ticked a row and unticked it again.
def selection_index(sel)
  index = {}
  for id in selection_ids_of(sel)
    index[id] = true
  end
  index
end

def selection_in?(index, sel, id)
  inside = index[id] == true
  return !inside if selection_all?(sel)

  inside
end

# The chosen ids, spelled out. Only for a list short enough that the server
# already holds every row — a windowed list must push `all` into its query
# instead, which is the whole point of the flag.
def selection_ids(sel, every)
  return selection_ids_of(sel) unless selection_all?(sel)

  every.filter(fn(id) { !selection_ids_of(sel).includes?(id) })
end

# Ten thousand reads as 10 000, not as 10000. A count this widget shows sits
# beside figures the application wrote itself, and one of them grouped and the
# other not looks like a bug rather than a choice.
def grouped_number(n)
  gn_chars = str(n).chars()
  gn_out = ""
  gn_i = 0
  while gn_i < gn_chars.length()
    gn_left = gn_chars.length() - gn_i
    gn_out = gn_out + " " if gn_i > 0 && gn_left % 3 == 0
    gn_out = gn_out + gn_chars[gn_i]
    gn_i = gn_i + 1
  end
  gn_out
end

# What the header says. Not "3 selected" on its own: the total is what makes
# "select all" mean anything, and over a windowed list it is the only number
# saying how much is out there at all.
def multi_select_count_text(sel, total)
  msc_count = selection_count(sel, total)
  return "None selected" if msc_count == 0
  return "All " + grouped_number(total) + " selected" if msc_count >= total

  grouped_number(msc_count) + " of " + grouped_number(total) + " selected"
end

# The tri-state tick, the count, and a way back to nothing.
#
# The tick is a real `checkbox` — it is a control in its own right, it is not
# inside a row, and `indeterminate` already gives it the `"mixed"` third state
# of 03 §6.1. Its visible label is empty and its accessible name comes from
# `name`, because the count beside it is a separate node and must not become
# part of the tick's name.
#
# The count keeps its node whatever it says. A live region that is removed and
# re-added is an insertion rather than a change, and an insertion is not
# reliably announced — so the node is always there and only its text moves.
def multi_select_header(sel, total, o)
  msh_mark = selection_mark(sel, total)
  msh_parts = []
  msh_parts = msh_parts.concat([checkbox("", msh_mark == "all", o["on_all"], {}, {
    "key": o["key"] + ":all",
    "size": o["size"] ?? "sm",
    "indeterminate": msh_mark == "some",
    "name": o["all_label"] ?? "Select every row"
  })]) if o["on_all"].present?
  msh_parts = msh_parts.concat([{
    "k": "box",
    "s": {"display": "row", "align": "center", "grow": 1},
    "p": {"role": "status", "live": "polite"},
    "c": [muted(multi_select_count_text(sel, total))]
  }])
  msh_parts = msh_parts.concat([text_link("Clear", o["on_clear"], {})]) if o["on_clear"].present? && msh_mark != "none"
  row({"gap": 2, "align": "center", "width": "100%"}, msh_parts)
end

# One row, and it is one `control` with one click handler.
#
# The tick inside it is `check_mark` and not `checkbox`, deliberately. The
# row's role is `option`, which 03 §6 rule 1 makes a leaf: a real checkbox in
# there would be dropped from the accessibility tree while hit-testing still
# handed it the click (06 §2 gives the event to the nearest handler on the
# path, which would be the mark and not the row). One handler per row is also
# one Tab stop per row, which is what a list forty rows long wants.
#
# Selected is said by a 3 px rule down the left edge and by the tick, and not
# by a background. `control` folds `selected` into the *resting* colours before
# the hover delta is taken, and the quiet tone's selected wash and its hover
# wash are both `surface.sunken` — so a selected row under the pointer would
# lose the only thing saying it was selected. A border width reserved at rest
# and merely coloured when chosen is the same trick `button_variant` uses
# below: layout has nothing to do, and hover has nothing to take away. So
# `selected` is not handed to `control` at all.
#
# `a11y.selected` is, and it is present even when false. `control`'s own
# `selected` is purely visual — `a11y_props` promotes only `disabled`,
# `loading` and `read_only` — and to an assistive technology an absent
# `selected` means "not selectable" where `false` means "selectable, not
# selected". Leave it off the unticked rows and the list reads as though only
# the chosen ones were ever there.
def multi_select_row(item, chosen, on_toggle, pos, total, o)
  msr_size = o["size"] ?? "sm"
  msr_on = chosen == true
  msr_off = item["disabled"] == true
  msr_body = [check_mark(msr_on, false, msr_off, msr_size)]
  if o["row"].nil?
    msr_body = msr_body.concat([text(item["label"], {
      "size": control_text_size(msr_size),
      "grow": 1,
      "weight": msr_on ? "semibold" : "regular",
      "fg": msr_off ? "text.disabled" : "text.default"
    })])
  else
    msr_make = o["row"]
    msr_body = msr_body.concat(msr_make(item, msr_on))
  end
  # `id` is what comes back as `params["props"]["id"]` and is what the
  # selection is keyed by. `row` is the absolute index a windowed list needs
  # (04 §7.1): a child without one is not laid out at all. Two numbers, two
  # jobs — and a selection keyed by the second would name a different record
  # the moment the list is sorted or filtered.
  msr_props = {"id": item["id"]}
  msr_props["row"] = item["row"] unless item["row"].nil?
  control({
    "key": o["key"] + ":row:" + item["id"].to_s,
    "size": msr_size,
    "tone": "quiet",
    "shape": {
      "justify": "start",
      "align": "center",
      "width": "100%",
      "min_width": 0,
      "gap": 2,
      "radius": 1,
      "pad": [1, 2, 1, 2],
      "border": [0, 0, 0, 3],
      "border_color": msr_on ? "accent.base" : "none"
    }.merge(o["row_shape"] ?? {}),
    "on": {"click": on_toggle},
    "props": msr_props,
    "disabled": msr_off,
    "a11y": {
      "role": "option",
      "selected": msr_on,
      "label": item["name"] ?? item["label"],
      "pos_in_set": pos,
      "set_size": total
    },
    "c": msr_body
  })
end

# The container's own semantics. `list_box` is not a leaf role, so it keeps its
# rows; `multi_selectable` is not in the client's atom table and reaches no
# assistive technology today, but 03 §6.1 requires a client to ignore a prop it
# does not understand, so it costs nothing and is right the day the client
# grows it.
def multi_select_semantics(o)
  msm_props = {"role": "list_box", "orientation": "vertical", "multi_selectable": true}
  msm_props["label"] = o["label"] unless o["label"].blank?
  msm_props
end

# Header, rule, whatever the caller wants standing above the rows, then the
# rows. `head` is for a caption that belongs to the list and must not scroll
# with it — a row of column names, say: the same argument `data_grid` makes for
# keeping its header outside the scroller, and version 1 has no sticky.
def multi_select_shell(sel, total, o, body)
  mss_parts = [multi_select_header(sel, total, o), divider()]
  mss_parts = mss_parts.concat(o["head"]) unless o["head"].nil?
  mss_parts = mss_parts.concat([body])
  column({"gap": 0, "width": "100%"}, mss_parts)
end

# A list of rows, all of them present, each one tickable.
#
# `items` are `{"id", "label"}` hashes, optionally `"name"` (the accessible
# name, when the label alone would not do) and `"disabled"`. `o` carries:
#
#   key       required, and the prefix for every key inside. The arena's key
#             map is flat and first-wins, so two of these over the same ids
#             would otherwise make each other's rows unreachable.
#   label     the list's accessible name.
#   size      "sm" by default; a list is denser than a form.
#   total     when the caller knows of more rows than it passed.
#   on_all    the header's tri-state tick. Without it there is no header tick.
#   on_clear  a way back to nothing, shown only when something is chosen.
#   height    px; present means the rows scroll inside it.
#   empty     what stands there when there is nothing to choose from.
#   row       fn(item, chosen) -> children, for a row that is more than a
#             label: a bar, a badge, a second line.
#   row_shape extra resting style for every row; the caller wins. Pin a
#             `height` here when the rows are windowed, so what is measured is
#             what `heights` promised.
#   head      nodes between the rule and the rows, outside the scroller.
def multi_select_list(items, sel, on_toggle, o = {})
  throw "multi_select_list: o[\"key\"] names every node inside it" if o["key"].blank?

  msl_total = o["total"] ?? items.length()
  return multi_select_shell(sel, msl_total, o, column(
    {"pad": 6, "align": "center", "width": "100%"},
    [muted(o["empty"] ?? "Nothing to choose from")]
  )) if items.length() == 0

  msl_index = selection_index(sel)
  msl_rows = range(0, items.length()).map(fn(i) {
    multi_select_row(items[i], selection_in?(msl_index, sel, items[i]["id"]), on_toggle, i + 1, msl_total, o)
  })
  msl_body = scroll({"width": "100%", "gap": 0}, msl_rows)
  msl_body["s"]["height"] = o["height"] unless o["height"].nil?
  msl_body["p"] = multi_select_semantics(o)
  multi_select_shell(sel, msl_total, o, msl_body)
end

# The same widget over rows the server does not hold (04 §7.1). `make` is
# `fn(i)` returning the item for absolute row `i` — a pure function of the
# index, as `erp_product` already is — and only the rows in `window` are built.
#
# `o` additionally carries `count`'s companions: `heights`, `item_height` and
# `on_window`. Two things this must not get wrong, and neither says so:
#
#   `set_size` is `count` and never the window's length. 03 §6.1 is explicit
#   that it counts what virtualisation left out, and the client already honours
#   it — so an assistive technology says "12 of 10 000" only if we send the
#   10 000.
#
#   A row must not change height when it is ticked. Row tops come from
#   `heights`, but a row that is present is measured at its content size and
#   drawn at that top — so a row that grows when chosen overlaps the one below
#   it, with no clamp and no warning, and the scrollbar comes up short.
def multi_select_window(make, count, window, sel, on_toggle, o = {})
  throw "multi_select_window: o[\"key\"] names every node inside it" if o["key"].blank?

  msw_win = window ?? [0, 0]
  msw_first = msw_win[0] ?? 0
  msw_last = msw_win[1] ?? 0
  msw_last = count - 1 if msw_last > count - 1
  msw_index = selection_index(sel)
  msw_rows = []
  msw_rows = range(msw_first, msw_last + 1).map(fn(i) {
    msw_item = make(i)
    msw_item["row"] = i
    multi_select_row(msw_item, selection_in?(msw_index, sel, msw_item["id"]), on_toggle, i + 1, count, o)
  }) if msw_first <= msw_last
  msw_body = list_window(
    {"width": "100%"},
    o["item_height"] ?? 28,
    count,
    o["heights"],
    msw_rows,
    o["on_window"]
  )
  msw_body["s"]["height"] = o["height"] unless o["height"].nil?
  # `list_window` writes `p` whole, so this merges. Assigning would drop
  # `count`, `heights` and `item_height`, and the list would quietly become an
  # ordinary virtualised one of the thirty rows it happens to be holding.
  msw_body["p"] = msw_body["p"].merge(multi_select_semantics(o))
  multi_select_shell(sel, count, o, msw_body)
end

# What the anchor shows: a chip per chosen option, capped, then how many more.
#
# Each chip's × sends the same `on_pick` with the same id, because removing a
# chip *is* toggling that option off — one handler, one meaning, and the panel
# never has to be open for it. `chip_remove` keys itself from `props["id"]`, so
# every chip must carry one: hand them all the same props and they share a key,
# and the arena's key map is flat and first-wins, so all but the first become
# unreachable.
def multi_select_chips(options, sel, on_pick, o)
  msp_chosen = options.filter(fn(opt) { selection_has?(sel, opt["id"]) })
  return text(o["placeholder"] ?? "Choose…", {"fg": "text.muted", "grow": 1}) if msp_chosen.length() == 0

  msp_max = o["max_chips"] ?? 3
  msp_show = msp_chosen.length() > msp_max ? msp_max : msp_chosen.length()
  msp_parts = range(0, msp_show).map(fn(i) {
    chip(msp_chosen[i]["label"], on_pick, {"id": msp_chosen[i]["id"]})
  })
  msp_parts = msp_parts.concat([
    muted("+" + str(msp_chosen.length() - msp_show))
  ]) if msp_chosen.length() > msp_show
  # Their own row, with their own gap: the anchor's is 0, so that the sizer
  # text beside them costs nothing, and the chips would otherwise touch.
  row({"gap": 1, "align": "center", "grow": 1, "shrink": 1, "min_width": 0}, msp_parts)
end

# The field itself. `combo_box` is not one of §6 rule 1's leaf roles, so the
# chips and their × stay visible to an assistive technology rather than being
# flattened into the anchor's name.
def multi_select_anchor(options, sel, open, on_toggle, on_pick, o)
  # The same shell as `select_sized`'s anchor, down to the padding: these two
  # stand side by side in a form and a field four pixels shorter than the one
  # beside it reads as a mistake.
  msa_style = {
    "display": "row",
    "align": "center",
    "gap": 0,
    "pad": [2, 3, 2, 3],
    "min_width": o["min_width"] ?? 200,
    "min_height": field_height(o),
    "border": 1,
    "border_color": "border.default",
    "radius": 2,
    "bg": "surface.sunken",
    "cursor": "pointer",
    "transition": "fast"
  }
  msa_style["grow"] = 1 if o["grow"] == true
  # A select is as tall as the line of text in it. This one holds chips, which
  # are shorter, so it would stand two pixels under its neighbour when
  # something is chosen and four when nothing is — and it would *change* height
  # as the first chip appeared. An empty text node has no width and a full
  # line's height, so it fixes the box to a select's without pinning a pixel
  # count that the viewer's font scale would then be wrong about. The gap is 0
  # for its sake: a zero-width child still takes one, and the chips would sit a
  # gap further in than the select's value beside them.
  msa_kids = [
    text("", {}),
    multi_select_chips(options, sel, on_pick, o),
    icon("chevron_down", {"fg": "text.muted", "width": 14, "height": 14, "margin": [0, 0, 0, 2]})
  ]
  {
    "k": "box",
    "key": o["key"] + ":anchor",
    "s": msa_style,
    "p": {"role": "combo_box", "expanded": open == true, "label": o["label"] ?? "Choose"},
    "on": stateful(msa_style, TONES["neutral"], {"click": on_toggle}),
    "c": msa_kids
  }
end

# Several of something, chosen in a panel, shown as chips.
#
# The panel stays open as options are ticked. That is the one behavioural
# difference from `select`, and it is not in the widget: the caller's `on_pick`
# toggles the selection and leaves `open` alone, where `select`'s closes it.
#
# Its rows are the same `multi_select_row` the listbox uses — that is the whole
# of what the two widgets share, and it is enough that a tick means the same
# thing in both.
#
# `options` are `{"id", "label"}` hashes. Do not hand it thousands: a panel is
# capped at `DROPDOWN_MAX_PX` and a virtualised one inside an overlay would
# need its own window and a scroll to restore on reopen — which is a combo box
# with a search field in it, and a different widget.
def multi_select(options, sel, open, on_toggle, on_pick, o = {})
  throw "multi_select: o[\"key\"] names every node inside it" if o["key"].blank?

  msd_anchor = multi_select_anchor(options, sel, open, on_toggle, on_pick, o)
  return msd_anchor unless open == true

  # A row is `width: 100%` in a list, and inside an absolutely positioned
  # overlay that resolves against the window rather than against the panel —
  # so a three-option panel came out eleven hundred pixels wide. In here the
  # rows ask for the anchor's width and the panel takes its size from them,
  # which is what lets the client place it under the anchor at all (04 §5).
  msd_opts = o.merge({
    "row_shape": {"width": "auto", "min_width": o["min_width"] ?? 200}.merge(o["row_shape"] ?? {})
  })
  msd_index = selection_index(sel)
  msd_rows = range(0, options.length()).map(fn(i) {
    msd_on = selection_in?(msd_index, sel, options[i]["id"])
    multi_select_row(options[i], msd_on, on_pick, i + 1, options.length(), msd_opts)
  })
  msd_panel = column({"gap": 0}, msd_rows)
  msd_panel["p"] = multi_select_semantics(o)
  dropdown(msd_anchor, [msd_panel], true, DROPDOWN_MAX_PX)
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

# Two selects, twenty-four hours and sixty minutes, so the clock cannot hold
# anything but HH:MM — there is no free text to parse and no "25:61" to
# reject. Both selects are the server's to open, like every other one.
def time_select(time, hour_open, min_open, on_hour_toggle, on_min_toggle, on_hour, on_min)
  bits = (time ?? "00:00").split(":")
  hour = bits[0]
  minute = "00"
  minute = bits[1] if bits.length() > 1
  hours = range(0, 24).map(fn(h) { two_digits(h) })
  minutes = range(0, 60).map(fn(m) { two_digits(m) })
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
  clock_column = column(
    {"gap": 1, "width": "100%"},
    [
      muted("Time"),
      time_select(time, hour_open, min_open, on_hour_toggle, on_min_toggle, on_hour, on_min)
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

# ---- Fields ----------------------------------------------------------------
#
# `field` above is a label over an input, which is all a text field ever
# needed. A *typed* field is the same three parts — a label, a control, and a
# line underneath — with the type choosing the control and judging what ends
# up in it.
#
# The client has no types. An `input` is an `input`, and 03 §3 is not growing
# an `email` kind so that a phone can pick a keyboard; the type lives here, on
# the server, which is where the value was going anyway. What that costs is
# the keyboard hint. What it buys is that "valid" means whatever this
# application means by it, written in Soli, next to the handler that stores
# the value — and that a field can be told it is wrong by something no
# client-side type could know, like a mailer that bounced.
#
# A value is judged when it arrives, and `change` arrives on blur or on Enter
# (never per keystroke), so a field is never red while it is still being typed
# into. An empty field is not wrong, it is empty: `o["required"]` is what says
# otherwise.
#
# Every field below takes the same options:
#
#   hint       a muted line under the control, while there is nothing wrong
#   error      what is wrong with this value, from the server. Shown instead
#              of the hint, and colours the control. It wins over the type's
#              own verdict, because it knows more than a shape does
#   required   the field is required — said in its props, and complained
#              about once `submitted` says the person has had their turn
#   submitted  the form has been sent: empty required fields may now speak
#   invalid    the wrong look, with no message of the caller's own
#   complaint  what to say instead of the type's own sentence
#   width      the control's width; "100%" by default
#   name       what the field is called to a screen reader, if not `label`

FIELD_DIGITS = "0123456789"

# One local part, one "@", one domain with a dot in it, and nothing blank on
# either side of a separator. That is the whole of what a field can honestly
# check: the only test of an address is a message sent to it, and a pattern
# that claims more turns real addresses away — quoted local parts,
# plus-addressing and IDN domains are all legal, and none of them are this
# function's business.
def email_valid?(value)
  address = (value ?? "").strip()
  return false if address == "" || address.includes?(" ")

  halves = address.split("@")
  return false unless halves.length() == 2
  return false if halves[0] == ""

  labels = halves[1].split(".")
  return false if labels.length() < 2

  labels.filter(fn(part) { part == "" }).length() == 0
end

# An optional sign, digits, and at most one point. No exponent and no
# thousands separator: a field that takes "1e3" has to explain itself, and a
# field that takes "1,000" has to know whose comma it is.
def number_valid?(value)
  glyphs = (value ?? "").strip().chars()
  span = glyphs.length()
  return false if span == 0

  i = ["-", "+"].includes?(glyphs[0]) ? 1 : 0
  digits = 0
  points = 0
  while i < span
    glyph = glyphs[i]
    if glyph == "."
      points = points + 1
    elsif FIELD_DIGITS.includes?(glyph)
      digits = digits + 1
    else
      return false
    end
    i = i + 1
  end
  digits > 0 && points < 2
end

# A number, and inside the bounds the caller gave — either of which may be
# absent, which is what an options hash hands over.
def number_within?(value, min, max)
  return false unless number_valid?(value)

  n = float((value ?? "").strip())
  return false unless min.nil? || n >= float(str(min))
  return false unless max.nil? || n <= float(str(max))
  true
end

# An ISO day, "YYYY-MM-DD" — the shape the calendar engine speaks, and the one
# that sorts correctly as a string.
def iso_day?(value)
  day = (value ?? "").strip()
  bits = day.split("-")
  return false unless bits.length() == 3
  return false unless bits[0].chars().length() == 4 && bits[1].chars().length() == 2 && bits[2].chars().length() == 2

  bits.filter(fn(part) { !number_valid?(part) }).length() == 0
end

# What the − and + buttons mean, for the handler that owns the value. The
# clamp is the one `number_within?` judges by, so a stepper cannot walk a
# field into an error nobody typed; an unreadable value steps from the floor,
# because "" + 1 has to be something.
def number_stepped(value, delta, o)
  step = float(str(o["step"] ?? 1))
  at = number_valid?(value) ? float((value ?? "").strip()) : float(str(o["min"] ?? 0))
  n = at + step * delta
  n = float(str(o["min"])) if !o["min"].nil? && n < float(str(o["min"]))
  n = float(str(o["max"])) if !o["max"].nil? && n > float(str(o["max"]))
  number_text(n)
end

# A number as a field holds it: "3", not "3.0", unless there is a fraction to
# keep.
def number_text(n)
  whole = int(n)
  n == float(str(whole)) ? str(whole) : str(n)
end

# What is wrong with this value, as a sentence, or "" when nothing is.
#
# An empty required field is not wrong yet. A form that opens already
# shouting at the person who has not typed in it is a form that has decided
# they were going to get it wrong; `o["submitted"]` is what says they have
# had their turn, and until then a required field says so in its props and
# stays quiet on the glass. A value that is *there* and malformed is a
# different matter — that one is judged the moment it arrives.
def field_error(value, judge, complaint, o)
  given = o["error"] ?? ""
  return given if given != ""

  said = (value ?? "").strip()
  return o["missing"] ?? "Required" if said == "" && o["required"] == true && o["submitted"] == true
  return "" if said == ""

  judge(said) ? "" : (o["complaint"] ?? complaint)
end

def field_bad(error, o)
  error != "" || o["invalid"] == true
end

# The line under a control: what is wrong with the value, or the hint that was
# there before anything was wrong with it. Never both — a field that explains
# itself twice is a field nobody reads.
def field_note(o)
  error = o["error"] ?? ""
  return [text(error, {"size": 1, "fg": "danger.base"})] if error != ""

  hint = o["hint"] ?? ""
  hint == "" ? [] : [muted(hint)]
end

def field_shell(label, control, o)
  head = (label ?? "") == "" ? [] : [muted(label)]
  column({"gap": 1, "width": "100%"}, head.concat([control]).concat(field_note(o)))
end

# The style every field control shares. It fills its column unless a width was
# asked for, and it goes danger when what is in it is wrong — a *colour*, over
# a border the resting style already reserved, so a field that turns red does
# not move the fields under it.
def field_style(bad, o)
  base = {"width": o["width"] ?? "100%"}
  base = base.merge({"border_color": "danger.base"}) if bad
  base.merge(o["style"] ?? {})
end

# What the field says about itself. The client reads `label`, `description`,
# `required` and `invalid` by name (03 §4), so a wrong value is announced as
# wrong rather than only painted that way, and the sentence a sighted person
# reads under the field is the one a screen reader is given.
def field_props(label, error, bad, o)
  props = {"label": o["name"] ?? label}
  note = error != "" ? error : (o["hint"] ?? "")
  props["description"] = note if note != ""
  props["invalid"] = true if bad
  props["required"] = true if o["required"] == true
  props
end

# A line of anything. It judges nothing on its own; `o["error"]` and
# `o["required"]` are the only ways it goes wrong.
def text_field(label, value, on_change, o = {})
  error = field_error(value, fn(said) { true }, "", o)
  bad = field_bad(error, o)
  field_shell(
    label,
    input(value, on_change, {
      "style": field_style(bad, o),
      "props": field_props(label, error, bad, o)
    }),
    o.merge({"error": error})
  )
end

def email_field(label, value, on_change, o = {})
  error = field_error(value, fn(said) { email_valid?(said) }, "That does not look like an email address", o)
  bad = field_bad(error, o)
  field_shell(
    label,
    input(value, on_change, {
      "style": field_style(bad, o),
      "props": field_props(label, error, bad, o)
    }),
    o.merge({"error": error})
  )
end

# `o["min"]`, `o["max"]` and `o["step"]` are the bounds and the stride;
# `o["on_step"]` adds the two buttons and names the event they send, with the
# direction in `params["props"]["delta"]`. The handler does the arithmetic —
# `number_stepped` is it — because the value is the server's, and a widget
# that stepped it locally would be guessing at what the server would have
# stored.
def number_field(label, value, on_change, o = {})
  error = field_error(value, fn(said) { number_within?(said, o["min"], o["max"]) }, number_complaint(o), o)
  bad = field_bad(error, o)
  step = o["on_step"] ?? ""
  props = field_props(label, error, bad, o)
  props["role"] = "spin_button" if step != ""
  props["value_now"] = value unless (value ?? "") == ""
  props["value_min"] = str(o["min"]) unless o["min"].nil?
  props["value_max"] = str(o["max"]) unless o["max"].nil?
  box = input(value, on_change, {
    "style": field_style(bad, o).merge(step == "" ? {} : {"width": "auto", "grow": 1}),
    "props": props
  })
  field_shell(
    label,
    step == "" ? box : row(
      {"gap": 2, "align": "center", "width": o["width"] ?? "100%"},
      [
        box,
        icon_button("−", step, {"delta": -1}, {"icon": "minus", "name": "Less", "size": "sm", "key": "nf-:" + label}),
        icon_button("+", step, {"delta": 1}, {"icon": "plus", "name": "More", "size": "sm", "key": "nf+:" + label})
      ]
    ),
    o.merge({"error": error})
  )
end

# "A number", or the bounds, because "invalid" tells nobody what to type
# instead.
def number_complaint(o)
  return "Between " + str(o["min"]) + " and " + str(o["max"]) unless o["min"].nil? || o["max"].nil?
  return str(o["min"]) + " or more" unless o["min"].nil?
  return str(o["max"]) + " or less" unless o["max"].nil?

  "Numbers only"
end

# The multi-line one. `o["rows"]` is the floor the empty box keeps.
def textarea_field(label, value, on_change, o = {})
  error = field_error(value, fn(said) { true }, "", o)
  bad = field_bad(error, o)
  field_shell(
    label,
    textarea(value, on_change, {
      "rows": o["rows"] ?? 3,
      "style": field_style(bad, o),
      "props": field_props(label, error, bad, o)
    }),
    o.merge({"error": error})
  )
end

# ---- Fields that open a calendar -------------------------------------------
#
# A value that is picked rather than typed. The panel is the same calendar
# `date_picker` draws; what changes is where it is. `dropdown` puts it in an
# `overlay`, absolutely placed against the anchor, so the card holding the
# form neither grows by a calendar's height when one opens nor clips one when
# the form scrolls — and where it actually lands (under the anchor when the
# window has room, over it when it has not, never past an edge) is the
# client's to decide, not the server's.
#
# The server owns `open`, exactly as it owns a select's. The anchor toggles
# it; what a picked day does to it is the handler's business, and the three
# fields disagree on purpose — a day closes a date field, and a range stays
# open until it has both of its ends.
#
# These take one options hash rather than eleven arguments:
#
#   label / hint / error / width / disabled   as every field above
#   value | start + finish   the ISO day, or the two ends of the range
#   time                     "HH:MM", for `datetime_field`
#   month                    the "YYYY-MM" the calendar is showing
#   open                     whether the panel is down
#   on_toggle                the anchor was clicked
#   on_pick                  a day, as params["props"]["date"]
#   on_nav                   a month, as params["props"]["delta"]
#   placeholder              what the anchor says while nothing is picked
#   on_hour_toggle / on_min_toggle / on_hour / on_min / hour_open / min_open
#                            the two clock selects, for `datetime_field`

# The floating panel's own width. A calendar is seven columns of a fixed day
# cell and an overlay has no parent to take a width from, so the panel states
# one rather than collapsing onto the widest thing inside it.
PICKER_PANEL_PX = 268

def picker_panel(children)
  column({"gap": 3, "width": PICKER_PANEL_PX}, children)
end

def picker_field(o, caption, empty, make)
  label = o["label"] ?? ""
  open = o["open"] == true
  bad = field_bad(o["error"] ?? "", o)
  anchor = control({
    "key": "pk:" + (o["key"] ?? label).to_s,
    "tone": "neutral",
    "shape": {
      "justify": "start",
      "gap": 2,
      "width": o["width"] ?? "100%",
      "min_height": field_height(o),
      # No `bg`: the neutral tone rests on `surface.sunken` and hovers to
      # `surface.raised`, which is what every other field does now. Naming
      # `raised` here made the resting state the hover state, so a date field
      # sat flat on its card and answered the pointer with nothing.
      "border_color": bad ? "danger.base" : "border.default"
    },
    "on": {"click": o["on_toggle"]},
    "disabled": o["disabled"] == true,
    "a11y": {
      "role": "combo_box",
      "label": label,
      "expanded": open
    },
    "c": [
      text(caption, {"grow": 1, "fg": empty ? "text.muted" : "text.default"}),
      icon("calendar", {"fg": "text.muted", "width": 16, "height": 16})
    ]
  })
  # `make` is a thunk, not a node: building a calendar for a panel nobody has
  # opened is a month of day cells thrown away on every render, and a `month`
  # the caller has not filled in yet is not an error until it is shown.
  field_shell(label, open ? dropdown(anchor, [make()], true) : anchor, o)
end

def date_field(o)
  day = o["value"] ?? ""
  picker_field(
    o,
    day.present? ? day : (o["placeholder"] ?? "Pick a day"),
    !day.present?,
    fn() { picker_panel([calendar(o["month"], day.present? ? [day] : [], "", "", o["on_pick"], o["on_nav"])]) }
  )
end

def datetime_field(o)
  day = o["value"] ?? ""
  time = o["time"] ?? "00:00"
  picker_field(
    o,
    day.present? ? day + " " + time : (o["placeholder"] ?? "Pick a day and a time"),
    !day.present?,
    fn() { picker_panel([
      calendar(o["month"], day.present? ? [day] : [], "", "", o["on_pick"], o["on_nav"]),
      column(
        {"gap": 1, "width": "100%"},
        [
          muted("Time"),
          time_select(
            time,
            o["hour_open"] == true,
            o["min_open"] == true,
            o["on_hour_toggle"],
            o["on_min_toggle"],
            o["on_hour"],
            o["on_min"]
          )
        ]
      )
    ]) }
  )
end

def date_range_field(o)
  start = o["start"] ?? ""
  finish = o["finish"] ?? ""
  ends = [start, finish].filter(fn(day) { day.present? })
  caption = finish.present? ? start + " → " + finish : (start.present? ? start + " → …" : (o["placeholder"] ?? "Pick two days"))
  picker_field(
    o,
    caption,
    ends.length() == 0,
    fn() { picker_panel([calendar(o["month"], ends, start, finish, o["on_pick"], o["on_nav"])]) }
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

# ---- Axes ------------------------------------------------------------------
#
# A chart without numbers on it asks the reader to take the shape on trust.
# The hairlines were already there; these put the readings they stand for
# beside them, and the categories under the marks, so "up and to the right"
# becomes "from four to nine".
#
# Four readings, because `chart_grid` draws four hairlines: the top of the
# scale, the bottom, and the two thirds between. They are rounded to whole
# numbers — a tick label is a landmark, not a measurement, and the chip under
# the pointer is where an exact figure belongs.

CHART_TICKS = 4

# `int` truncates towards zero, which rounds −20.5 to −20 and turns a
# symmetric axis into a crooked one: −20, −6, 7, 21 where the reader was
# promised the same reach either side. Round away from zero instead.
def chart_round(v)
  v >= 0 ? int(v + 0.5) : 0 - int(0 - v + 0.5)
end

# The `1.0` is load-bearing: `/` on two whole numbers is whole division, so a
# reach of 21 over four gaps stepped −21, −11, 0, 10, 21 — a symmetric axis
# that was a unit crooked on one side, from a truncation three operations
# before the rounding.
def chart_scale_ticks(low, high)
  range(0, CHART_TICKS).map(fn(i) { chart_round(high - (high - low) * i * 1.0 / (CHART_TICKS - 1)) })
end

# Wide enough for the longest of them. Text is measured by the client and the
# server never sees a glyph, so this is an estimate — seven pixels a character
# at the smallest step, plus a little air — and it is an estimate the plot can
# afford to have wrong by a pixel.
def chart_gutter(ticks)
  n = 1
  for v in ticks
    n = str(v).length() if str(v).length() > n
  end
  8 + n * 7
end

# The column of readings. Each sits in a band as deep as the gap between two
# hairlines with its text at the top, so a label's first line lands on the
# line it belongs to; the last has no band under it and simply ends there.
def chart_y_axis(ticks, h, gutter)
  band = (h - 8) / (CHART_TICKS - 1)
  column(
    {"width": gutter, "height": h, "pad": [0, 1, 0, 0]},
    range(0, CHART_TICKS).map(fn(i) {
      {
        "k": "box",
        "s": {
          "display": "row",
          "justify": "end",
          "align": "start",
          "width": "100%",
          "height": i < CHART_TICKS - 1 ? band : "auto",
          "shrink": 0
        },
        "c": [text(str(ticks[i]), {"size": 0, "fg": "text.muted"})]
      }
    })
  )
end

# The categories, one cell a band, centred under the marks the bands cover.
# For a bar chart that is exactly where the bar is; `chart_x_axis_points` is
# the other case.
def chart_x_axis_bands(labels, spans)
  row(
    {"gap": 0},
    range(0, spans.length()).map(fn(i) {
      {
        "k": "box",
        "s": {"display": "row", "justify": "center", "width": spans[i], "shrink": 0, "overflow": "clip"},
        "c": [text(labels[i] ?? "", {"size": 0, "fg": "text.muted", "clamp": 1})]
      }
    })
  )
end

# A line's marks sit on the edges of the plot rather than in the middle of a
# slot, so its labels are spread between the ends instead of centred in cells:
# the first hugs the left edge, the last the right, and the rest fall where
# the marks do.
def chart_x_axis_points(labels, w)
  row(
    {"gap": 0, "justify": "between", "width": int(w - 8)},
    labels.map(fn(l) { text(l, {"size": 0, "fg": "text.muted"}) })
  )
end

# Readings along the bottom of a chart whose rows are the categories — a
# ranked bar, a Gantt, a dumbbell. `chart_grid_v` puts its hairlines at the
# same divisions, so the labels land on them.
def chart_value_axis(low, high, w, divisions)
  row(
    {"gap": 0, "justify": "between", "width": int(w - 8)},
    range(0, divisions + 1).map(fn(i) {
      text(str(chart_round(low + (high - low) * i * 1.0 / divisions)), {"size": 0, "fg": "text.muted"})
    })
  )
end

# The plot with its two axes around it: readings down the left, categories
# along the bottom. The height the caller gave is the whole thing, so the
# plot is that less the row of labels — `chart_plot_h` is what a builder
# scales its marks into, and `chart_plot_w` what it draws them across.

CHART_AXIS_H = 15

def chart_plot_h(h)
  h - CHART_AXIS_H
end

def chart_framed(ticks, x_axis, w, h, gutter, layers)
  row(
    {"gap": 0, "align": "start"},
    [
      chart_y_axis(ticks, h, gutter),
      column({"gap": 0, "align": "center", "width": w}, [layers, x_axis])
    ]
  )
end

# The labels a chart falls back on when the caller passed none: the ordinal of
# each mark, which is at least a count.
def chart_x_labels(labels, count)
  return labels if labels.length() >= count

  range(0, count).map(fn(i) { str(i + 1) })
end

def flatten_points(points)
  flat = []
  for p in points
    flat = flat.concat(p)
  end
  flat
end

# The five roles a chart spends, in order and never cycled. Past the fifth
# there is no sixth hue to reach for — a generated one is indistinguishable
# from one already here to a reader with a colour vision deficiency — so the
# tail goes to the de-emphasis ink and the chart is expected to name it
# "other", facet, or drop it.
def chart_role(i)
  roles = ["series.1", "series.2", "series.3", "series.4", "series.5"]
  if i >= roles.length()
    "text.muted"
  else
    roles[i]
  end
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

# The tooltip is an `overlay`, and that is the whole of the fix. A chip that
# lives inside its band is measured against the band, a band is thirty pixels
# wide, and anything longer than a bare number came back as four wrapped lines
# sitting on top of the marks it was describing. An absolute `overlay` child
# of a `stack` is a popover (04 §5): measured against the *window*, clipped by
# no ancestor. It takes the width its text asks for and hangs over the card's
# edge if that is what the reading needs.
#
# `position: pointer` is what puts it under the hand. An anchored popover
# hangs off its stack's first in-flow child, and a band is the full height of
# the plot, so a chip anchored to one sat at the foot of the chart wherever in
# the column the pointer actually was. A chunk cannot help: it has no access
# to the pointer, by design (07 §1). Only the client knows where the hand is,
# so the client places it — above the cursor and centred on it, dropping below
# only when there is no room over it (04 §5).
#
# One chip a chart, then, rather than one a band: since it no longer hangs off
# anything, the band's handler has only to write the reading into it.
#
# Hidden is `display: none` and not `opacity: 0`, because the top layer is
# hit-tested first and asks nothing about opacity: an invisible chip left in
# the layout would quietly eat every hover that landed under it. The cost is
# the fade, which is a fair price for the legend underneath still working.
def chart_tip_style(shown)
  shown ? {
    "bg": "surface.overlay",
    "fg": "text.default",
    "border": 1,
    "border_color": "border.subtle",
    "radius": 2,
    "shadow": 2,
    "pad": [1, 2, 1, 2],
    "position": "pointer",
    "margin": [2, 0, 0, 0],
    "z": 5
  } : {"display": "none", "position": "pointer"}
end

def chart_tip(id)
  {
    "k": "overlay",
    "key": "tip_" + id,
    "s": chart_tip_style(false),
    "c": [keyed("tt_" + id, text(" ", {"size": 0, "weight": "semibold"}))]
  }
end

# What a band says to show itself, and what it says to put the chip away.
def chart_tip_show(id, label)
  "tip_" + id + ".style = @shown; tt_" + id + ".text = " + chart_quoted(label)
end

def chart_tip_hide(id)
  "tip_" + id + ".style = @hidden"
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
  {
    "k": "box",
    "s": {"width": width, "height": "100%"},
    "on": {
      "pointer_enter": {
        "local": wash_key + ".style = @lit; " + chart_tip_show(id, label),
        "styles": {"lit": chart_wash_style(width, true), "shown": chart_tip_style(true)}
      },
      "pointer_leave": {
        "local": wash_key + ".style = @rest; " + chart_tip_hide(id),
        "styles": {"rest": chart_wash_style(width, false), "hidden": chart_tip_style(false)}
      }
    }
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
      row({"width": int(w - 8), "height": int(h - 8)}, bands),
      chart_tip(id)
    ]
  )
end

def chart_labels(values)
  values.map(fn(v) { str(v) })
end

# The chip a single-series mark shows: the category and the reading, because
# the chip is now the only place an exact figure lives.
def chart_point_chips(names, values)
  range(0, values.length()).map(fn(i) { (names[i] ?? str(i + 1)) + " · " + str(values[i]) })
end

def chart_line(id, values, w, h, labels = [])
  top = chart_max(values)
  ticks = chart_scale_ticks(0, top)
  gutter = chart_gutter(ticks)
  pw = w - gutter
  ph = chart_plot_h(h)
  names = chart_x_labels(labels, values.length())
  points = chart_points(values, pw, ph)
  line = [0, "series.1", 2].concat(flatten_points(points))
  dots = points.map(fn(p) { [
    3,
    "series.1",
    p[0],
    p[1],
    3
  ] })
  drawing = canvas(pw, ph, chart_grid(pw, ph).concat([line]).concat(dots))
  spans = chart_spans(points.map(fn(p) { p[0] }), pw)
  layers = chart_layers(id, spans, chart_point_chips(names, values), pw, ph, drawing)
  chart_framed(ticks, chart_x_axis_points(names, pw), pw, ph, gutter, layers)
end

def chart_area(id, values, w, h, labels = [])
  top = chart_max(values)
  ticks = chart_scale_ticks(0, top)
  gutter = chart_gutter(ticks)
  pw = w - gutter
  ph = chart_plot_h(h)
  names = chart_x_labels(labels, values.length())
  points = chart_points(values, pw, ph)
  flat = flatten_points(points)
  # The wash under the line is a border role rather than a tinted status one:
  # `info.subtle` was doing the job, and borrowing a status colour for a chart
  # is the thing the series roles exist to stop. `border.subtle` is the theme's
  # one step off the surface and it steps the same distance in both modes —
  # `surface.sunken` does not, and in dark mode it draws a hole rather than a
  # wash.
  area = [2, "border.subtle", ph - 4].concat(flat)
  line = [0, "series.1", 2].concat(flat)
  drawing = canvas(pw, ph, chart_grid(pw, ph).concat([area, line]))
  spans = chart_spans(points.map(fn(p) { p[0] }), pw)
  layers = chart_layers(id, spans, chart_point_chips(names, values), pw, ph, drawing)
  chart_framed(ticks, chart_x_axis_points(names, pw), pw, ph, gutter, layers)
end

def chart_bar(id, values, w, h, labels = [])
  top = chart_max(values)
  ticks = chart_scale_ticks(0, top)
  gutter = chart_gutter(ticks)
  pw = w - gutter
  ph = chart_plot_h(h)
  count = values.length()
  names = chart_x_labels(labels, count)
  slot = (pw - 8) / count
  bars = range(0, count).map(fn(i) {
    bar_h = (ph - 8) * values[i] / top;
    [1, "series.1", 4 + i * slot + slot / 8, ph - 4 - bar_h, slot - slot / 4, bar_h, 1]
  })
  drawing = canvas(pw, ph, chart_grid(pw, ph).concat(bars))
  centres = range(0, count).map(fn(i) { 4 + i * slot + slot / 2 })
  spans = chart_spans(centres, pw)
  layers = chart_layers(id, spans, chart_point_chips(names, values), pw, ph, drawing)
  chart_framed(ticks, chart_x_axis_bands(names, spans), pw, ph, gutter, layers)
end

# ---- Candlesticks ----------------------------------------------------------
#
# A price is four numbers a session, not one: `[open, high, low, close]`. The
# scale is the extent of the lows and highs rather than 0 to the top, because
# a price that moves 3 % about 400 is a flat smudge against a zero baseline.
def chart_extent(bars)
  return [0, 1] if bars.length() == 0

  low = bars[0][2]
  high = bars[0][1]
  for b in bars
    low = b[2] if b[2] < low
    high = b[1] if b[1] > high
  end
  high > low ? [low, high] : [low - 1, high + 1]
end

# Where a price sits in the plot: the top of the extent at the top of the box.
def chart_price_y(value, extent, h)
  4 + (h - 8) * (extent[1] - value) / (extent[1] - extent[0])
end

# One session is two rectangles: a hairline wick from high to low, and a body
# from open to close — `success` when the close is above the open, `danger`
# when it is under. A body that rounds to nothing is still drawn a pixel tall,
# so a session that opened and closed at the same price is a line, not a gap.
def chart_candle_paths(bars, w, h)
  extent = chart_extent(bars)
  count = bars.length()
  slot = (w - 8) / count
  body_w = slot * 5 / 8
  body_w = 3 if body_w < 3
  paths = []
  for i in range(0, count)
    b = bars[i]
    tone = b[3] >= b[0] ? "success.base" : "danger.base"
    cx = 4 + i * slot + slot / 2
    y_high = chart_price_y(b[1], extent, h)
    y_low = chart_price_y(b[2], extent, h)
    y_open = chart_price_y(b[0], extent, h)
    y_close = chart_price_y(b[3], extent, h)
    y_top = y_open < y_close ? y_open : y_close
    body_h = (y_open < y_close ? y_close : y_open) - y_top
    body_h = 1 if body_h < 1
    paths = paths.concat([[1, tone, cx, y_high, 1, y_low - y_high, 0]])
    paths = paths.concat([[1, tone, cx - body_w / 2, y_top, body_w, body_h, 1]])
  end
  paths
end

# The chip a session shows: its close, and what the session did to it.
def chart_candle_label(b)
  delta = b[3] - b[0]
  str(b[3]) + (delta >= 0 ? " +" + str(delta) : " " + str(delta))
end

# A candlestick chart. `bars` is a list of `[open, high, low, close]`, oldest
# first; the bands are the sessions, so it hovers like every other chart here.
def chart_candle(id, bars, w, h, labels = [])
  count = bars.length()
  # 03 §1.1: a chart with no series still shows its grid.
  return canvas(w, h, chart_grid(w, h)) if count == 0

  # The one scale here that does not start at zero: a price axis is an extent,
  # so its readings run from the lowest low to the highest high.
  extent = chart_extent(bars)
  ticks = chart_scale_ticks(extent[0], extent[1])
  gutter = chart_gutter(ticks)
  pw = w - gutter
  ph = chart_plot_h(h)
  names = chart_x_labels(labels, count)
  slot = (pw - 8) / count
  drawing = canvas(pw, ph, chart_grid(pw, ph).concat(chart_candle_paths(bars, pw, ph)))
  centres = range(0, count).map(fn(i) { 4 + i * slot + slot / 2 })
  chips = range(0, count).map(fn(i) { names[i] + " · " + chart_candle_label(bars[i]) })
  spans = chart_spans(centres, pw)
  layers = chart_layers(id, spans, chips, pw, ph, drawing)
  chart_framed(ticks, chart_x_axis_bands(names, spans), pw, ph, gutter, layers)
end

# ---- Gantt -----------------------------------------------------------------
#
# A task is `{"label": …, "start": …, "span": …}`, counted in whatever unit the
# caller counts in — days, here. The rows are the tasks and the axis is the
# time, so this is the one chart whose bands run across rather than down: a
# column of washes and a column of bands, not a row of each.

# Where the plan ends, which is where the axis ends.
def chart_gantt_end(tasks)
  last = 1
  for t in tasks
    last = t["start"] + t["span"] if t["start"] + t["span"] > last
  end
  last
end

# Hairlines down rather than across: on a time axis the divisions are dates.
def chart_grid_v(w, h, divisions)
  range(0, divisions + 1).map(fn(i) {
    x = 4 + (w - 8) * i / divisions;
    [0, "border.subtle", 1, x, 4, x, h - 4]
  })
end

# One rounded bar a row, in the four `base` roles, three fifths of the row
# tall. A task shorter than two pixels is still two pixels: a milestone is a
# task of no span, and it has to be visible.
def chart_gantt_bars(tasks, w, h)
  total = chart_gantt_end(tasks)
  count = tasks.length()
  row_h = (h - 8) / count
  bar_h = row_h * 3 / 5
  bar_h = 6 if bar_h < 6
  range(0, count).map(fn(i) {
    x0 = 4 + (w - 8) * tasks[i]["start"] / total;
    x1 = 4 + (w - 8) * (tasks[i]["start"] + tasks[i]["span"]) / total;
    x1 = x1 - x0 < 2 ? x0 + 2 : x1;
    [1, chart_role(i), x0, 4 + i * row_h + (row_h - bar_h) / 2, x1 - x0, bar_h, 1]
  })
end

# The row equivalents of `chart_wash_style` and `chart_band`: full width and a
# height, and the chip sits at the end of the row rather than over the mark.
def chart_row_wash_style(height, lit)
  {
    "width": "100%",
    "height": height,
    "radius": 2,
    "bg": lit ? "surface.sunken" : "none",
    "transition": "fast"
  }
end

def chart_row_band(id, i, height, label)
  wash_key = "cw_" + id + "_" + str(i)
  {
    "k": "box",
    "s": {"width": "100%", "height": height},
    "on": {
      "pointer_enter": {
        "local": wash_key + ".style = @lit; " + chart_tip_show(id, label),
        "styles": {"lit": chart_row_wash_style(height, true), "shown": chart_tip_style(true)}
      },
      "pointer_leave": {
        "local": wash_key + ".style = @rest; " + chart_tip_hide(id),
        "styles": {"rest": chart_row_wash_style(height, false), "hidden": chart_tip_style(false)}
      }
    }
  }
end

def chart_row_layers(id, heights, labels, w, h, drawing)
  count = heights.length()
  washes = range(0, count).map(fn(i) {
    keyed("cw_" + id + "_" + str(i), {"k": "box", "s": chart_row_wash_style(heights[i], false)})
  })
  bands = range(0, count).map(fn(i) { chart_row_band(id, i, heights[i], labels[i]) })
  stack(
    {"width": w, "height": h, "justify": "center", "align": "center"},
    [
      column({"width": int(w - 8), "height": int(h - 8)}, washes),
      drawing,
      column({"width": int(w - 8), "height": int(h - 8)}, bands),
      chart_tip(id)
    ]
  )
end

# The names beside the plot, one row each, ranged against it so that a short
# name and a long one both end where the bars begin. The box clips: a task
# called something long is cut off rather than widening the gutter the plot
# was measured against.
def chart_gantt_names(tasks, gutter, row_h, h)
  rows = range(0, tasks.length()).map(fn(i) {
    row(
      {"width": gutter, "height": row_h, "justify": "end", "align": "center", "overflow": "clip"},
      [text(tasks[i]["label"], {"size": 1, "fg": "text.muted"})]
    )
  })
  stack(
    {"width": gutter, "height": h, "justify": "center", "align": "center"},
    [column({"width": gutter, "height": int(h - 8)}, rows)]
  )
end

# A Gantt chart: the names in a gutter on the left, the bars in a plot on the
# right, and a band over each row saying when it runs.
def chart_gantt(id, tasks, w, h)
  count = tasks.length()
  return canvas(w, h, chart_grid_v(w, h, 4)) if count == 0

  gutter = w * 3 / 10
  gutter = 72 if gutter < 72
  gutter = 140 if gutter > 140
  plot_w = w - gutter - 8
  ph = chart_plot_h(h)
  row_h = (ph - 8) / count
  drawing = canvas(plot_w, ph, chart_grid_v(plot_w, ph, CHART_TICKS - 1).concat(chart_gantt_bars(tasks, plot_w, ph)))
  heights = range(0, count).map(fn(i) { row_h })
  labels = range(0, count).map(fn(i) {
    str(tasks[i]["start"]) + " → " + str(tasks[i]["start"] + tasks[i]["span"]) + " · " + str(tasks[i]["span"]) + " d"
  })
  row(
    {"gap": 2, "width": w, "align": "start"},
    [
      column({"gap": 0, "width": gutter}, [chart_gantt_names(tasks, gutter, row_h, ph), {"k": "box", "s": {"height": CHART_AXIS_H}}]),
      column(
        {"gap": 0, "align": "center", "width": plot_w},
        [
          chart_row_layers(id, heights, labels, plot_w, ph, drawing),
          chart_value_axis(0, chart_gantt_end(tasks), plot_w, CHART_TICKS - 1)
        ]
      )
    ]
  )
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

# ---- More than one series --------------------------------------------------
#
# Every chart above draws one series, and one series needs no legend: the
# title names it. Two or more is a different job — the reader has to tell
# them apart — and that job has a rule attached. Identity is never carried by
# colour alone: each of these forms ships a legend, and the ones with four
# series or fewer are direct-labelled on top of it, so a reader who cannot
# separate two of the hues still reads the chart correctly.
#
# The scale is shared. Two measures of different cell_px go in two charts, never
# in one with two axes: a second y-scale lets the author decide which line
# looks higher, which is not a decision a chart is allowed to make.

# The key to a multi-series chart: a swatch and a name per series, in the same
# fixed order the marks were drawn in.
def chart_legend(names)
  {
    "k": "box",
    "s": {"display": "row", "wrap": "wrap", "gap": 3, "width": "100%", "align": "center", "justify": "center"},
    "c": range(0, names.length()).map(fn(i) {
      row(
        {"gap": 1, "align": "center"},
        [
          {"k": "box", "s": {"width": 10, "height": 10, "radius": 4, "bg": chart_role(i), "shrink": 0}},
          text(names[i], {"size": 0, "fg": "text.muted"})
        ]
      )
    })
  }
end

# The top of a shared scale: the largest value anywhere in the set, so the
# series are read against each other and not each against itself.
def chart_rows_max(sets)
  top = 0
  for r in sets
    for v in r
      top = v if v > top
    end
  end
  top > 0 ? top : 1
end

# How many marks across — the longest series decides, and a short one simply
# stops early rather than being stretched to fit.
def chart_rows_count(sets)
  count = 0
  for r in sets
    count = r.length() if r.length() > count
  end
  count
end

# The chip over a band names every series at that x, because a tooltip that
# says only a number leaves the reader to guess which line they are on.
def chart_rows_labels(sets, names, count)
  range(0, count).map(fn(i) {
    parts = [];
    for j in range(0, sets.length())
      parts = parts.concat([(names[j] ?? ("Series " + str(j + 1))) + " " + str(sets[j][i] ?? 0)])
    end
    parts.join(" · ")
  })
end

# ---- Multi-line ------------------------------------------------------------
#
# Trend over time for several series. The markers are discs with a ring of the
# card's own surface behind them, which is what keeps two lines legible where
# they cross: the one drawn later interrupts the one drawn first instead of
# blending into it.
def chart_multi_line(id, sets, names, w, h, x_labels = [])
  top = chart_rows_max(sets)
  count = chart_rows_count(sets)
  ticks = chart_scale_ticks(0, top)
  gutter = chart_gutter(ticks)
  pw = w - gutter
  ph = chart_plot_h(h)
  cats = chart_x_labels(x_labels, count)
  step = count > 1 ? (pw - 8) / (count - 1) : 0
  paths = chart_grid(pw, ph)
  centres = range(0, count).map(fn(i) { 4 + i * step })
  for j in range(0, sets.length())
    vals = sets[j]
    points = range(0, vals.length()).map(fn(i) { [4 + i * step, 4 + (ph - 8) * (top - vals[i]) / top] })
    paths = paths.concat([[0, chart_role(j), 2].concat(flatten_points(points))])
    for p in points
      paths = paths.concat([[3, "surface.raised", p[0], p[1], 5]])
      paths = paths.concat([[3, chart_role(j), p[0], p[1], 3]])
    end
  end
  drawing = canvas(pw, ph, paths)
  parts = chart_rows_labels(sets, names, count)
  chips = range(0, count).map(fn(i) { cats[i] + " · " + parts[i] })
  layers = chart_layers(id, chart_spans(centres, pw), chips, pw, ph, drawing)
  column(
    {"gap": 2, "width": w},
    [chart_framed(ticks, chart_x_axis_points(cats, pw), pw, ph, gutter, layers), chart_legend(names)]
  )
end

# ---- Grouped bar -----------------------------------------------------------
#
# Compare the series within each category, and the categories with each other.
# A 2 px gap of the card's surface separates neighbouring bars, so the eye
# reads two marks rather than one two-tone mark.
def chart_grouped_bar(id, sets, names, labels, w, h)
  top = chart_rows_max(sets)
  count = chart_rows_count(sets)
  depth = sets.length()
  ticks = chart_scale_ticks(0, top)
  gutter = chart_gutter(ticks)
  pw = w - gutter
  ph = chart_plot_h(h)
  cats = chart_x_labels(labels, count)
  slot = (pw - 8) / count
  inner = slot * 3 / 4
  bar_w = (inner - (depth - 1) * 2) / depth
  bar_w = 2 if bar_w < 2
  bars = []
  for i in range(0, count)
    left = 4 + i * slot + (slot - inner) / 2;
    for j in range(0, depth)
      v = sets[j][i] ?? 0;
      bar_h = (ph - 8) * v / top;
      bar_h = 2 if bar_h < 2;
      bars = bars.concat([[1, chart_role(j), left + j * (bar_w + 2), ph - 4 - bar_h, bar_w, bar_h, 1]])
    end
  end
  drawing = canvas(pw, ph, chart_grid(pw, ph).concat(bars))
  centres = range(0, count).map(fn(i) { 4 + i * slot + slot / 2 })
  parts = chart_rows_labels(sets, names, count)
  chips = range(0, count).map(fn(i) { cats[i] + " · " + parts[i] })
  spans = chart_spans(centres, pw)
  layers = chart_layers(id, spans, chips, pw, ph, drawing)
  column(
    {"gap": 2, "width": w},
    [chart_framed(ticks, chart_x_axis_bands(cats, spans), pw, ph, gutter, layers), chart_legend(names)]
  )
end

# ---- Stacked bar -----------------------------------------------------------
#
# Part-to-whole, one column a category. The scale is the largest *total*, not
# the largest part, and the 2 px between segments is the same surface gap the
# grouped bars use — without it a stack of five reads as one striped block.
def chart_stack_totals(sets, count)
  range(0, count).map(fn(i) {
    total = 0;
    for r in sets
      total = total + (r[i] ?? 0)
    end
    total
  })
end

def chart_stacked_bar(id, sets, names, labels, w, h)
  count = chart_rows_count(sets)
  totals = chart_stack_totals(sets, count)
  top = chart_max(totals)
  ticks = chart_scale_ticks(0, top)
  gutter = chart_gutter(ticks)
  pw = w - gutter
  ph = chart_plot_h(h)
  cats = chart_x_labels(labels, count)
  slot = (pw - 8) / count
  bar_w = slot * 3 / 5
  bars = []
  for i in range(0, count)
    x = 4 + i * slot + (slot - bar_w) / 2;
    y = ph - 4;
    for j in range(0, sets.length())
      v = sets[j][i] ?? 0;
      seg = (ph - 8) * v / top;
      if seg > 0
        seg = 2 if seg < 2;
        bars = bars.concat([[1, chart_role(j), x, y - seg, bar_w, seg, 1]]);
        y = y - seg - 2
      end
    end
  end
  drawing = canvas(pw, ph, chart_grid(pw, ph).concat(bars))
  centres = range(0, count).map(fn(i) { 4 + i * slot + slot / 2 })
  parts = chart_rows_labels(sets, names, count)
  chips = range(0, count).map(fn(i) {
    cats[i] + " · " + str(totals[i]) + " (" + parts[i] + ")"
  })
  spans = chart_spans(centres, pw)
  layers = chart_layers(id, spans, chips, pw, ph, drawing)
  column(
    {"gap": 2, "width": w},
    [chart_framed(ticks, chart_x_axis_bands(cats, spans), pw, ph, gutter, layers), chart_legend(names)]
  )
end

# ---- Ranked bar ------------------------------------------------------------
#
# Magnitude, low to high, for categories whose names are words rather than
# dates. It goes horizontal for the same reason a table does: a name has room
# to be read, and a rotated label is a label nobody reads. One hue throughout,
# because the job here is cell_px and not identity — colouring each row
# differently would say the sets are different kinds of thing.
def chart_ranked_sorted(items)
  left = items
  out = []
  while left.length() > 0
    best = 0
    for i in range(1, left.length())
      best = i if left[i]["value"] > left[best]["value"]
    end
    out = out.concat([left[best]])
    rest = []
    for i in range(0, left.length())
      rest = rest.concat([left[i]]) if i != best
    end
    left = rest
  end
  out
end

def chart_ranked_bar(id, items, w, h)
  ranked = chart_ranked_sorted(items)
  count = ranked.length()
  return muted("nothing to rank") if count == 0

  gutter = w / 3
  gutter = 72 if gutter < 72
  gutter = 160 if gutter > 160
  plot_w = w - gutter - 8
  ph = chart_plot_h(h)
  row_h = (ph - 8) / count
  bar_h = row_h * 3 / 5
  bar_h = 6 if bar_h < 6
  top = chart_max(ranked.map(fn(it) { it["value"] }))
  bars = range(0, count).map(fn(i) {
    bw = (plot_w - 8) * ranked[i]["value"] / top;
    bw = 2 if bw < 2;
    [1, "series.1", 4, 4 + i * row_h + (row_h - bar_h) / 2, bw, bar_h, 1]
  })
  drawing = canvas(plot_w, ph, chart_grid_v(plot_w, ph, CHART_TICKS - 1).concat(bars))
  names = column(
    {"gap": 0, "width": gutter},
    range(0, count).map(fn(i) {
      {
        "k": "box",
        "s": {"display": "row", "align": "center", "justify": "end", "height": row_h, "width": "100%"},
        "c": [text(ranked[i]["label"], {"size": 0, "fg": "text.muted", "clamp": 1})]
      }
    })
  )
  heights = range(0, count).map(fn(i) { row_h })
  labels = range(0, count).map(fn(i) { ranked[i]["label"] + " · " + str(ranked[i]["value"]) })
  row(
    {"gap": 2, "width": w, "align": "start"},
    [
      column({"gap": 0, "width": gutter}, [names, {"k": "box", "s": {"height": CHART_AXIS_H}}]),
      column(
        {"gap": 0, "align": "center", "width": plot_w},
        [chart_row_layers(id, heights, labels, plot_w, ph, drawing), chart_value_axis(0, top, plot_w, CHART_TICKS - 1)]
      )
    ]
  )
end

# ---- Diverging bar ---------------------------------------------------------
#
# Distance from a baseline, which is a different question from cell_px: the
# reader wants the sign first and the amount second. Two hues either side of a
# neutral zero, never a ramp through a third.
#
# The obvious pair is red against green, and it is measurably the wrong one:
# this theme's `danger.base` and `success.base` sit ΔE 0.9 apart under
# simulated deuteranopia in light mode and 1.1 in dark — to something like one
# man in twelve they are the same colour, and a chart whose whole point is the
# sign then says nothing at all. Red against `series.1` measures 16.7 and
# 19.5 instead, so under-target keeps the red a reader expects and over-target
# takes the blue.
#
# Even that is not enough on its own. Every row carries its signed number in
# text beside the bar, because a sign is exactly the kind of thing a reader
# must never be asked to infer from a hue.
def chart_diverging_extent(items)
  reach = 0
  for it in items
    v = it["value"] < 0 ? 0 - it["value"] : it["value"]
    reach = v if v > reach
  end
  reach > 0 ? reach : 1
end

def chart_diverging_bar(id, items, w, h)
  count = items.length()
  return muted("nothing to compare") if count == 0

  gutter = w / 4
  gutter = 64 if gutter < 64
  gutter = 140 if gutter > 140
  plot_w = w - gutter - 48
  ph = chart_plot_h(h)
  row_h = (ph - 8) / count
  bar_h = row_h * 3 / 5
  bar_h = 6 if bar_h < 6
  reach = chart_diverging_extent(items)
  mid = plot_w / 2
  # Four divisions rather than three, so the middle hairline falls on the
  # zero rule and the axis has a 0 on it: an odd number of gaps puts the
  # centre of a diverging chart between two labels, which is the one place it
  # must not be.
  rules = chart_grid_v(plot_w, ph, 4)
  zero = [0, "border.strong", 1, mid, 4, mid, ph - 4]
  bars = range(0, count).map(fn(i) {
    v = items[i]["value"];
    span = (mid - 6) * (v < 0 ? 0 - v : v) / reach;
    span = 2 if span < 2;
    y = 4 + i * row_h + (row_h - bar_h) / 2;
    v < 0 ? [1, "danger.base", mid - span, y, span, bar_h, 1] : [1, "series.1", mid, y, span, bar_h, 1]
  })
  drawing = canvas(plot_w, ph, rules.concat([zero]).concat(bars))
  names = column(
    {"gap": 0, "width": gutter},
    range(0, count).map(fn(i) {
      {
        "k": "box",
        "s": {"display": "row", "align": "center", "justify": "end", "height": row_h, "width": "100%"},
        "c": [text(items[i]["label"], {"size": 0, "fg": "text.muted", "clamp": 1})]
      }
    })
  )
  heights = range(0, count).map(fn(i) { row_h })
  labels = range(0, count).map(fn(i) {
    items[i]["label"] + " " + chart_signed(items[i]["value"])
  })
  # The signed number, always on, past the end of the plot: a bar that reaches
  # left and a bar that reaches right put their readings in the same column,
  # so the numbers are a column a reader can run down.
  readings = column(
    {"gap": 0, "width": 38},
    range(0, count).map(fn(i) {
      {
        "k": "box",
        "s": {"display": "row", "align": "center", "justify": "start", "height": row_h, "width": "100%"},
        "c": [text(chart_signed(items[i]["value"]), {"size": 0, "weight": "semibold"})]
      }
    })
  )
  # The axis of a diverging chart is symmetric about its rule: the same reach
  # either side, so a bar left and a bar right of the same length are the same
  # number and the eye can be trusted with the comparison.
  plot = row(
    {"gap": 2, "width": w, "align": "start"},
    [
      column({"gap": 0, "width": gutter}, [names, {"k": "box", "s": {"height": CHART_AXIS_H}}]),
      column(
        {"gap": 0, "align": "center", "width": plot_w},
        [chart_row_layers(id, heights, labels, plot_w, ph, drawing), chart_value_axis(0 - reach, reach, plot_w, 4)]
      ),
      readings
    ]
  )
  column({"gap": 2, "width": w}, [plot, chart_legend_poles("Over target", "Under target")])
end

# A number that says which way it went before it says how far.
def chart_signed(v)
  v > 0 ? ("+" + str(v)) : str(v)
end

# The diverging chart's key: two swatches and two words, because the hues
# group the rows and only the words say what the grouping means.
def chart_legend_poles(over, under)
  row(
    {"gap": 3, "align": "center", "justify": "center", "width": "100%"},
    [
      row(
        {"gap": 1, "align": "center"},
        [{"k": "box", "s": {"width": 10, "height": 10, "radius": 4, "bg": "series.1", "shrink": 0}}, text(over, {"size": 0, "fg": "text.muted"})]
      ),
      row(
        {"gap": 1, "align": "center"},
        [{"k": "box", "s": {"width": 10, "height": 10, "radius": 4, "bg": "danger.base", "shrink": 0}}, text(under, {"size": 0, "fg": "text.muted"})]
      )
    ]
  )
end

# ---- Heatmap ---------------------------------------------------------------
#
# Magnitude over a grid — a week by an hour, a region by a month. This one is
# boxes rather than a canvas, and that is the point: a cell is a box, so the
# client hit-tests it for free and the sequential ramp is one role at varying
# opacity instead of a family of baked colours. More is denser, in one hue.
#
# The role is `accent.base` rather than `series.1`, and the reason is the dark
# mode. A sequential ramp is only as good as the distance between its ends,
# and the far end has to travel away from the surface in *both* modes: accent
# is the one role the theme moves for the mode, sitting around L 0.50 on white
# and L 0.70 on near-black, where `series.1` holds still at 0.50 and leaves a
# dark-mode ramp with barely a third of the range it has in light. The near
# end runs almost to nothing, because a cell with no load in it should read as
# an empty cell and not as a pale one.
def chart_heat_max(grid)
  top = 0
  for r in grid
    for v in r
      top = v if v > top
    end
  end
  top > 0 ? top : 1
end

def chart_heat_cell_style(alpha, cell_px)
  {
    "width": cell_px,
    "height": cell_px,
    "radius": 2,
    "bg": "accent.base",
    "opacity": alpha,
    "transition": "fast",
    "shrink": 0
  }
end

# A cell lights by going opaque, and carries its own chip the way a band does:
# the reading belongs under the cell the hand is on, not in a line at the foot
# of the grid where the reader has to look away from what they are pointing at.
def chart_heat_cell(id, r, c, value, alpha, cell_px, label)
  key = "hc_" + id + "_" + str(r) + "_" + str(c)
  {
    "k": "box",
    "key": key,
    "s": chart_heat_cell_style(alpha, cell_px),
    "on": {
      "pointer_enter": {
        "local": "self.style = @lit; " + chart_tip_show(id, label),
        "styles": {
          "lit": chart_heat_cell_style(255, cell_px).merge({"border": 1, "border_color": "text.default"}),
          "shown": chart_tip_style(true)
        }
      },
      "pointer_leave": {
        "local": "self.style = @rest; " + chart_tip_hide(id),
        "styles": {"rest": chart_heat_cell_style(alpha, cell_px), "hidden": chart_tip_style(false)}
      }
    }
  }
end

def chart_heatmap(id, grid, col_labels, row_labels, w, h)
  row_count = grid.length()
  return muted("nothing to map") if row_count == 0

  cols = grid[0].length()
  top = chart_heat_max(grid)
  # A cell is a square with a ceiling on it. Without the ceiling a five-column
  # grid in a wide card draws five slabs, and a slab reads as a bar chart lying
  # down: what the reader is meant to take in at a glance is the *pattern*, and
  # a pattern needs the whole grid inside one look.
  gutter = 72
  cell_px = (w - gutter - (cols - 1) * 2) / cols
  cell_px = 10 if cell_px < 10
  cell_px = 34 if cell_px > 34
  body = column(
    {"gap": 2, "align": "start"},
    range(0, row_count).map(fn(r) {
      row(
        {"gap": 2, "align": "center"},
        [{
          "k": "box",
          "s": {"display": "row", "justify": "end", "align": "center", "width": gutter - 2},
          "c": [text(row_labels[r] ?? str(r + 1), {"size": 0, "fg": "text.muted"})]
        }].concat(range(0, cols).map(fn(c) {
          v = grid[r][c] ?? 0;
          chart_heat_cell(
            id,
            r,
            c,
            v,
            int(24 + 231 * v / top),
            cell_px,
            (row_labels[r] ?? str(r + 1)) + " " + (col_labels[c] ?? str(c + 1)) + " · " + str(v)
          )
        }))
      )
    })
  )
  heads = row(
    {"gap": 2, "align": "center"},
    [{"k": "box", "s": {"width": gutter - 2}}].concat(range(0, cols).map(fn(c) {
      {
        "k": "box",
        "s": {"width": cell_px, "display": "row", "justify": "center", "shrink": 0},
        "c": [text(col_labels[c] ?? str(c + 1), {"size": 0, "fg": "text.muted"})]
      }
    }))
  )
  # The grid is a `stack` so the chip has one to live in: a popover is an
  # absolute overlay child of a stack, whatever it is that places it.
  stack({"width": w, "justify": "start", "align": "start"}, [column({"gap": 2, "width": w, "align": "start"}, [heads, body]), chart_tip(id)])
end

# ---- Dumbbell --------------------------------------------------------------
#
# Before and after, per item. Not two bars side by side: what the reader is
# after is the distance, and a line between two dots draws the distance
# itself. The "before" dot is the de-emphasis grey and the "after" dot is the
# series hue, because the story is where things ended up.

# The dumbbell's key. "Before" is the de-emphasis ink and "after" is the
# series hue, because the story is where things ended up — and both are named,
# because which end is which is not something a reader works out from a
# colour.
def chart_legend_pair(before, after)
  row(
    {"gap": 3, "align": "center", "justify": "center", "width": "100%"},
    [
      row(
        {"gap": 1, "align": "center"},
        [{"k": "box", "s": {"width": 10, "height": 10, "radius": 4, "bg": "text.muted", "shrink": 0}}, text(before, {"size": 0, "fg": "text.muted"})]
      ),
      row(
        {"gap": 1, "align": "center"},
        [{"k": "box", "s": {"width": 10, "height": 10, "radius": 4, "bg": "series.1", "shrink": 0}}, text(after, {"size": 0, "fg": "text.muted"})]
      )
    ]
  )
end

def chart_dumbbell(id, items, w, h)
  count = items.length()
  return muted("nothing to compare") if count == 0

  gutter = w / 4
  gutter = 64 if gutter < 64
  gutter = 140 if gutter > 140
  plot_w = w - gutter - 8
  ph = chart_plot_h(h)
  row_h = (ph - 8) / count
  top = 0
  for it in items
    top = it["from"] if it["from"] > top
    top = it["to"] if it["to"] > top
  end
  top = 1 if top <= 0
  # Hairlines down the plot, because a distance without a scale behind it is
  # only a picture of a distance: four divisions is enough for the eye to see
  # that one bar is twice another without the chart growing an axis.
  paths = chart_grid_v(plot_w, ph, CHART_TICKS - 1)
  for i in range(0, count)
    y = 4 + i * row_h + row_h / 2;
    x0 = 8 + (plot_w - 20) * items[i]["from"] / top;
    x1 = 8 + (plot_w - 20) * items[i]["to"] / top;
    paths = paths.concat([[0, "border.strong", 3, x0, y, x1, y]]);
    paths = paths.concat([[3, "surface.raised", x0, y, 7]]);
    paths = paths.concat([[3, "text.muted", x0, y, 5]]);
    paths = paths.concat([[3, "surface.raised", x1, y, 7]]);
    paths = paths.concat([[3, "series.1", x1, y, 5]])
  end
  drawing = canvas(plot_w, ph, paths)
  names = column(
    {"gap": 0, "width": gutter},
    range(0, count).map(fn(i) {
      {
        "k": "box",
        "s": {"display": "row", "align": "center", "justify": "end", "height": row_h, "width": "100%"},
        "c": [text(items[i]["label"], {"size": 0, "fg": "text.muted", "clamp": 1})]
      }
    })
  )
  heights = range(0, count).map(fn(i) { row_h })
  labels = range(0, count).map(fn(i) {
    items[i]["label"] + " · " + str(items[i]["from"]) + " → " + str(items[i]["to"])
  })
  plot = row(
    {"gap": 2, "width": w, "align": "start"},
    [
      column({"gap": 0, "width": gutter}, [names, {"k": "box", "s": {"height": CHART_AXIS_H}}]),
      column(
        {"gap": 0, "align": "center", "width": plot_w},
        [chart_row_layers(id, heights, labels, plot_w, ph, drawing), chart_value_axis(0, top, plot_w, CHART_TICKS - 1)]
      )
    ]
  )
  column({"gap": 2, "width": w}, [plot, chart_legend_pair("Before", "After")])
end

# ---- Sparkline -------------------------------------------------------------
#
# A trend beside a headline number, with no axis, no grid and no labels: it
# says "and it has been going this way", which is all the room allows. A
# single bar chart of one value is not a chart; a number with a sparkline is.
def chart_sparkline(vals, w, h)
  points = chart_points(vals, w, h)
  canvas(w, h, [[0, "series.1", 2].concat(flatten_points(points))])
end

# `stat` with the shape of the last few periods under the number.
def stat_spark(label, value, hint, vals, w)
  card(
    {"gap": 1, "width": "100%"},
    [
      muted(label),
      text(value, {"size": 6, "weight": "bold"}),
      chart_sparkline(vals, w, 28),
      text(hint, {"fg": "text.muted", "size": 0})
    ]
  )
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
