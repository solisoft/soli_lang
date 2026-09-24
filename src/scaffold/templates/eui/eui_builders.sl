# EUI view builders. Each returns a plain hash; nothing here is native.
# Lives in app/controllers/ so it loads with the handlers (one namespace).
#
# This is the reference catalogue spec/03-widgets.md §4 names. It is the
# copy `soli new <app> --eui` writes, vendored in the language repository
# under `src/scaffold/templates/eui/`; each file is kept byte for byte
# identical with its copy there by `scripts/sync-catalogue.sh`.
#
# The catalogue is six files, cut where the calls already stopped
# crossing. This one holds the primitives and the controls the rest is
# built from; the other four are leaves that call in and are never called
# back. It is one namespace either way, so the split says where to look,
# not what to import, and no file depends on which loads first.
#
#   eui_builders.sl           primitives, control base, split panes, data grid
#   eui_builders_forms.sl     pages, select, questions, fields, files, dev bar
#   eui_builders_charts.sl    axes, series, and every mark
#   eui_builders_feed.sl      tags, dragging, feed
#   eui_builders_markdown.sl  the document, its rows, and the editor
#   eui_builders_tw.sl        tw("..."): Tailwind-style classes as EUI styles
#
#   {"k": kind, "s": style, "t": text, "c": children, "on": handlers, "key": key, "p": props}
#
# Style keys are the spec's vocabulary: display, gap, pad, margin, bg, fg,
# border, border_color, radius, size, weight, width, height, align, justify,
# wrap, grow, cursor… Colours are role names ("accent.base") or "#RRGGBB".
#
# A style may also say `"tw": "flex items-center gap-3 hover:bg-gray-50"`, and
# `node()` — so `column`, `row` and `stack` — turn the classes into those keys
# and the `hover:`/`active:`/`focus:` ones into local handlers
# (`eui_builders_tw.sl`). Keys written beside `tw` win over it.

def node(kind, style, children)
  # Not `n`: a bare assignment here would write the caller's `n`, and half
  # the catalogue calls this with one of its own in hand.
  nd_made = {
    "k": kind,
    "s": style,
    "c": children
  }
  return nd_made if style.nil? || style["tw"].nil?

  tw_node(nd_made)
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
  return tw_text(content, style) unless style.nil? || style["tw_case"].nil?

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

# A 3D picture (EUI spec 03 §1.2), drawn by a graphics program rather than
# decoded from a file. `props` may carry "shader" and "mesh" — both content
# hashes, both optional, and the client draws its own cube when neither is
# given — plus "playing", "fps" and "uniforms". Those eight numbers are the
# author's half of the uniform block; the other twenty-four are the matrix,
# the clock and the size, which the client fills, so a server never sends a
# camera and cannot send a broken one.
#
# It needs the `scene` capability and a client that speaks EUI 2. Without
# either, the node paints its own background and nothing else — the module
# is not even fetched.
def scene(props, style)
  {
    "k": "scene",
    "s": style ?? {},
    "p": props ?? {}
  }
end

# `o` narrows a field without its caller having to reach into the hash
# afterwards: `style` is merged over the resting style, `props` is the
# identity and the semantics the handler and the screen reader read back,
# `key` names the node, `on` adds handlers beside `change`, and
# `placeholder` is the hint the client draws in `text.muted` while the field
# is empty (03 §3) -- never the value, and never sent back.
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
# The field is the Tailwind UI one: white (`surface.raised`) inside a
# `border.default` hairline, 14 px text, a small shadow under it. What says
# where the box is, is the border — it used to be a `surface.sunken` fill, and
# a grey well on a white card read as disabled to anyone who has used a web
# form this decade. `border.default` carries its own 3:1 against both surfaces
# (05 §4), so the edge survives the dark palette without the server knowing
# which one is on.
def editable(kind, value, on_change, o)
  base = {
    "pad": [2, 4, 2, 4],
    "border": 1,
    "border_color": "border.default",
    "radius": 2,
    "bg": "surface.raised",
    "size": 1,
    "shadow": 1,
    "transition": "fast"
  }
  style = base.merge(o["style"] ?? {})
  # A field drawn inside another box — the entry of a tag well, the number
  # in a currency field — says `bg: none` and lets the shell be the field. Its
  # shadow would then be cast by nothing, a grey slab inside the shell.
  style["shadow"] = 0 if style["bg"] == "none" && (o["style"] ?? {})["shadow"].nil?
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
  hint = o["placeholder"] ?? ""
  props = props.merge({"placeholder": hint}) if hint != ""
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
  # The ground does not change under the pointer or the caret: a web field
  # keeps its white, and says it is live with its edge. Focus paints the edge
  # in the accent as well as the client's ring, because the ring is drawn for
  # keyboard focus alone (03 §3) and a field clicked into had nothing else.
  hover = style.merge({})
  focus = style.merge({})
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
SPACE = [0, 2, 4, 8, 12, 16, 20, 24, 32, 40, 48, 64, 96, 6, 10, 14, 80, 128]

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

# Page title, section title, secondary line: `text-3xl font-bold`,
# `text-xl font-semibold`, `text-sm text-gray-500`. Body text in an
# application is 14 px — Tailwind UI sets `text-sm` as its default for
# anything that is not prose — so `muted` is that size too.
def h1(content)
  text(
    content,
    {"size": 6, "weight": "bold"}
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
#
# Every handler that declares styles, not a named four: a field also states a
# `focus` and a `blur` look (`editable_states`), and patching only the pointer
# events left the keyboard to undo the patch.
def restyle(n, patch)
  n["s"] = n["s"].merge(patch)
  handlers = n["on"]
  return n if handlers.nil?

  for name in handlers.keys()
    handler = handlers[name]
    # A handler that is only an event name is a string, and a string declares
    # no styles.
    next if handler.nil? || handler.to_s == handler

    styles = handler["styles"] ?? {}
    next if styles.keys().length() == 0

    for key in styles.keys()
      styles[key] = styles[key].merge(patch)
    end
    handler["styles"] = styles
    handlers[name] = handler
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
#
# The look is Tailwind UI's. A primary is a filled accent with a small shadow
# and no visible edge; its hover only moves the fill (the theme's
# `accent.hover` is the lighter indigo-500, as Tailwind's is). A secondary is
# white inside a `border.default` hairline — Tailwind's `ring-1 ring-inset
# ring-gray-300` — and hovers to the page grey. The width of the edge is
# reserved on every tone, coloured or not, so a primary and a secondary side
# by side are the same height, and nothing moves when a colour changes.
TONES = {
  "accent": {
    "bg": "accent.base", "fg": "accent.on", "border_color": "none", "shadow": 1,
    "hover": {"bg": "accent.hover"},
    "press": {"bg": "accent.active"},
    "selected": {}
  },
  "neutral": {
    "bg": "surface.raised", "fg": "text.default", "border_color": "border.default", "shadow": 1,
    "hover": {"bg": "surface.base"},
    "press": {"bg": "surface.sunken"},
    "selected": {"bg": "surface.sunken"}
  },
  "ghost": {
    "bg": "none", "fg": "accent.base", "border_color": "none",
    "hover": {"bg": "surface.sunken"},
    "press": {"bg": "surface.sunken", "fg": "accent.active"},
    "selected": {"bg": "surface.sunken"}
  },
  # The theme gives `accent` a hover and an active offset and gives the status
  # roles neither — there is no `danger.hover` to reach for. Tailwind's red
  # button lightens a step under the pointer (red-500 over red-600), and a
  # tenth off the node's opacity is that step without a colour the theme does
  # not have; the press drops the shadow, which is the button going down.
  "danger": {
    "bg": "danger.base", "fg": "danger.on", "border_color": "none", "shadow": 1,
    "hover": {"opacity": 230},
    "press": {"opacity": 255, "shadow": 0},
    "selected": {}
  },
  "quiet": {
    "bg": "none", "fg": "text.default", "border_color": "none",
    "hover": {"bg": "surface.sunken"},
    "press": {"bg": "surface.sunken"},
    "selected": {"bg": "surface.sunken"}
  },
  # An underlined tab, Tailwind's `border-b-2`: the ink is on the box and the
  # label inherits it, so the hover can darken the word and draw a grey rule
  # without the label having a style of its own to fight. The tab you are on
  # is a tone of its own, not a selection over this one — a hover delta would
  # otherwise repaint its accent rule grey under the pointer.
  "tab": {
    "bg": "none", "fg": "text.muted", "border_color": "none",
    "hover": {"fg": "text.default", "border_color": "border.default"},
    "press": {"fg": "text.default"},
    "selected": {}
  },
  "tab_on": {
    "bg": "none", "fg": "accent.base", "border_color": "accent.base",
    "hover": {},
    "press": {},
    "selected": {}
  }
}

# Disabled is one patch over the resting style, and there are no other styles
# left once the handlers are gone. `text.disabled` and `border.subtle` carry
# their own contrast guarantee (05 §4), so this stays legible in every mode.
DISABLED = {
  "fg": "text.disabled",
  "bg": "none",
  "border_color": "border.subtle",
  "shadow": 0,
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
#
# A control's label is `text-sm`, 14 px, at every size but `lg`: that is the
# size Tailwind UI sets its buttons, fields and menus in, and the size of the
# body text round them, so a control does not shout over its own form.
SIZES = {
  "sm": {"text": 1, "pad": [1, 3, 1, 3], "gap": 2, "min_width": 32, "icon": 24, "mark": 14},
  "md": {"text": 1, "pad": [2, 4, 2, 4], "gap": 3, "min_width": 44, "icon": 28, "mark": 16},
  "lg": {"text": 2, "pad": [3, 5, 3, 5], "gap": 3, "min_width": 56, "icon": 36, "mark": 20}
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
  base = {"bg": tone["bg"], "fg": tone["fg"], "border_color": tone["border_color"], "shadow": tone["shadow"] ?? 0}
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
#
# `tone` may also be a class string, or what `tw()` answered: its `hover:` and
# `active:` classes are the deltas, and a `focus:` adds a focus and a blur.
def stateful(base, tone, on)
  sf_tone = tone.class == "string" ? tw(tone) : tone
  hover = base.merge(sf_tone["hover"] ?? {})
  active = base.merge(sf_tone["press"] ?? {})
  sf_out = on.merge({
    "pointer_enter": {"local": "self.style = @hover", "styles": {"hover": hover}},
    "pointer_leave": {"local": "self.style = @base", "styles": {"base": base}},
    "pointer_down": {"local": "self.style = @active", "styles": {"active": active}},
    "pointer_up": {"local": "self.style = @hover", "styles": {"hover": hover}}
  })
  sf_focus = sf_tone["focus"] ?? {}
  return sf_out if sf_focus.keys().length() == 0

  sf_out.merge({
    "focus": {"local": "self.style = @focus", "styles": {"focus": base.merge(sf_focus)}},
    "blur": {"local": "self.style = @base", "styles": {"base": base}}
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
#   tw        the same, as Tailwind classes (`eui_builders_tw.sl`); its
#             `hover:`, `active:` and `focus:` classes are laid over the
#             tone's deltas, and `disabled:` over the disabled look.
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
  ct_tone = tone
  unless o["tw"].nil?
    ct_tw = tw(o["tw"])
    base = base.merge(ct_tw["s"])
    ct_tone = {
      "hover": (tone["hover"] ?? {}).merge(ct_tw["hover"]),
      "press": (tone["press"] ?? {}).merge(ct_tw["press"]),
      "focus": ct_tw["focus"]
    }
  end
  base = base.merge(o["shape"] ?? {})
  base = base.merge(DISABLED) if disabled
  base = base.merge(tw(o["tw"])["disabled"]) if disabled && !o["tw"].nil?
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
  n["on"] = stateful(base, ct_tone, o["on"] ?? {}) unless inert
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
#
# It is `control` now, like every other interactive widget: the four
# variants are four tones, and the fill and the ink a caller passes pick the
# tone. A pair nobody named is still honoured — it becomes the resting look
# over the quiet tone — so an application that wrote its own variant keeps
# it. The floor is one control tall (36 px at cozy density, `h-9`), which is
# where Tailwind UI puts a button and where the client already puts a field,
# so a button beside an input lines up with it.
def button_variant(label, on_click, bg, fg)
  bv_tone = "quiet"
  bv_tone = "accent" if bg == "accent.base"
  bv_tone = "neutral" if bg == "surface.sunken" || bg == "surface.raised"
  bv_tone = "danger" if bg == "danger.base"
  bv_tone = "ghost" if bg == "none" && fg == "accent.base"
  bv_shape = {"min_height": control_px("md", "cozy")}
  bv_shape = bv_shape.merge({"bg": bg, "fg": fg}) if bv_tone == "quiet"
  bv_shape = bv_shape.merge({"fg": fg}) if bv_tone == "neutral" && fg != "text.default"
  control({
    "key": "btn:" + str(on_click) + ":" + str(label),
    "tone": bv_tone,
    "size": "md",
    "shape": bv_shape,
    "on": {"click": on_click},
    "a11y": {"role": "button", "label": label},
    "c": [text(label, {"weight": "semibold", "size": control_text_size("md")})]
  })
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

# One button with a second opinion. The label is the thing you usually want;
# the caret beside it opens the things you sometimes want instead.
#
# It is one control and not two, because two would be two tab stops for one
# decision. The caret is a `context_menu` anchor rather than its own button:
# the menu already knows how to open, close on Escape and pick, and a split
# button that reimplemented any of that would drift from it.
#
# `o["items"]` are the alternatives, `o["open"]` is the server's, and
# `o["on_pick"]` reads `params["props"]["item"]`.
def split_button(label, on_click, o = {})
  sb_items = o["items"] ?? []
  sb_tone = o["tone"] ?? "accent"
  sb_size = o["size"] ?? "md"
  sb_pads = control_metrics(sb_size)["pad"]
  sb_key = o["key"] ?? ("split:" + on_click.to_s)
  # The sb_caret is inside the button, not beside it.
  #
  # Two controls side by side cannot be made to look like one: `radius` is a
  # single `u8` in the protocol, so a half cannot be round on its outer edge
  # and square against its neighbour, and the clip that would hide the seam
  # is rectangular (the renderer's clip stack holds rects, not rounded
  # rects). Two rounded halves read as two buttons, which is what they are.
  #
  # So: one rounded button, and the sb_caret is a node within it that takes the
  # click first -- an event goes to the nearest ancestor that handles it
  # (06 §2), so pressing the chevron opens the menu and pressing anywhere
  # else saves. The rule between them is what says the two ends do different
  # things; the hover is the whole button's, because the whole button is one
  # control.
  sb_caret = {
    "k": "box",
    "key": sb_key + ":more",
    "s": {
      "display": "row", "align": "center", "justify": "center",
      # Square against the button's own height, so the sb_caret is a target and
      # not a sliver: a 14 px chevron with the label's side padding is half
      # again as wide as it needs, and with none at all it is too small to hit.
      "pad": [sb_pads[0], 3, sb_pads[2], 3], "cursor": "pointer", "shrink": 0
    },
    "p": {"role": "button", "label": o["more_label"] ?? "More actions", "expanded": o["open"] == true},
    "on": {"click": o["on_toggle"]},
    "c": [icon("chevron_down", {"width": 14, "height": 14})]
  }
  sb_menu = context_menu(
    sb_caret,
    sb_items,
    o["open"] == true,
    o["on_toggle"],
    o["on_pick"],
    {"on_close": o["on_close"] ?? o["on_toggle"]}
  )
  control({
    "key": sb_key,
    "tone": sb_tone,
    "size": sb_size,
    # `self: start` so it is as wide as its label and its sb_caret. A control in
    # a column is stretched by the column's align, and a button the width of
    # the card it sits in is a button that looks like a banner.
    "shape": {"gap": 0, "pad": [0, 0, 0, sb_pads[3]], "align": "stretch", "self": "start", "min_height": control_px(sb_size, "cozy")},
    "on": {"click": on_click},
    "disabled": o["disabled"] == true,
    "loading": o["loading"] == true,
    "a11y": {"role": "button", "label": label},
    "c": [
      row({"align": "center", "grow": 1, "pad": [sb_pads[0], sb_pads[1], sb_pads[2], 0]}, [text(label, {"weight": "semibold", "size": control_text_size(sb_size)})]),
      node("box", {"width": 1, "bg": (TONES[sb_tone] ?? TONES["quiet"])["hover"]["bg"] ?? "border.subtle", "shrink": 0}, []),
      sb_menu
    ]
  })
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
      "border": 1,
      "border_color": disabled ? "border.subtle" : (lit ? "accent.base" : "border.strong"),
      "bg": (lit && !disabled) ? "accent.base" : "surface.raised",
      "display": "row",
      "justify": "center",
      "align": "center",
      "transition": "fast"
    },
    "c": lit ? [icon(
      mixed ? "minus" : "check",
      {
        "fg": disabled ? "text.disabled" : "accent.on",
        "width": box - 4,
        "height": box - 4
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
  # Tailwind's switch: a white knob with a small shadow, in a grey track that
  # fills with the accent. The knob stays white in both states, which is what
  # makes it read as the thing that moves.
  knob = {"k": "box", "s": {
    "width": knob_px,
    "height": knob_px,
    "radius": 4,
    "bg": "surface.raised",
    "shadow": 1,
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
      "bg": disabled ? "border.subtle" : (on ? "accent.base" : "border.default"),
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
      "border": selected ? 5 : 1,
      "border_color": disabled ? "border.subtle" : (selected ? "accent.base" : "border.strong"),
      "bg": "surface.raised",
      "display": "row",
      "justify": "center",
      "align": "center",
      "transition": "fast"
    },
    # Tailwind's checked radio is a filled accent disc with a white dot; a
    # thick accent border over a white ground is the same picture in one box.
    "c": []
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

# Tailwind UI's badge: a tint and its own ink, 12 px medium, `rounded-md`
# (radius 1) and not a pill — `rounded-full` is kept for dots and avatars, so a
# status reads as a label and not as a button. `tone` is a status role
# (`success`, `warning`, `danger`, `info`); `"neutral"` is the grey one.
def badge(label, tone)
  bd_bg = tone == "neutral" ? "surface.sunken" : tone + ".subtle"
  bd_fg = tone == "neutral" ? "text.muted" : tone + ".base"
  {
    "k": "box",
    "s": {
      "display": "row",
      "align": "center",
      "pad": [1, 3, 1, 3],
      "radius": 1,
      "bg": bd_bg,
      "shrink": 0
    },
    "c": [text(
      label,
      {
        "fg": bd_fg,
        "size": 0,
        "weight": "medium"
      }
    )]
  }
end

# A panel of the page: white, `rounded-lg`, the faint ring Tailwind draws
# with `ring-1 ring-gray-900/5`, a small shadow and 20 px of room. Overlays —
# dialogs, sheets, menus — are a step up on both radius and shadow, so a card
# never reads as something floating over the page.
def card(style, children)
  style["bg"] = style["bg"] ?? "surface.raised"
  style["radius"] = style["radius"] ?? 2
  style["border"] = style["border"] ?? 1
  style["border_color"] = style["border_color"] ?? "border.subtle"
  style["shadow"] = style["shadow"] ?? 1
  style["pad"] = style["pad"] ?? 6
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
      "tone": is_active ? "tab_on" : "tab",
      "shape": {
        "pad": [3, 1, 3, 1],
        "min_width": 0,
        "radius": 0,
        "border": [0, 0, 2, 0]
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
      # No `fg`: the label takes the box's, which is what the hover changes.
      "c": [text(name, {
        "size": control_text_size(size),
        "weight": "medium"
      })]
    })
  })
  strip = row({"gap": 7, "border": [0, 0, 1, 0], "border_color": "border.subtle"}, cells)
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
      "min_height": 36,
      "bg": "surface.raised",
      "fg": "text.default",
      "border": 1,
      "border_color": "border.default",
      "shadow": 1,
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
  caption = text(label, {"weight": "semibold", "size": 1})
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
      "min_height": 36,
      "bg": "surface.raised",
      "fg": "text.default",
      "border": 1,
      "border_color": "border.default",
      "shadow": 1,
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

# Tailwind UI's notification: a white panel over the page, `rounded-lg`
# and `shadow-lg`, with the tone in a mark beside the words rather than
# flooding the panel. A tinted panel said the tone louder than the message,
# and the message is the point.
def toast(message, tone)
  ts_mark = tone == "success" ? "check" : (tone == "danger" || tone == "warning" ? "warning" : "dot")
  {
    "k": "box",
    "p": {
      "role": "status",
      "live": tone == "danger" ? "assertive" : "polite"
    },
    "s": {
      "display": "row",
      "align": "center",
      "gap": 4,
      "pad": [5, 5, 5, 5],
      "radius": 3,
      "shadow": 3,
      "bg": "surface.overlay",
      "border": 1,
      "border_color": "border.subtle"
    },
    "c": [
      icon(ts_mark, {"width": 20, "height": 20, "shrink": 0, "fg": tone + ".base"}),
      text(message, {"fg": "text.default", "weight": "medium", "size": 1})
    ]
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

# A panel that a click stops at.
#
# An event goes to the nearest ancestor that handles it (06 §2), so once a
# scrim carries `click` to close, every click *inside* the panel would reach
# it too and shut the thing the person is using. An empty local chunk is the
# cheapest way to stop one: it handles the click, runs no instruction and
# sends nothing, so the click dies at the panel without a round trip.
def swallow_clicks(node)
  node["on"] = (node["on"] ?? {}).merge({"click": {"local": []}})
  node
end

def dialog(title, body_children, actions, opts = {})
  # A modal is a step above a card: radius 3 and the largest shadow, and
  # Tailwind's `text-base font-semibold` title rather than a page heading —
  # the dialog is a question about the page, not a page of its own.
  panel = card(
    {
      "width": opts["width"] ?? 400,
      "height": opts["height"] ?? "auto",
      "gap": 4,
      "self": "center",
      "radius": 3,
      "shadow": 3,
      "pad": 7
    },
    [text(title, {"size": 3, "weight": "semibold"})].concat(body_children, [row(
      {"gap": 3, "justify": "end", "pad": [2, 0, 0, 0]},
      actions
    )])
  )
  panel = swallow_clicks(panel)
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
      # And an `exit` is the same sentence backwards: the scrim unfrosts
      # rather than being cut away, which is what stops a dismissed dialog
      # from looking like a dropped frame.
      "animation": ["enter", "exit"],
      "transition": opts["transition"] ?? "slow"
    },
    # `modal` keeps Tab inside the dialog, and puts it there when it opens:
    # a server cannot do either, because it does not own Tab. `keys` claims
    # Escape alone, so the buttons inside keep Enter as the press they stand
    # for — and Escape now reaches the server, which is what closes it.
    # `opts["props"]` rides on the overlay itself, so that a caller whose
    # close event reads `params["props"]` — the handbook's `lazy_close`
    # wants an `id` — gets the same props from Escape and from the scrim as
    # it gets from its own Close button.
    "p": {
      "role": opts["alert"] == true ? "alert_dialog" : "dialog",
      "label": title,
      "modal": true,
      "autofocus": true,
      "keys": ["Escape"]
    }.merge(opts["props"] ?? {}),
    "c": [panel]
  }
  # Escape and the scrim say the same thing: this is over. 03 §5 — the
  # page behind is out of reach, and a click on it is a person reaching
  # for the way out.
  n["on"] = {"key_down": opts["on_close"], "click": opts["on_close"]} unless opts["on_close"].nil?
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
  dialog(title, [muted(message)], [button(opts["ok"] ?? "OK", on_close)], opts.merge({
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
  dialog(title, [muted(message)], [secondary_button(opts["cancel"] ?? "Cancel", on_cancel), ok], opts.merge({
    "alert": true,
    "on_close": on_cancel
  }))
end

# The label above a field: Tailwind's `text-sm font-medium text-gray-900`.
# Not muted — the label is what the field *is*, and grey said it was a hint.
def field_label(label)
  text(label, {"size": 1, "weight": "medium", "fg": "text.default"})
end

def field(label, value, on_change, o = {})
  column({"gap": 3}, [field_label(label), input(value, on_change, o)])
end

def form(children, submit_label, on_submit)
  column({"gap": 6}, children.concat([row({"justify": "end"}, [button(submit_label, on_submit)])]))
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

def split_panel(build, px, across: Bool, cross, dragging = false)
  {
    "k": "box",
    "s": {
      "display": "column",
      "width": across ? px : cross,
      "height": across ? cross : px,
      "cursor": dragging ? (across ? "resize_h" : "resize_v") : "default",
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
      # While the drag is live the whole split shows the resize cursor, not
      # just the six pixels of the bar. The move and the release already
      # belong to the container so that the pointer may leave the divider
      # without the drag ending -- but the cursor came from whatever node was
      # under it, so it flickered between resize and default as the hand
      # strayed a pixel onto a pane. The drag is the container's; so is the
      # cursor for as long as it lasts.
      "cursor": o["dragging"] == true ? (across ? "resize_h" : "resize_v") : "default",
      "overflow": "clip"
    },
    "c": [
      split_panel(o["a"], sizes[0], across, cross, o["dragging"] == true),
      split_divider(key + ":bar", across, bar, cross, fraction, on_drag, o["dragging"] == true, o["label"]),
      split_panel(o["b"], sizes[1], across, cross, o["dragging"] == true)
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

# A row that opens onto what is inside it.
#
# The alternative is a dialog, and a dialog is the wrong shape for this: the
# lines of an order belong *under* the order, in the table you were reading,
# with the rows above and below still there to compare against. What opens
# here is a panel in the flow, so the table grows and nothing is covered.
#
# The chevron is not a separate control. The whole row opens it, because a
# row whose only open affordance is a 14 px glyph is a row most people do not
# know opens at all -- and the glyph still turns, so it says which it is.
def expandable_row(key, values, widths, open, on_toggle, detail, o = {})
  er_head = control({
    "key": key + ":head",
    "kind": "box",
    "tone": "quiet",
    "size": "md",
    "selected": open,
    "shape": {
      "display": "row", "gap": 4, "align": "center", "width": "100%",
      "justify": "start", "pad": [4, 3, 4, 3], "radius": 0,
      "border": [0, 0, 1, 0], "border_color": "border.subtle",
      "min_width": 0
    },
    "on": {"click": on_toggle},
    "props": o["props"] ?? {},
    "a11y": {"role": "row", "label": o["label"] ?? values[0].to_s, "expanded": open},
    "c": [icon(open ? "chevron_down" : "chevron_right", {"width": 14, "height": 14, "fg": "text.muted", "shrink": 0})].concat(
      range(0, values.length()).map(fn(i) { text(values[i], {"width": widths[i], "size": 1}) })
    )
  })
  er_body = open ? [{
    "k": "box",
    "s": {
      "display": "column", "gap": 3, "width": "100%",
      "pad": [3, 3, 3, 8], "bg": "surface.sunken",
      "border": [0, 0, 1, 0], "border_color": "border.subtle"
    },
    "c": detail
  }] : []
  keyed(key, column({"gap": 0, "width": "100%"}, [er_head].concat(er_body)))
end

# A table whose rows nest.
#
# `tree_view` is a rail and `data_grid` is flat, and between them sits every
# hierarchy that also has columns: a bill of materials with quantities, a
# chart of accounts with balances, a folder tree with sizes. Indentation
# lives in the first cell rather than on the row, so the numbers down the
# right stay in their columns however deep the nesting goes -- which is the
# whole reason to put a tree in a table instead of beside one.
#
# A row is `{"id", "cells": [...], "children": [...]}`. Depth is this
# function's own business; callers pass 0 and let the recursion carry it.
def tree_table(labels, widths, rows, open_ids, on_toggle, depth = 0, o = {})
  tt_opened = open_ids ?? []
  tt_drawn = rows.map(fn(r) {
    kids = r["children"] ?? []
    has_kids = kids.length() > 0
    is_open = tt_opened.includes?(r["id"])
    cells = r["cells"] ?? []
    first = row({"gap": 1, "align": "center", "width": widths[0], "shrink": 0}, [
      node("box", {"width": depth * 4, "shrink": 0}, []),
      has_kids
        ? icon(is_open ? "chevron_down" : "chevron_right", {"width": 14, "height": 14, "fg": "text.muted", "shrink": 0})
        : node("box", {"width": 14, "shrink": 0}, []),
      text(cells[0], {"grow": 1, "clamp": 1, "size": 1, "weight": has_kids ? "semibold" : "regular"})
    ])
    rest = range(1, cells.length()).map(fn(i) { text(cells[i], {"width": widths[i], "size": 1, "fg": "text.muted"}) })
    line = control({
      "key": (o["key"] ?? "tt") + ":" + r["id"].to_s,
      "kind": "box",
      "tone": "quiet",
      "shape": {
        "display": "row", "gap": 4, "align": "center", "width": "100%",
        "justify": "start", "pad": [4, 3, 4, 3], "radius": 0,
        "border": [0, 0, 1, 0], "border_color": "border.subtle",
        "min_width": 0, "cursor": has_kids ? "pointer" : "default"
      },
      "on": has_kids ? {"click": on_toggle} : {},
      "props": {"id": r["id"]},
      "a11y": {
        "role": "tree_item",
        "label": cells[0].to_s,
        "expanded": has_kids ? is_open : null,
        "level": depth + 1
      },
      "c": [first].concat(rest)
    })
    under = is_open && has_kids
      ? [tree_table(labels, widths, kids, tt_opened, on_toggle, depth + 1, o)]
      : []
    column({"gap": 0, "width": "100%"}, [line].concat(under))
  })
  return column({"gap": 0, "width": "100%"}, tt_drawn) if depth > 0

  tt_whole = column({"gap": 0, "width": "100%"}, [table_header(labels, widths)].concat(tt_drawn))
  tt_whole["p"] = {"role": "tree", "label": o["label"] ?? "Tree"}
  tt_whole
end

# Tailwind UI's table: a header in `text-sm font-semibold text-gray-900` on
# the same white as the rows, over a `gray-300` rule, and rows split by the
# fainter `gray-200` one with 12 px of room above and below. The first column
# is the row's name and keeps the default ink; the rest are `text-gray-500`,
# which is what lets the eye run down the names.
#
# A row answers the pointer with the page grey (`hover:bg-gray-50`). The
# hover is local and styles the row itself (`self`), so it costs no round trip.
#
# `o["dense"]` is the other table: 8 px of room instead of 24, and no hover.
# A virtualised list of ten thousand rows wants both — its `item_height` has
# to be the row's, and four handlers on every one of ten thousand rows is
# bytes on a mount that spec 10 §4 budgets per node.
def table_header(labels, widths)
  cells = range(0, labels.length()).map(fn(i) {
    {
      "k": "text",
      "t": labels[i],
      "s": {
        "width": widths[i],
        "weight": "semibold",
        "size": 1,
        "fg": "text.default"
      }
    }
  })
  row(
    {
      "gap": 4,
      "pad": [3, 3, 3, 3],
      "border": [0, 0, 1, 0],
      "border_color": "border.default"
    },
    cells
  )
end

def table_row(key, values, widths, o = {})
  cells = range(0, values.length()).map(fn(i) { {
    "k": "text",
    "t": values[i],
    "s": {"width": widths[i], "size": 1, "fg": i == 0 ? "text.default" : "text.muted", "weight": i == 0 ? "medium" : "regular"}
  } })
  tr_dense = o["dense"] == true
  tr_base = {
    "display": "row",
    "gap": 4,
    "pad": tr_dense ? [2, 3, 2, 3] : [4, 3, 4, 3],
    "border": [0, 0, 1, 0],
    "border_color": "border.subtle",
    "transition": "fast"
  }
  tr_row = {"k": "box", "s": tr_base, "c": cells}
  tr_row["on"] = stateful(tr_base, {"hover": {"bg": "surface.base"}, "press": {"bg": "surface.base"}}, {}) unless tr_dense
  keyed(key, tr_row)
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
  hover = base.merge({"bg": selected == true ? "info.subtle" : "surface.base"})
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
      "pad": [3, 2, 3, 2],
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
          "fg": active == true ? "accent.base" : "text.default"
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
      "border_color": "border.default"
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
      "shadow": 1,
      "overflow": "clip",
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

# Who is on this, in the space of one and a half faces.
#
# The overlap is the point: a row of eight avatars at full width is a row
# nobody reads, and the question being asked is "roughly who", not "exactly
# which". Past `max` the rest become a count, which is the honest way to say
# more than fits. Negative margin does the stacking, so the group is as wide
# as what it shows rather than as wide as what it has.
#
# Each entry is `{"src"}` or `{"initial", "tone"}` -- a picture when there is
# one, a letter when there is not, the same pair `post_avatar` draws.
def avatar_group(people, o = {})
  ag_size = o["size"] ?? 28
  # Not `max`: it is a builtin, and a bare assignment rebinds the global.
  ag_cap = o["max"] ?? 4
  ag_shown = people.length() > ag_cap ? people.slice(0, ag_cap) : people
  ag_rest = people.length() - ag_shown.length()
  # `margin` is unsigned in the protocol -- `[u8; 4]`, 0-255 -- so there is no
  # negative margin to pull a face back over the one before it. What does the
  # same work: give every face but the ag_last a slot narrower than it is. A box
  # does not clip unless it is told to, so each face spills over the slot
  # after it, and the row is as wide as the slots rather than as wide as the
  # ag_faces.
  # A quarter, not a third: a third reads as a pile rather than a row, and
  # the initials stop being legible, which is the only thing a letter avatar
  # has to offer.
  ag_lap = ag_size / 4
  ag_last = ag_shown.length() - 1
  ag_faces = range(0, ag_shown.length()).map(fn(i) {
    p = ag_shown[i]
    face = (p["src"] ?? "") == ""
      ? initial_avatar(p["initial"] ?? "?", p["tone"] ?? "accent.base", ag_size)
      : avatar(p["src"], ag_size)
    face["s"] = face["s"].merge({"border": 2, "border_color": "surface.base", "shrink": 0})
    node("box", {"width": i == ag_last && ag_rest <= 0 ? ag_size : ag_size - ag_lap, "height": ag_size, "shrink": 0}, [face])
  })
  # Not `initial_avatar`: it paints its letter in `accent.on`, which is the
  # right colour on a tone and invisible on a sunken ground.
  ag_tail = ag_rest <= 0 ? [] : [{
    "k": "box",
    "s": {
      "width": ag_size, "height": ag_size, "radius": 4,
      "display": "row", "justify": "center", "align": "center",
      "bg": "surface.sunken",
      "border": 2, "border_color": "surface.base", "shrink": 0
    },
    "c": [text("+" + ag_rest.to_s, {"size": 0, "weight": "semibold", "fg": "text.muted"})]
  }]
  ag_strip = row({"gap": 0, "align": "center", "shrink": 0}, ag_faces.concat(ag_tail))
  ag_strip["p"] = {"role": "group", "label": o["label"] ?? (people.length().to_s + " people")}
  ag_strip
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
      "height": 8,
      "radius": 4,
      "bg": "border.subtle",
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

# A removable tag, drawn as Tailwind's grey badge: `rounded-md`, 12 px
# medium, a faint ring. A pill was a button's shape, and a chip is a value.
def chip(label, on_remove, props)
  parts = [text(label, {"size": 0, "weight": "medium", "fg": "text.default"})]
  parts = parts.concat([chip_remove(on_remove, props, label)]) if on_remove.present?
  {
    "k": "box",
    "s": {
      "display": "row",
      "align": "center",
      "gap": 1,
      "pad": [1, 2, 1, 3],
      "radius": 1,
      "bg": "surface.base",
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
#
# It goes through `restyle`, because the patch has to reach the styles the
# node's local handlers declare as well as its resting one. A `stateful`
# card builds its hover out of the style it had *before* it was tiled, so a
# tile that only touched `node["s"]` left a hover carrying the old
# `width: 100%`: the first pointer_enter dropped the basis, the card grew to
# the whole row, and the grid reflowed under the pointer. Every style on a
# node is a whole style, so every one of them has to be tiled.
def tile(basis, node)
  restyle(node, {"width": "auto", "basis": basis, "grow": 1, "shrink": 1})
end

# Tailwind UI's stat: the name in `text-sm font-medium text-gray-500`, the
# figure in `text-3xl font-semibold`, and the note under it small.
def stat(label, value, hint)
  card(
    {"gap": 2, "width": "100%"},
    [text(label, {"size": 1, "weight": "medium", "fg": "text.muted"}), text(
      value,
      {"size": 6, "weight": "semibold"}
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
      text(title, {"size": 1, "weight": "semibold"}),
      text(
        body,
        {"fg": "text.muted", "size": 1, "text_align": "center"}
      ),
      node("box", {"height": 3}, []),
      button(action_label, on_action)
    ]
  )
end

def banner(message, tone, action_label, on_action)
  row(
    {
      "gap": 3,
      "align": "center",
      "pad": [3, 5, 3, 5],
      "radius": 2,
      "bg": tone + ".subtle",
      "border": [0, 0, 0, 4],
      "border_color": tone + ".base"
    },
    [text(message, {"fg": "text.default", "size": 1}), spacer(), ghost_button(action_label, on_action)]
  )
end

# Breadcrumb: every crumb but the last is a link carrying its path.
def breadcrumb(crumbs, on_go)
  parts = []
  i = 0
  # Tailwind's: grey `text-sm font-medium` crumbs split by a chevron, the
  # one you are on in the default ink. A crumb is a way back, not a link out,
  # so it is not painted in the accent.
  for crumb in crumbs
    parts = parts.concat([icon("chevron_right", {"width": 14, "height": 14, "fg": "text.disabled", "shrink": 0})]) if i > 0
    if i == crumbs.length() - 1
      parts = parts.concat([text(crumb["label"], {"weight": "medium", "size": 1, "fg": "text.default"})])
    else
      link = {
        "k": "text",
        "t": crumb["label"],
        "s": {"fg": "text.muted", "weight": "medium", "size": 1, "cursor": "pointer"},
        "on": {"click": on_go},
        "p": {"path": crumb["path"], "role": "link", "label": crumb["label"]}
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
# `segmented` asks which one; this asks which ones. The difference is not
# cosmetic: a filter bar that offers four statuses and lets you pick one is
# a filter bar that cannot express "late or unpaid", which is the question
# people actually have. `chosen` is a list, and a cell sends the option it
# stands for -- the server does the adding and removing, because what the
# list means is the server's business.
def toggle_group(options, chosen, on_toggle, o = {})
  tg_picked = chosen ?? []
  tg_count = options.length()
  tg_cells = range(0, tg_count).map(fn(i) {
    opt = options[i]
    lit = tg_picked.includes?(opt)
    control({
      "key": (o["key"] ?? "toggles") + ":" + opt.to_s,
      "size": o["size"] ?? "sm",
      "tone": "quiet",
      "selected": lit,
      "shape": {"radius": 1, "pad": [1, 3, 1, 3], "min_width": 0, "bg": lit ? "surface.raised" : "none", "border": 0, "shadow": lit ? 1 : 0},
      "on": {"click": on_toggle},
      "props": {"option": opt, "item": opt},
      "a11y": {
        "role": "check_box",
        "label": opt.to_s,
        "checked": lit,
        "pos_in_set": i + 1,
        "set_size": tg_count
      },
      "c": [text(opt, lit ? {"weight": "medium", "size": 1} : {"fg": "text.muted", "weight": "medium", "size": 1})]
    })
  })
  tg_strip = row({"gap": 1, "pad": 1, "radius": 2, "bg": "surface.sunken", "shrink": 0, "wrap": "wrap"}, tg_cells)
  tg_strip["p"] = {"role": "group", "label": o["label"] ?? "Filters"}
  tg_strip
end

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
        "shadow": is_sel ? 1 : 0,
        "cursor": "pointer"
      },
      "on": {"click": on_select},
      "c": [text(opt, is_sel ? {"weight": "medium", "size": 1} : {"fg": "text.muted", "weight": "medium", "size": 1})]
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

# Accordion: sections with a header that toggles by id; the open one shows its
# body. A section is `{"id", "title", "body"}`, or `"children"` in place of the
# body when the body is more than a line of text.
def accordion(sections, open_id, on_toggle)
  column(
    {
      "gap": 0,
      "border": 1,
      "border_color": "border.subtle",
      "radius": 2,
      "bg": "surface.raised",
      "overflow": "clip"
    },
    sections.map(fn(sec) {
      is_open = sec["id"] == open_id
      header = {
        "k": "box",
        "s": {
          "display": "row",
          "align": "center",
          "gap": 3,
          "pad": [4, 5, 4, 5],
          "cursor": "pointer",
          "border": [0, 0, 1, 0],
          "border_color": "border.subtle"
        },
        "on": {"click": on_toggle},
        "p": {"id": sec["id"]},
        "c": [text(sec["title"], {"weight": "semibold", "size": 1, "grow": 1}), icon(
          is_open ? "chevron_up" : "chevron_down",
          {"fg": "text.muted", "width": 16, "height": 16, "shrink": 0}
        )]
      }
      body = is_open ? [column({"pad": [
        0,
        5,
        5,
        5
      ]}, sec["children"] ?? [text(sec["body"].to_s, {"fg": "text.muted", "size": 1})])] : []
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
      [dot, text(label, i == current ? {"weight": "semibold", "size": 1, "fg": "accent.base"} : {"fg": "text.muted", "weight": "medium", "size": 1})]
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
#
# Tailwind UI's dropdown: white, `rounded-lg`… a step up from a card on
# radius and on shadow, because it floats; items in `text-sm text-gray-700`
# that wash to `gray-100` under the pointer. The wash is a local handler on
# `self`, so it costs no round trip and needs no key.
def menu(items, on_pick)
  column(
    {
      "gap": 0,
      "pad": [1, 0, 1, 0],
      "radius": 3,
      "shadow": 3,
      "bg": "surface.overlay",
      "border": 1,
      "border_color": "border.subtle",
      "min_width": 176
    },
    items.map(fn(it) {
      mn_base = {
        "pad": [2, 5, 2, 5],
        "fg": "text.default",
        "cursor": "pointer",
        "transition": "fast"
      }
      {
        "k": "box",
        "s": mn_base,
        "on": stateful(mn_base, TONES["quiet"], {"click": on_pick}),
        "p": {"item": it, "role": "menu_item", "label": it.to_s},
        "c": [text(it, {"size": 1})]
      }
    })
  )
end

# A question asked where it was raised.
#
# `confirm` puts a modal over the whole window, which is right for "discard
# this draft" and much too much for "remove this row": the page goes dark,
# the eye leaves the thing being removed, and the answer arrives somewhere
# else entirely. This asks beside the control instead, and the row stays
# visible behind the question about it.
#
# The caller owns `open` and gives the anchor its own click. `o["props"]`
# ride on the confirming button, because the question is always about a
# particular thing and the handler has to be told which.
def popconfirm(anchor, open, o = {})
  pc_key = o["key"] ?? "popconfirm"
  pc_panel = column(
    {
      "gap": 3, "pad": 5, "radius": 3, "shadow": 3,
      "bg": "surface.overlay", "border": 1, "border_color": "border.subtle",
      "max_width": 280
    },
    [
      text(o["question"] ?? "Are you sure?", {"weight": "semibold", "size": 1}),
      o["detail"].nil? ? spacer() : muted(o["detail"]),
      row({"gap": 2, "justify": "end", "align": "center"}, [
        control({
          "key": pc_key + ":no",
          "tone": "quiet",
          "size": "sm",
          "on": {"click": o["on_cancel"]},
          "a11y": {"role": "button", "label": o["cancel"] ?? "Cancel"},
          "c": [text(o["cancel"] ?? "Cancel", {"size": 1})]
        }),
        control({
          "key": pc_key + ":yes",
          "tone": o["tone"] ?? "danger",
          "size": "sm",
          "on": {"click": o["on_confirm"]},
          "props": o["props"] ?? {},
          "a11y": {"role": "button", "label": o["confirm"] ?? "Remove"},
          "c": [text(o["confirm"] ?? "Remove", {"weight": "semibold", "size": 1})]
        })
      ])
    ]
  )
  pc_panel["p"] = {"role": "dialog", "label": o["question"] ?? "Are you sure?", "keys": ["Escape"]}
  pc_panel["on"] = {"key_down": o["on_cancel"]} unless o["on_cancel"].nil?
  popover(anchor, [pc_panel], open == true)
end

# The menu that a right-click opens. The client already emits `context_menu`
# on button 1 (06 §1); this is the overlay it opens, the same column `menu`
# draws, hung off the node that was pointed at.
#
# `on_open` is the event the right-click sends. `on_pick` is what a row
# sends, with `params["props"]["item"]` the label. `o["on_close"]` is Escape.
# The server owns `open`, as it owns a popover's.
def context_menu(anchor, items, open, on_open, on_pick, o = {})
  on = (anchor["on"] ?? {}).merge({"context_menu": on_open})
  on["key_down"] = o["on_close"] unless o["on_close"].nil?
  anchor["on"] = on
  props = anchor["p"] ?? {}
  props["keys"] = ["Escape"] unless o["on_close"].nil?
  anchor["p"] = props
  return anchor unless open == true

  popover(anchor, [menu(items, on_pick)], true)
end

# A command is `{id, label, hint, group}`. A string is a label that is its
# own id. The four functions below are the whole of the model, so a spec
# can pin them without a client.
def command_row(it)
  return {"id": it.to_s, "label": it.to_s, "hint": "", "group": ""} unless it.class == "hash"

  {
    "id": (it["id"] ?? it["label"]).to_s,
    "label": (it["label"] ?? it["id"]).to_s,
    "hint": (it["hint"] ?? "").to_s,
    "group": (it["group"] ?? "").to_s,
    # Optional, and blank when a command does not name one: a palette row
    # without an icon should hold the same left edge as one with it, or the
    # labels of a mixed list zig-zag down the panel.
    "icon": (it["icon"] ?? "").to_s
  }
end

def command_match(items, query)
  said = (query ?? "").strip().downcase()
  out = []
  for it in items
    # Not `row`: a bare assignment rebinds the global, and `row()` is the
    # builder half this file is made of (line 28). Opening the palette once
    # turned it into a node and every later view raised.
    hit = command_row(it)
    hay = (hit["label"] + " " + hit["hint"] + " " + hit["group"]).downcase()
    out = out.concat([hit]) if said == "" || hay.index_of(said) >= 0
  end
  out
end

PALETTE_KEYS = ["ArrowDown", "ArrowUp", "Escape"]

# Overlay, a field, a list. Type to narrow, arrows to walk, Enter to take.
# The caller owns `query` and `at`; this is a view of them. Closed, it
# draws nothing — include it in the tree only while it is up, the way a
# dialog is.
#
# `o`: `on_change`, `on_key`, `on_pick`, `on_submit`, `on_close`, `key`.
def command_palette(query, items, at, o = {})
  pal_key = (o["key"] ?? "palette").to_s
  pal_at = at ?? -1
  pal_rows = command_match(items, query)
  pal_entry = input(query ?? "", o["on_change"], {
    "key": pal_key + ":entry",
    "style": {"width": "100%"},
    "props": {
      "keys": PALETTE_KEYS,
      "role": "combo_box",
      "expanded": true,
      "label": "Command",
      "autofocus": true
    },
    "on": {
      "key_down": o["on_key"],
      "submit": o["on_submit"]
    }
  })
  if pal_at >= 0 && pal_at < pal_rows.length()
    pal_entry["p"]["active_descendant"] = pal_key + ":opt:" + pal_rows[pal_at]["id"]
  end
  pal_list = []
  pal_group = ""
  i = 0
  while i < pal_rows.length()
    pal_row = pal_rows[i]  # not `row`: it is the builder at line 28
    if pal_row["group"] != "" && pal_row["group"] != pal_group
      pal_group = pal_row["group"]
      pal_list = pal_list.concat([muted(pal_group)])
    end
    lit = i == pal_at
    kids = [(pal_row["icon"] ?? "") == ""
      ? node("box", {"width": 16, "shrink": 0}, [])
      : icon(pal_row["icon"], {"width": 16, "height": 16, "shrink": 0, "fg": lit ? "accent.base" : "text.muted"}),
      text(pal_row["label"], {"grow": 1, "size": 1, "weight": lit ? "medium" : "regular"})]
    kids = kids.concat([muted(pal_row["hint"])]) unless pal_row["hint"] == ""
    pal_list = pal_list.concat([control({
      "key": pal_key + ":opt:" + pal_row["id"],
      "size": "sm",
      "tone": "quiet",
      "shape": {
        "justify": "start",
        "width": "100%",
        "gap": 3,
        "radius": 2,
        "bg": lit ? "surface.sunken" : "none",
        "border": [0, 0, 0, 3],
        "border_color": lit ? "accent.base" : "none"
      },
      "on": {"click": o["on_pick"]},
      "props": {"id": pal_row["id"], "item": pal_row["label"]},
      "a11y": {
        "role": "option",
        "selected": lit,
        "label": pal_row["label"],
        "pos_in_set": i + 1,
        "set_size": pal_rows.length()
      },
      "c": kids
    })])
    i = i + 1
  end
  pal_body = pal_list.length() == 0 ? [muted("Nothing matches")] : pal_list
  panel = card(
    {
      "width": o["width"] ?? 480,
      "max_width": "100%",
      "gap": 3,
      "self": "center",
      "radius": 3,
      "shadow": 3,
      "pad": 4
    },
    [pal_entry, scroll({"max_height": 360}, pal_body)]
  )
  panel = swallow_clicks(panel)
  n = {
    "k": "overlay",
    "key": pal_key,
    "s": {
      "display": "stack",
      "justify": "center",
      "align": "center",
      "pad": [10, 4, 4, 4],
      "blur": 16,
      "bg": "#00000073",
      "animation": "enter",
      "transition": "slow"
    },
    "p": {
      "role": "dialog",
      "label": "Command palette",
      "modal": true,
      "autofocus": true,
      "keys": ["Escape"]
    },
    "c": [panel]
  }
  n["on"] = {"key_down": o["on_close"], "click": o["on_close"]} unless o["on_close"].nil?
  n
end

def tooltip(content)
  {
    "k": "box",
    "s": {
      "pad": [1, 3, 1, 3],
      "radius": 2,
      "shadow": 2,
      "bg": "text.default"
    },
    "c": [text(
      content,
      {"fg": "text.inverted", "size": 0, "weight": "medium"}
    )]
  }
end

# A sheet slides from an edge over the page: overlay, dim, panel at the edge.
def sheet(side, children, opts = {})
  panel = column(
    {
      "gap": 4,
      "pad": 7,
      "bg": "surface.raised",
      "shadow": 3,
      "width": 384,
      "self": "stretch"
    },
    children
  )
  panel = swallow_clicks(panel)
  n = {
    "k": "overlay",
    "key": opts["key"] ?? ("sheet:" + side),
    "s": {
      "display": "row",
      "justify": side == "left" ? "start" : "end",
      "align": "stretch",
      "blur": 10,
      "bg": "#00000047",
      # A sheet slides in by the edge it belongs to and leaves the same way.
      # Its direction never flips — it is not a page in a stack, it is a panel
      # attached to a side — so it names its own and needs no mirror.
      "animation": ["enter", "exit"],
      "motion": side == "left" ? "leading" : "trailing",
      "transition": opts["transition"] ?? "base"
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
  n["on"] = {"key_down": opts["on_close"], "click": opts["on_close"]} unless opts["on_close"].nil?
  n
end
# Lighter than a dialog's, and blurred less: a sheet is somewhere you
# went, not a question you have to answer, and the page it slid over
# should stay recognisable behind it.
