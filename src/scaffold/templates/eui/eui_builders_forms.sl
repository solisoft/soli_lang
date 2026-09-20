# EUI view builders, part 2: what a form is made of.
#
# Pages and the navigator they move through, select, the question widgets,
# multi-selection, sliders, the calendar engine and the fields that open
# it, file pickers, and the dev bar.
#
# Part of the reference catalogue — `eui_builders.sl` has the header that
# explains the whole of it, and the primitives everything here calls.

# ---- Pages ------------------------------------------------------------
#
# A navigator is one page on screen, a stack in state, and the client owning
# the movement between them. Nothing here says how long a push takes or which
# way it goes: the page says how it *arrives*, the client mirrors that for
# whatever is leaving, and a pop is the same sentence read backwards.

# One page. The name carries the `nav_` prefix its neighbours do, and not the
# bare `page` it wants, because `page` is what half the views in the world
# call a page *number* — and a bare assignment to it rebinds the function for
# the rest of the run. A catalogue that ships to every new project cannot
# take a name that common.
#
# `key` is what makes it a different page rather than the same one
# with new contents — a keyed child that disappears is removed, and removal is
# what an exit is a rendering of.
#
# `motion` is where it comes *in* from. The page it covers goes the other way
# without being told to, so there is only ever one direction to get right.
#
# It **restyles the node it is given** rather than wrapping it in one. A
# wrapper is not free: `grow` only shares out space that is left over, so a
# box around a scroller whose content is taller than the window keeps its
# content size instead of shrinking into what the header left — and the
# scroller then runs off the bottom of the screen, where nothing can reach
# it. Being the page rather than boxing it has no layout at all.
def nav_page(key, node, o = {})
  motion = o["motion"] ?? "trailing"
  # `none` is a page that still *is* a page — keyed, so it is replaced rather
  # than edited in place — and simply appears. Nothing is animated, nothing is
  # kept after it goes, and the client is asked for no frames: the cut this
  # application had before any of this existed.
  return keyed("page:" + key, node) if motion == "none"

  n = restyle(node, {
    "z": o["z"] ?? 0,
    "animation": ["enter", "exit"],
    "motion": motion,
    "transition": o["transition"] ?? "base"
  })
  keyed("page:" + key, n)
end

# A shared element: one thing on two pages (03 §5.3).
#
# Give this to the node on the page that is leaving **and** to the node on
# the page that is arriving, under the same `name`, and the client flies the
# arriving one out of the box the leaving one had instead of letting each go
# the way its page goes. A row's avatar becoming the header's avatar is the
# whole of it; nothing here says how far or how long, because both ends are
# boxes the client already laid out.
#
# `animation` is not optional and not something the client fills in: a style
# record naming a motion with neither an entrance nor an exit is refused on
# the wire ("motion needs an entrance or an exit to belong to"), in this
# client and in all six SDKs.
#
# **A name that resolves to nothing is the ordinary case and not an error.**
# The node simply takes the motion of the page it is on. Which is convenient
# — a panel is built and torn down as it opens — and is also why a wrong name
# is invisible: nothing moves, nothing complains. `EUI_TRACE=1` prints a line
# per pair, including the ones that did not resolve and why.
#
# So the list side wears it on **every** row: which row is about to be the
# one is not known until it is tapped.
#
# It restyles the node it is given rather than wrapping it, for the reason
# `nav_page` gives above: a wrapper is layout, and this is not.
def shared_element(name, node, o = {})
  restyle(keyed("pair:" + name, node), {
    "animation": ["enter", "exit"],
    "motion": "paired",
    "transition": o["transition"] ?? "base"
  })
end

# The way out of a page you went into (06 §1.3).
#
# It asks the client first and the server after. `back()` is the one thing a
# local handler can say that is a *request* rather than a change: what it asks
# for is the server's to grant, so this button and a swipe from the leading
# edge cannot come to mean two different things — they are the same event,
# arriving by different hands.
def back_button(after)
  b = icon_button("‹", after, {}, {"icon": "chevron_left", "name": "Back", "key": "nav_back"})
  b["on"]["click"] = {"local": "back()", "then": after}
  b
end

# Nothing, where a layout wants a child and there is none to give it.
def spacer_none()
  {"k": "spacer", "s": {"width": 0, "height": 0}}
end

# The stack, rendered. `pages` is a hash of **thunks**, so only what is on
# screen is built — the `lazy` discipline, one level up: a navigator over
# eight sections costs two of them, not eight.
#
# Two are rendered and not one, when there is one underneath. An edge swipe
# reveals what it is going back to from the first frame it moves, and there is
# nothing to reveal if the server never sent it. `{"under": false}` is the
# opt-out for a page too heavy to keep laid out, and costs the swipe its
# liveness — it then waits for the server, like a tapped button.
def navigator(state, pages, o = {})
  st = state["nav_stack"] ?? ["home"]
  top = st[st.length() - 1]
  back = st.length() > 1 ? st[st.length() - 2] : nil
  # A pop arrives from where a push would have gone.
  edge = (state["nav_dir"] ?? "none") == "pop" ? "leading" : "trailing"
  kids = []
  kids = kids.concat([
    nav_page(back, pages[back](), {"z": 0, "motion": "leading"})
  ]) if !back.nil? && (o["under"] ?? true)
  kids = kids.concat([
    nav_page(top, pages[top](), {
      "z": back.nil? ? 0 : 1,
      "motion": (state["nav_dir"] ?? "none") == "none" ? "fade" : edge
    })
  ])
  n = stack({"grow": 1, "width": "100%"}, kids)
  keyed(o["key"] ?? "nav", n)
end

# The handler half. Three assignments and no knowledge of the client at all:
# what the stack is, and which way it last moved.
def nav_push(state, key)
  st = (state["nav_stack"] ?? ["home"]).concat([key])
  set_key(set_key(state, "nav_stack", st), "nav_dir", "push")
end

def nav_pop(state)
  st = state["nav_stack"] ?? ["home"]
  return state if st.length() <= 1
  set_key(set_key(state, "nav_stack", st.slice(0, st.length() - 1)), "nav_dir", "pop")
end

# Where the stack is, for a view that wants to mark its own nav.
def nav_top(state)
  st = state["nav_stack"] ?? ["home"]
  st[st.length() - 1]
end

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

# A patch, read as one column.
#
# Unified and not side by side, and that is a judgement rather than a
# shortcut: two panes make you compare across a gutter, they halve the width
# every line has, and they need two scrollers that have to be kept in step.
# One column puts the removed line directly above the line that replaced it,
# which is where the eye wants it.
#
# The sign column carries the meaning and the tint repeats it, because a
# tint alone is a diff that says nothing in high contrast and nothing to
# anyone who cannot separate the two greens. `old` and `new` are line
# numbers, and a blank means the line does not exist on that side.
#
# A line is `{"kind": "add" | "del" | "same" | "hunk", "text", "old"?, "new"?}`.
def diff_view(lines, o = {})
  dv_gutter = o["gutter"] ?? 44
  dv_drawn = range(0, lines.length()).map(fn(i) {
    l = lines[i]
    kind = (l["kind"] ?? "same").to_s
    # A hunk header carries its own `@@`; signing it again reads as `@@@`.
    sign = kind == "add" ? "+" : (kind == "del" ? "-" : " ")
    wash = kind == "add" ? "success.subtle" : (kind == "del" ? "danger.subtle" : (kind == "hunk" ? "surface.sunken" : "none"))
    ink = kind == "hunk" ? "text.muted" : "text.default"
    keyed((o["key"] ?? "diff") + ":" + i.to_s, {
      "k": "box",
      "s": {"display": "row", "gap": 0, "width": "100%", "bg": wash, "pad": [0, 2, 0, 2]},
      "p": {"role": "row", "label": (kind == "add" ? "added " : (kind == "del" ? "removed " : "")) + l["text"].to_s},
      "c": [
        text((l["old"] ?? "").to_s, {"font": "mono", "size": 0, "fg": "text.muted", "width": dv_gutter}),
        text((l["new"] ?? "").to_s, {"font": "mono", "size": 0, "fg": "text.muted", "width": dv_gutter}),
        text(sign, {"font": "mono", "size": 1, "fg": ink, "width": 14, "weight": "semibold"}),
        text(l["text"], {"font": "mono", "size": 1, "fg": ink, "grow": 1})
      ]
    })
  })
  dv_whole = column({"gap": 0, "width": "100%", "radius": 2, "overflow": "clip", "border": 1, "border_color": "border.subtle"}, dv_drawn)
  dv_whole["p"] = {"role": "group", "label": o["label"] ?? "Changes"}
  dv_whole
end

# How many lines a patch adds and takes away, for a header beside it.
def diff_tally(lines)
  added = lines.filter(fn(l) { (l["kind"] ?? "").to_s == "add" }).length()
  removed = lines.filter(fn(l) { (l["kind"] ?? "").to_s == "del" }).length()
  {"added": added, "removed": removed}
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
  # `o["key"]` and `o["props"]` are how two selects that answer the same
  # event tell themselves apart. Without them the key is the event name, so
  # a second select wearing it silently restyles the first (07 §3), and
  # neither the toggle nor the pick can say which row it came from -- which
  # is exactly what a filter builder, or any list of rows with a select in
  # it, has to be able to say.
  props = o["props"] ?? {}
  anchor = {
    "k": "box",
    "key": o["key"] ?? ("sel:" + on_toggle.to_s),
    "s": s,
    "on": stateful(s, TONES["neutral"], {"click": on_toggle}),
    "c": [text(value, {"grow": 1}), icon(
      "chevron_down",
      {"fg": "text.muted", "width": 14, "height": 14}
    )]
  }
  anchor["p"] = props if props.keys().length() > 0
  dropdown(anchor, options.map(fn(opt) { select_option(opt, opt == value, on_pick, min_width, props) }), open, DROPDOWN_MAX_PX, on_toggle)
end

def select_option(label, selected, on_pick, min_width, props = {})
  {
    "k": "box",
    "s": {
      "pad": [1, 3, 1, 3],
      "radius": 1,
      "min_width": min_width,
      "bg": selected ? "surface.sunken" : "none",
      "cursor": "pointer"
    },
    "p": props.merge({"value": label}),
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
# How wide a popover may get. See the note in `dropdown`.
DROPDOWN_MAX_W = 320

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
# `on_close` is what makes it light-dismissable (03 §1). A press outside the
# widget — outside the `stack` below, which is the anchor and the panel
# together — reaches the overlay as `blur`, and the client decides that
# without asking, because the client owns the hand. A panel that passes
# nothing here is not dismissible and is never closed behind the
# application's back.
#
# For a widget whose anchor *toggles*, the toggle is the right thing to pass:
# the panel is open, so toggling it is closing it, and no second handler has
# to exist. A press on the anchor is inside the widget and sends no `blur`,
# so the toggle still fires once and not twice.
def dropdown(anchor, content, open, max_px = 0, on_close = "")
  return anchor unless open

  pane = {"gap": 0}
  pane["max_height"] = max_px if max_px > 0
  panel = {
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
        # A cap, because a popover with no width of its own comes out as wide
        # as the window: it is measured against the window (03 §5, so its
        # options are not clipped to a narrow field) and an `auto` width then
        # fills that bound instead of shrinking to the options. Left alone the
        # panel spanned all 1400 px and `settle_anchored` clamped it to x=0,
        # so it hung under the whole page rather than under its field. The cap
        # is a stopgap over a layout question, not the answer to it: the panel
        # should shrink-wrap and take the field's width as its floor.
        "max_width": DROPDOWN_MAX_W,
        "z": 5
      },
      "c": [scroll(pane, content)]
  }
  panel["on"] = {"blur": on_close} unless on_close.to_s.blank?
  stack({"gap": 0}, [anchor, panel])
end

# The keys the filter field hands back: arrows walk the panel, Escape
# shuts it. `Enter` is a `submit`, same as `tag_field`.
COMBO_KEYS = ["ArrowDown", "ArrowUp", "Escape"]

# What of a fixed list still belongs under the draft. An empty draft offers
# everything, because a panel that appears only once you have typed is a
# panel most people never learn is there — the same reason `tag_suggest`
# does.
def combo_filter(options, query)
  said = (query ?? "").strip().downcase()
  return options if said == ""

  options.filter(fn(o) { o.to_s.downcase().index_of(said) >= 0 })
end

# A select you can type into. Closed, it is its value and a chevron;
# open, a field at the top of the panel filters the options. The caller
# owns `open`, `query` and `at`, the way it owns a select's `open`.
#
# `o`: `query`, `open`, `at`, `on_toggle`, `on_change`, `on_pick`, `on_key`,
# `on_submit`, `on_close`, `key`, `label`, `density`, `width`, `min_width`.
def combobox(options, value, o = {})
  cb_key = (o["key"] ?? ("combo:" + (o["label"] ?? value).to_s)).to_s
  cb_open = o["open"] == true
  cb_query = o["query"] ?? ""
  cb_at = o["at"] ?? -1
  cb_min = o["min_width"] ?? 160
  words = combo_filter(options, cb_query)
  s = {
    "display": "row",
    "align": "center",
    "gap": 2,
    "pad": [2, 3, 2, 3],
    "min_width": cb_min,
    "min_height": field_height(o),
    "border": 1,
    "border_color": "border.default",
    "radius": 2,
    "bg": "surface.sunken",
    "cursor": "pointer",
    "transition": "fast"
  }
  s["width"] = o["width"] unless o["width"].nil?
  s["grow"] = 1 if o["grow"] == true
  anchor = {
    "k": "box",
    "key": cb_key,
    "s": s,
    "p": {
      "role": "combo_box",
      "expanded": cb_open,
      "label": o["label"] ?? value.to_s
    },
    "on": stateful(s, TONES["neutral"], {"click": o["on_toggle"]}),
    "c": [text(value, {"grow": 1}), icon(
      "chevron_down",
      {"fg": "text.muted", "width": 14, "height": 14}
    )]
  }
  return anchor unless cb_open

  cb_props = {
    "keys": COMBO_KEYS,
    "role": "combo_box",
    "expanded": true,
    "label": o["label"] ?? value.to_s,
    "autofocus": true
  }
  cb_props["active_descendant"] = cb_key + ":opt:" + words[cb_at].to_s if cb_at >= 0 && cb_at < words.length()
  cb_on = {}
  cb_on["key_down"] = o["on_key"] unless o["on_key"].nil?
  cb_on["submit"] = o["on_submit"] unless o["on_submit"].nil?
  cb_entry = input(cb_query, o["on_change"], {
    "key": cb_key + ":entry",
    "style": {
      "width": "100%",
      "border": 0,
      "bg": "none",
      "pad": [1, 2, 1, 2]
    },
    "props": cb_props,
    "on": cb_on
  })
  cb_rows = range(0, words.length()).map(fn(i) {
    word = words[i]
    lit = i == cb_at
    control({
      "key": cb_key + ":opt:" + word.to_s,
      "size": "sm",
      "tone": "quiet",
      "shape": {
        "justify": "start",
        "min_width": cb_min,
        "bg": lit ? "surface.sunken" : "none",
        "border": [0, 0, 0, 3],
        "border_color": lit ? "accent.base" : "none"
      },
      "on": {"click": o["on_pick"]},
      "props": {"value": word, "id": word},
      "a11y": {
        "role": "option",
        "selected": lit,
        "label": word.to_s,
        "pos_in_set": i + 1,
        "set_size": words.length()
      },
      "c": [text(word.to_s, {"weight": lit ? "semibold" : "regular"})]
    })
  })
  cb_panel = column({"gap": 0}, [cb_entry].concat(cb_rows.length() == 0 ? [muted("Nothing matches")] : cb_rows))
  cb_panel["p"] = {"role": "list_box", "label": (o["label"] ?? "Options").to_s}
  dropdown(anchor, [cb_panel], true, DROPDOWN_MAX_PX, o["on_close"] ?? o["on_toggle"] ?? "")
end

# What you can do to the rows you have ticked.
#
# It appears because something is ticked and leaves when nothing is, which
# is the whole interaction: a bar that is always there is a toolbar, and a
# toolbar cannot say "3 of 63". The count comes from `selection_count`, so
# it is right under the "all except these" reading too -- tick the header,
# untick two rows, and it says 61 without anybody enumerating 61 ids.
#
# `o["scope_total"]` is how many rows the query behind it has, which is not
# the same as how many are on screen: "select all 63" is an offer about the
# query, and taking it is what `selection_all` means.
#
# The actions are `{"label", "event", "tone"?, "icon"?}`. Each carries the
# selection's scope, because an action on "all except these" that arrived
# without its scope would be an action on a different question's rows.
def bulk_bar(sel, total, actions, on_clear, o = {})
  bb_count = selection_count(sel, total)
  return spacer() if bb_count == 0

  bb_scope = selection_scope(sel)
  bb_said = bb_count == 1 ? "1 selected" : bb_count.to_s + " selected"
  bb_offer = selection_all?(sel) || bb_count >= total || o["on_all"].nil?
    ? []
    : [control({
        "key": (o["key"] ?? "bulk") + ":all",
        "size": "sm", "tone": "ghost",
        "on": {"click": o["on_all"]},
        "props": {"scope": bb_scope},
        "a11y": {"role": "button", "label": "Select all " + total.to_s},
        "c": [text("Select all " + total.to_s, {"size": 0, "weight": "semibold"})]
      })]
  # Keyed by position and label, not by event. Several actions may answer
  # the same event and tell themselves apart by their props -- "Chase",
  # "Export" and "Archive" all sending `demo_bulk` is the ordinary case --
  # and keying by the event gives all three the same key, which 07 §3 refuses
  # outright: two children of one box cannot share one.
  bb_doing = range(0, actions.length()).map(fn(bb_i) {
    a = actions[bb_i]
    control({
      "key": (o["key"] ?? "bulk") + ":" + bb_i.to_s + ":" + (a["label"] ?? "").to_s,
      "size": "sm",
      "tone": a["tone"] ?? "neutral",
      "on": {"click": a["event"]},
      # The label rides along so a handler shared by several actions can say
      # which one was pressed.
      "props": {"scope": bb_scope, "count": bb_count, "action": a["label"]},
      "a11y": {"role": "button", "label": a["label"].to_s},
      "c": (a["icon"] ?? "") == ""
        ? [text(a["label"], {"size": 0, "weight": "semibold"})]
        : [icon(a["icon"], {"width": 14, "height": 14}), text(a["label"], {"size": 0, "weight": "semibold"})]
    })
  })
  bb_bar = row({
    "gap": 2, "align": "center", "width": "100%", "wrap": "wrap",
    "pad": [2, 3, 2, 3], "radius": 2,
    "bg": "info.subtle", "border": 1, "border_color": "border.subtle",
    "animation": "enter", "transition": "fast"
  }, [text(bb_said, {"weight": "semibold"})].concat(bb_offer).concat([spacer()]).concat(bb_doing).concat([
    icon_button("close", on_clear, {"scope": bb_scope}, {
      "icon": "close", "name": "Clear the selection", "size": "sm", "key": (o["key"] ?? "bulk") + ":clear"
    })
  ]))
  bb_bar["p"] = {"role": "toolbar", "label": bb_said}
  keyed(o["key"] ?? "bulk", bb_bar)
end

# ---- A question you can come back to -----------------------------------------
#
# A filter builder answers one question at a time and forgets it. Most of
# the questions worth asking are asked again next week, which is what a
# saved view is: a name, and the condition that was in the builder when the
# name was given.
#
# A view is `{"id", "name", "filter"}`. The model below is the whole of it --
# adding, dropping and finding by id -- because the condition is already a
# value and a list of them needs nothing cleverer.

def views_add(views, id, name, tree)
  (views ?? []).filter(fn(v) { v["id"] != id }).concat([{"id": id, "name": name, "filter": tree}])
end

def views_drop(views, id)
  (views ?? []).filter(fn(v) { v["id"] != id })
end

def views_find(views, id)
  found = (views ?? []).filter(fn(v) { v["id"] == id })
  found.length() == 0 ? null : found[0]
end

# The saved views, as a row of chips, with the one that is up lit.
#
# `o["dirty"]` is the honest part: the condition in the builder has been
# edited since the view was chosen, so the chip is still lit but no longer
# describes what is on screen. Saying so is the difference between a saved
# view and a decoration -- otherwise the name goes on claiming a question
# that has since been changed underneath it.
def saved_views(views, current, on_pick, o = {})
  sv_chips = (views ?? []).map(fn(v) {
    lit = v["id"].to_s == (current ?? "").to_s
    row({"gap": 0, "align": "center", "shrink": 0}, [
      control({
        "key": (o["key"] ?? "view") + ":" + v["id"].to_s,
        "size": "sm",
        "tone": "quiet",
        "selected": lit,
        "shape": {"radius": 2, "pad": [1, 3, 1, 3], "min_width": 0, "gap": 1},
        "on": {"click": on_pick},
        "props": {"id": v["id"]},
        "a11y": {"role": "tab", "label": v["name"].to_s, "selected": lit},
        "c": [text(v["name"], {"size": 0, "weight": lit ? "semibold" : "regular"})].concat(
          lit && o["dirty"] == true ? [text("·", {"size": 0, "fg": "warning.base", "weight": "bold"})] : []
        )
      })
    ].concat(o["on_drop"].nil? ? [] : [icon_button("close", o["on_drop"], {"id": v["id"]}, {
      "icon": "close", "name": "Forget " + v["name"].to_s, "size": "sm", "key": (o["key"] ?? "view") + ":x:" + v["id"].to_s
    })]))
  })
  sv_tail = o["on_save"].nil? ? [] : [control({
    "key": (o["key"] ?? "view") + ":save",
    "size": "sm", "tone": "ghost",
    "on": {"click": o["on_save"]},
    "a11y": {"role": "button", "label": o["save_label"] ?? "Save this view"},
    "c": [text(o["save_label"] ?? "Save this view", {"size": 0, "weight": "semibold"})]
  })]
  sv_strip = row({"gap": 2, "align": "center", "wrap": "wrap", "width": "100%"}, sv_chips.concat(sv_tail))
  sv_strip["p"] = {"role": "tab_list", "label": o["label"] ?? "Saved views"}
  sv_strip
end

# ---- A question, built out of smaller questions -----------------------------
#
# Every list in this catalogue can be filtered by whatever the application
# thought of in advance: a status, a month, a search box. That covers the
# questions someone anticipated and none of the ones they did not, and the
# gap shows the moment anybody asks something like "late, or unpaid and over
# ten thousand". A filter builder is how a person asks a question the author
# never wrote down.
#
# The condition is a tree, and the tree is the server's:
#
#   group  {"op": "and" | "or", "items": [...]}
#   leaf   {"field", "cmp", "value"}
#
# Nothing here holds state. Every control sends a `path` -- the dotted index
# of the item it belongs to, `""` for the root, `"1.0"` for the first item of
# the second -- and the handler walks that path and changes what it finds.
# That is what makes the whole thing answerable from one server event per
# act, and what `filter_path_*` below are for.

# The item at `path` in `tree`, or `null`.
def filter_at(tree, path)
  return tree if path.to_s == ""

  here = tree
  steps = path.to_s.split(".")
  i = 0
  while i < steps.length()
    items = here["items"] ?? []
    at = int(steps[i])
    return null if at < 0 || at >= items.length()

    here = items[at]
    i = i + 1
  end
  here
end

# `tree` with `f` applied to the item at `path`. A rebuild rather than a
# mutation, because a group holds its children by value and editing one in
# place would leave the copy the view is drawing untouched.
def filter_edit(tree, path, f)
  return f(tree) if path.to_s == ""

  steps = path.to_s.split(".")
  at = int(steps[0])
  rest = steps.slice(1, steps.length()).join(".")
  items = tree["items"] ?? []
  return tree if at < 0 || at >= items.length()

  out = range(0, items.length()).map(fn(i) { i == at ? filter_edit(items[i], rest, f) : items[i] })
  tree.merge({"items": out})
end

# `tree` without the item at `path`. The root cannot be removed.
def filter_drop(tree, path)
  return tree if path.to_s == ""

  steps = path.to_s.split(".")
  return filter_edit(tree, steps.slice(0, steps.length() - 1).join("."), fn(g) {
    at = int(steps[steps.length() - 1])
    kept = range(0, (g["items"] ?? []).length()).filter(fn(i) { i != at }).map(fn(i) { g["items"][i] })
    g.merge({"items": kept})
  }) if steps.length() > 1

  at = int(steps[0])
  kept = range(0, (tree["items"] ?? []).length()).filter(fn(i) { i != at }).map(fn(i) { tree["items"][i] })
  tree.merge({"items": kept})
end

# An empty condition, for "Add condition" to put somewhere.
def filter_blank(fields, cmps)
  {"field": (fields[0] ?? "").to_s, "cmp": (cmps[0] ?? "is").to_s, "value": ""}
end

def filter_group_blank(fields, cmps)
  {"op": "and", "items": [filter_blank(fields, cmps)]}
end

# Whether an item is a group. A leaf has no `items`, and that is the only
# difference worth testing: an empty group is still a group.
def filter_group?(it)
  !(it["items"]).nil?
end

# Does this row answer the question?
#
# The half that was missing. A filter builder that only edits a tree draws a
# question nobody asks; this is what asks it. `row` is a hash and a leaf's
# `field` is a key in it -- no schema, no registry, because the fields the
# builder offers came from the caller in the first place and the caller is
# the one that knows what they mean.
#
# Comparison is on strings unless both sides read as numbers, which is the
# rule that stops "Amount > 9" matching 10 000 by alphabet. `number_valid?`
# is the same judgement the number field makes, so a value that a field
# would have refused is compared the way it was typed.
def filter_match?(row, node)
  return true if node.nil?

  return filter_group_match?(row, node) if filter_group?(node)

  said = (row[(node["field"] ?? "").to_s] ?? "").to_s
  want = (node["value"] ?? "").to_s
  cmp = (node["cmp"] ?? "is").to_s
  return said.downcase().index_of(want.downcase()) >= 0 if cmp == "contains"
  return said.downcase() == want.downcase() if cmp == "is"
  return said.downcase() != want.downcase() if cmp == "is not"

  # `>` and `<`. Numbers when both sides are numbers, text otherwise -- and
  # an empty draft matches nothing rather than everything, or a half-typed
  # condition would empty the list under the hands typing it.
  return false if want == ""

  both = number_valid?(said) && number_valid?(want)
  return (both ? said.to_f > want.to_f : said > want) if cmp == ">"
  return (both ? said.to_f < want.to_f : said < want) if cmp == "<"

  true
end

def filter_group_match?(row, group)
  items = group["items"] ?? []
  # An empty group asks nothing, and a question nobody asked excludes nobody.
  return true if items.length() == 0

  return items.filter(fn(it) { filter_match?(row, it) }).length() == items.length() if (group["op"] ?? "and").to_s == "and"

  items.filter(fn(it) { filter_match?(row, it) }).length() > 0
end

# The rows that answer it.
def filter_apply(rows, tree)
  rows.filter(fn(r) { filter_match?(r, tree) })
end

# How a condition reads as a sentence, for a chip that stands for it.
def filter_says(node)
  return "everything" if node.nil?

  return filter_group_says(node) if filter_group?(node)

  ((node["field"] ?? "").to_s + " " + (node["cmp"] ?? "").to_s + " " + (node["value"] ?? "").to_s).strip()
end

def filter_group_says(group)
  items = group["items"] ?? []
  return "everything" if items.length() == 0

  items.map(fn(it) { filter_says(it) }).join((group["op"] ?? "and").to_s == "and" ? " and " : " or ")
end

# The tree, drawn.
#
# `o`: `fields`, `cmps`, `open` (the key of whichever select is up, or ""),
# and the events -- `on_op`, `on_add`, `on_add_group`, `on_remove`,
# `on_open`, `on_field`, `on_cmp`, `on_value`. Every one of them is handed
# `params["props"]["path"]`.
#
# Depth is drawn as a rule down the left rather than as indentation alone.
# Three levels of padding all look like one level of padding by the time the
# rows are 32 px tall, and what matters here is precisely which `or` a
# condition belongs to.
def filter_builder(tree, o = {})
  fields = o["fields"] ?? []
  cmps = o["cmps"] ?? ["is", "is not", "contains", ">", "<"]
  whole = filter_group_view(tree ?? filter_group_blank(fields, cmps), "", 0, o)
  whole["p"] = {"role": "group", "label": o["label"] ?? "Filter"}
  whole
end

def filter_group_view(group, path, depth, o)
  fgv_fields = o["fields"] ?? []
  fgv_cmps = o["cmps"] ?? ["is", "is not", "contains", ">", "<"]
  fgv_items = group["items"] ?? []
  fgv_root = path.to_s == ""
  fgv_head = row({"gap": 2, "align": "center", "width": "100%"}, [
    segmented_at(["and", "or"], (group["op"] ?? "and").to_s, o["on_op"], path, depth),
    muted(fgv_items.length() == 1 ? "1 condition" : fgv_items.length().to_s + " conditions"),
    spacer(),
    control({
      "key": "fb:add:" + depth.to_s + ":" + path.to_s,
      "size": "sm", "tone": "ghost",
      "on": {"click": o["on_add"]},
      "props": {"path": path},
      "a11y": {"role": "button", "label": "Add condition"},
      "c": [text("Add condition", {"size": 0, "weight": "semibold"})]
    }),
    control({
      "key": "fb:grp:" + depth.to_s + ":" + path.to_s,
      "size": "sm", "tone": "ghost",
      "on": {"click": o["on_add_group"]},
      "props": {"path": path},
      "a11y": {"role": "button", "label": "Add group"},
      "c": [text("Add group", {"size": 0, "weight": "semibold"})]
    })
  ].concat(fgv_root ? [] : [icon_button("close", o["on_remove"], {"path": path}, {
    "icon": "close", "name": "Remove group", "size": "sm", "key": "fb:rmg:" + depth.to_s + ":" + path.to_s
  })]))
  fgv_kids = range(0, fgv_items.length()).map(fn(i) {
    here = (fgv_root ? "" : path.to_s + ".") + i.to_s
    here = i.to_s if fgv_root
    it = fgv_items[i]
    filter_group?(it)
      ? filter_group_view(it, here, depth + 1, o)
      : filter_leaf_view(it, here, depth + 1, o, fgv_fields, fgv_cmps)
  })
  column({
    "gap": 2, "width": "100%",
    "pad": fgv_root ? 0 : [2, 0, 2, 3],
    "border": fgv_root ? 0 : [0, 0, 0, 2],
    "border_color": "border.subtle"
  }, [fgv_head].concat(fgv_kids))
end

def filter_leaf_view(leaf, path, depth, o, fields, cmps)
  flv_open = (o["open"] ?? "").to_s
  keyed("fb:row:" + path.to_s, row({"gap": 2, "align": "center", "width": "100%", "wrap": "wrap"}, [
    select_sized(fields, (leaf["field"] ?? "").to_s, flv_open == path.to_s + ":field", o["on_open"], o["on_field"], 140, false, {
      "key": "fb:f:" + path.to_s, "props": {"path": path, "slot": "field"}, "density": o["density"]
    }),
    select_sized(cmps, (leaf["cmp"] ?? "").to_s, flv_open == path.to_s + ":cmp", o["on_open"], o["on_cmp"], 110, false, {
      "key": "fb:c:" + path.to_s, "props": {"path": path, "slot": "cmp"}, "density": o["density"]
    }),
    input((leaf["value"] ?? "").to_s, o["on_value"], {
      "key": "fb:v:" + path.to_s,
      "style": {"grow": 1, "min_width": 120},
      "props": {"path": path, "label": "Value"}
    }),
    icon_button("close", o["on_remove"], {"path": path}, {
      "icon": "close", "name": "Remove condition", "size": "sm", "key": "fb:rm:" + path.to_s
    })
  ]))
end

# `segmented`, but every cell says which group it belongs to. The plain one
# sends the option alone, which is all a page with one segmented control
# needs and not enough for a tree of them.
def segmented_at(options, selected, on_select, path, depth)
  cells = range(0, options.length()).map(fn(i) {
    opt = options[i]
    lit = opt == selected
    control({
      "key": "fb:op:" + depth.to_s + ":" + path.to_s + ":" + opt.to_s,
      "size": "sm", "tone": "quiet", "selected": lit,
      "shape": {"radius": 1, "pad": [0, 2, 0, 2], "min_width": 0, "bg": lit ? "surface.raised" : "none", "border": lit ? 1 : 0},
      "on": {"click": on_select},
      "props": {"path": path, "option": opt},
      "a11y": {"role": "tab", "label": opt.to_s, "selected": lit, "pos_in_set": i + 1, "set_size": options.length()},
      "c": [text(opt.to_s.upcase(), {"size": 0, "weight": lit ? "bold" : "regular", "fg": lit ? "text.default" : "text.muted"})]
    })
  })
  row({"gap": 1, "pad": 1, "radius": 2, "bg": "surface.sunken", "shrink": 0}, cells)
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
  dropdown(msd_anchor, [msd_panel], true, DROPDOWN_MAX_PX, on_toggle)
end

# ---- Slider ----------------------------------------------------------------

# A 240 px track. A press sets the value from the pointer x and a drag
# follows it; once the track has focus (Tab reaches it through its click
# handler) the arrow keys nudge it. The server owns the value: `on_set`
# receives `params["kind"]` — "click", "pointer_down", "pointer_move",
# "pointer_up", or "key_down" — and `params["payload"]`.
# How many out of how many, as stars you can click.
#
# Every star is its own control, which is what makes it answerable from a
# keyboard and legible to a screen reader: five buttons in a `radio_group`,
# each saying what it would set. A single node with a pointer offset would
# be one target with five meanings, and no way to say which one has focus.
#
# `value` is the server's. A star lights when its position is at or under it,
# so the fill reads left to right without any state on this side.
# Two handles on one track: from and to.
#
# The track is declared, not arranged (03 §3.4). `track: "x"` says this box
# is one; `track_min`, `track_max` and `track_step` say what it measures;
# `track_value` says where the handles are; the children say which part of
# it they are. Everything between the press and the release is the client's
# after that -- which handle the press took, where it goes, the thumb under
# the pointer every frame -- and the only thing that comes back is a
# `change`, and only when the quantised value has moved.
#
# What that replaces was five handlers and a round trip per mouse move. The
# server was told where the pointer was, inverted it against a `width` baked
# into the props -- a lie the moment a parent stretched the track -- worked
# out on this side which end the hand was nearest, and sent a whole tree
# back so the handles could catch up. The moves arrived whether the value
# had changed or not, and dragging this widget cost 39 ms a frame against
# the 9 of a page without one.
#
# There is no arithmetic left here because the only width that was ever true
# is the one the client laid out. There is no `drag_only` either, and
# nothing for it to gate: the prop exists for a node that must hear
# `pointer_move` while it is dragged, and this one never hears it at all.
# What must never come back is the handler declared only while a drag is
# live -- an event in flight naming a handler the server has since removed
# is refused by 06 §4, and the session goes with it.
#
# The filled run is between the handles rather than from the left, which is
# the whole visual difference between "up to here" and "between here and
# here".
def range_slider(low, high, min, max, on_set, o = {})
  {
    "k": "box",
    "key": o["key"] ?? "range",
    "s": {"display": "row", "align": "center", "width": o["width"] ?? 240, "height": 24, "cursor": "grab"},
    "p": {
      "track": "x",
      "track_min": min,
      "track_max": max,
      "track_step": o["step"] ?? 1,
      "track_value": [low, high],
      "role": "group",
      "label": o["label"] ?? "Range"
    },
    "on": {"change": on_set},
    "c": [
      track_part("groove", {"height": 4, "bg": "surface.sunken", "radius": 4}),
      track_thumb(o["low_label"] ?? "From", low, min, high),
      track_part("fill", {"height": 4, "bg": "accent.base", "radius": 4}),
      track_thumb(o["high_label"] ?? "To", high, low, max)
    ]
  }
end

# One piece of a track for the client to place: the line, or the run of it
# the value covers. It is sized from the value every frame, so the width
# here is only what it has before the first paint.
def track_part(part, style)
  {"k": "box", "s": style, "p": {"track_part": part}, "c": []}
end

# A handle. It is its own focus stop and its own `slider` to a reader, with
# its own bounds -- the handle beside it, which is how two of them cannot
# cross without a line of code saying so.
def track_thumb(label, at, floor_v, ceil_v)
  {
    "k": "box",
    "s": {
      "width": 14, "height": 14, "radius": 4, "shrink": 0,
      "bg": "accent.base", "border": 2, "border_color": "surface.base"
    },
    "p": {
      "track_part": "thumb",
      "role": "slider",
      "label": label,
      "value_now": at,
      "value_min": floor_v,
      "value_max": ceil_v,
      "orientation": "horizontal"
    },
    "c": []
  }
end

# Money, typed.
#
# The ERP shows amounts on every screen and offers nowhere to type one, and
# the reason a plain `number_field` is not enough is the symbol: a field that
# says `1500` where the column beside it says `1 500,00 €` makes the person
# do the conversion, and doing it wrong is expensive in exactly this kind of
# application. The unit sits in the field, inside the border, so it reads as
# part of the value and not as a label that might belong to something else.
#
# What is typed is what is sent. Grouping a number while a caret is inside it
# moves the caret, and a field that moves your caret while you type is a
# field you cannot type in; the grouped form belongs beside it, and
# `o["hint"]` is where this puts it.
def currency_field(label, value, on_change, o = {})
  cf_unit = (o["unit"] ?? "€").to_s
  cf_said = (value ?? "").to_s
  cf_error = field_error(value, fn(x) { number_valid?(x) }, o["complaint"] ?? "That is not an amount", o)
  cf_bad = field_bad(cf_error, o)
  cf_props = field_props(label, cf_error, cf_bad, o)
  cf_box = input(cf_said, on_change, {
    "key": o["key"].nil? ? null : o["key"].to_s,
    "style": {"width": "auto", "grow": 1, "border": 0, "bg": "none", "pad": [1, 2, 1, 2], "align": "end"},
    "props": cf_props
  })
  cf_shell = row({
    "gap": 1, "align": "center", "width": o["width"] ?? "100%",
    "min_height": field_height(o), "pad": [0, 3, 0, 3], "radius": 2,
    "border": 1, "border_color": cf_bad ? "danger.base" : "border.default",
    "bg": "surface.sunken"
  }, (o["unit_after"] == true ? [cf_box, muted(cf_unit)] : [muted(cf_unit), cf_box]))
  field_shell(label, cf_shell, o.merge({"error": cf_error}))
end

def rating(value, on_set, o = {})
  # Not `max`: builtin.
  rt_max = o["max"] ?? 5
  rt_now = value ?? 0
  rt_size = o["size"] ?? 18
  rt_ro = o["read_only"] == true
  rt_stars = range(1, rt_max + 1).map(fn(i) {
    lit = i <= rt_now
    glyph = icon("star", {
      "width": rt_size,
      "height": rt_size,
      "fg": lit ? (o["tone"] ?? "warning.base") : "border.default"
    })
    return restyle(glyph, {"shrink": 0}) if rt_ro

    control({
      "key": (o["key"] ?? "rating") + ":" + i.to_s,
      "size": "sm",
      "tone": "quiet",
      "shape": {"pad": 0, "min_width": 0, "bg": "none", "border": 0, "height": rt_size + 4},
      "on": {"click": on_set},
      "props": {"value": i, "id": i},
      "a11y": {
        "role": "radio",
        "label": i.to_s + " of " + rt_max.to_s,
        "checked": i == rt_now,
        "pos_in_set": i,
        "set_size": rt_max
      },
      "c": [glyph]
    })
  })
  rt_strip = row({"gap": rt_ro ? 0 : 1, "align": "center", "shrink": 0}, rt_stars)
  rt_strip["p"] = {
    "role": rt_ro ? "group" : "radio_group",
    "label": o["label"] ?? "Rating",
    "value_now": rt_now,
    "value_min": 0,
    "value_max": rt_max
  }
  rt_strip
end

# One key, drawn as the cap it is printed on. `kbd("Ctrl")`, not `Ctrl` in
# prose: a shortcut sheet that sets its keys in body text is a sheet you have
# to read rather than scan.
def kbd(key)
  {
    "k": "box",
    "s": {
      "pad": [0, 2, 0, 2], "radius": 1, "min_width": 22,
      "display": "row", "justify": "center", "align": "center",
      "bg": "surface.sunken", "border": 1, "border_color": "border.default", "shrink": 0
    },
    "c": [text(key.to_s, {"size": 0, "weight": "semibold", "fg": "text.muted"})]
  }
end

# What the application claims of the keyboard, on one screen.
#
# It belongs in the catalogue rather than in each application because the
# list is not decoration: 03 §3.1 says a node claims keys, and an application
# that claims `Ctrl+K` without ever saying so has a shortcut only its author
# knows about. This is where it says so.
#
# `groups` are `{"name", "keys": [{"keys": ["Ctrl", "K"], "does": "..."}]}`.
def shortcut_sheet(groups, on_close, o = {})
  ss_body = groups.map(fn(g) {
    lines = (g["keys"] ?? []).map(fn(k) {
      row({"gap": 3, "align": "center", "width": "100%"}, [
        text(k["does"].to_s, {"grow": 1}),
        row({"gap": 1, "align": "center", "shrink": 0}, (k["keys"] ?? []).map(fn(cap) { kbd(cap) }))
      ])
    })
    column({"gap": 2, "width": "100%"}, [muted(g["name"].to_s)].concat(lines))
  })
  dialog(
    o["title"] ?? "Keyboard shortcuts",
    [column({"gap": 4, "width": "100%"}, ss_body)],
    [secondary_button(o["done"] ?? "Close", on_close)],
    {"width": o["width"] ?? 420, "key": "shortcuts", "on_close": on_close}
  )
end

def slider(value, min, max, on_set, o = {})
  {
    "k": "box",
    "s": {"display": "row", "align": "center", "width": o["width"] ?? 240, "height": 24, "cursor": "grab"},
    "p": {
      "track": "x",
      "track_min": min,
      "track_max": max,
      "track_step": o["step"] ?? 1,
      "track_value": value,
      "role": "slider",
      "label": o["label"] ?? "Value",
      "value_now": value,
      "value_min": min,
      "value_max": max,
      "orientation": "horizontal"
    },
    "on": {"change": on_set},
    "c": [
      track_part("groove", {"height": 4, "bg": "surface.sunken", "radius": 4}),
      track_part("fill", {"height": 4, "bg": "accent.base", "radius": 4}),
      {
        "k": "box",
        "s": {
          "width": 16, "height": 16, "radius": 4, "shrink": 0,
          "bg": "accent.base", "border": 2, "border_color": "surface.base"
        },
        "p": {"track_part": "thumb"},
        "c": []
      }
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

# An input of a stated width. The width goes in through the style `input` is
# handed, not onto `box["s"]` afterwards: `input` builds its hover and focus
# styles out of that style, and a width written on after the fact reaches the
# resting state alone. The field then collapsed to its own text the moment the
# pointer touched it — 200 px at rest, 78 hovered — and stayed collapsed,
# because the style it goes back to on leaving had no width either.
def sized_input(value, on_change, width)
  input(value, on_change, {"style": {"width": width}})
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

# A line of anything, painted as marks. The client reads `secret` (03 §3);
# without it this is a text field whose value is on screen, which is the
# whole of why a login cannot be composed from `text_field`.
#
# `o["shown"]` is the reveal: true paints the text, false (the default)
# paints the marks. `o["on_reveal"]` is the event the Show/Hide control
# sends. The handler owns both; this widget holds neither.
def password_field(label, value, on_change, o = {})
  error = field_error(value, fn(said) { true }, "", o)
  bad = field_bad(error, o)
  shown = o["shown"] == true
  props = field_props(label, error, bad, o)
  props["secret"] = true unless shown
  reveal = o["on_reveal"] ?? ""
  box = input(value, on_change, {
    "style": field_style(bad, o).merge(reveal == "" ? {} : {"width": "auto", "grow": 1}),
    "props": props
  })
  # Not `control`: a bare assignment rebinds the global of that name, and
  # `control` is the function every widget below is built on (line 564). The
  # first field with a Show button turned it into a node, and the next
  # `control(...)` — `ghost_button`, `icon_button`, anything — raised
  # "Cannot call non-function value" from inside a view, which reaches a
  # window as a blank screen and no error.
  body = reveal == "" ? box : row(
    {"gap": 2, "align": "center", "width": o["width"] ?? "100%"},
    [box, ghost_button(shown ? "Hide" : "Show", reveal)]
  )
  field_shell(label, body, o.merge({"error": error}))
end

# Six boxes, one code. The value is a prefix of length 0..n; the live cell
# is the next empty one, or the last when the code is full. Typing a digit
# appends and the live cell moves — the old input is gone, so `autofocus`
# can take the new one (03 §3.1: a later batch will not yank focus that is
# already settled). Backspace on an empty cell is reported (nothing to
# delete) and pops the prefix; Backspace on the last filled cell is local
# and arrives as `change` once the field has gone quiet.
#
# Paste of more than one character replaces. `o["numeric"]` defaults true.
# `o["secret"]` paints marks, for a PIN. `o["on_input"]` hears `text_input`,
# `change`, `key_down` and a click on a filled cell (jump: keep the prefix
# up to there). `o["take_focus"]` and `o["gen"]` are how the handler asks
# for the caret on the batch that answered, and how it throws away a local
# edit the code refused.
OTP_KEYS = ["Backspace"]

def otp_clean(text, o = {})
  n = o["digits"] ?? 6
  numeric = o["numeric"] != false
  said = (text ?? "").to_s
  out = ""
  i = 0
  while i < said.length() && out.length() < n
    ch = said.substring(i, i + 1)
    ok = numeric ? "0123456789".includes?(ch) : (ch != " " && ch != "\n")
    out = out + ch if ok
    i = i + 1
  end
  out
end

def otp_take(value, text, o = {})
  incoming = otp_clean(text, o)
  return value if incoming == ""
  return incoming if incoming.length() != 1

  otp_clean((value ?? "").to_s + incoming, o)
end

def otp_pop(value)
  said = (value ?? "").to_s
  return "" if said.length() <= 1

  said.substring(0, said.length() - 1)
end

def otp_jump(value, i)
  said = (value ?? "").to_s
  return said if i.nil?
  return "" if i <= 0
  return said if i >= said.length()

  said.substring(0, i)
end

def otp_apply(value, kind, payload, props, o = {})
  if kind == "text_input"
    return otp_take(value, payload.to_s, o)
  end
  if kind == "change"
    cell = otp_clean(payload.to_s, {"digits": 1, "numeric": o["numeric"]})
    return otp_pop(value) if cell == ""
    return value
  end
  if kind == "key_down"
    key = payload.class == "array" ? payload[0] : payload.to_s
    return otp_pop(value) if key == "Backspace"
    return value
  end
  return otp_jump(value, props["i"] ?? 0) if kind == "click"

  value
end

def otp_field(label, value, o = {})
  otp_n = o["digits"] ?? 6
  otp_said = (value ?? "").to_s
  otp_at = o["at"] ?? otp_said.length()
  otp_at = otp_n - 1 if otp_at >= otp_n
  otp_at = 0 if otp_at < 0
  otp_key = (o["key"] ?? ("otp:" + label)).to_s
  otp_on = o["on_input"] ?? ""
  otp_cells = range(0, otp_n).map(fn(i) {
    otp_cell(i, otp_said, otp_at, otp_n, otp_key, otp_on, o)
  })
  otp_row = {
    "k": "box",
    "s": {"display": "row", "gap": 2, "align": "center"},
    "p": {"role": "group", "label": label},
    "c": otp_cells
  }
  field_shell(label, otp_row, o)
end

def otp_cell(i, said, at, n, key, on_input, o)
  ch = i < said.length() ? said.substring(i, i + 1) : ""
  secret = o["secret"] == true
  mark = (secret && ch != "") ? "•" : ch
  live = i == at
  shell = {
    "display": "row",
    "justify": "center",
    "align": "center",
    "width": 44,
    "height": 48,
    "shrink": 0,
    "radius": 2,
    "border": 1,
    "border_color": live ? "accent.base" : "border.default",
    "bg": "surface.sunken"
  }
  return otp_live_cell(i, ch, n, key, on_input, o, shell) if live

  {
    "k": "box",
    "key": key + ":x:" + i.to_s,
    "s": shell.merge({"cursor": "pointer"}),
    "on": {"click": on_input},
    "p": {
      "i": i,
      "role": "button",
      "label": "Digit " + (i + 1).to_s + " of " + n.to_s
    },
    "c": [text(mark == "" ? " " : mark, {"weight": "semibold", "size": 4})]
  }
end

def otp_live_cell(i, ch, n, key, on_input, o, shell)
  props = {
    "keys": OTP_KEYS,
    "label": "Digit " + (i + 1).to_s + " of " + n.to_s,
    "i": i
  }
  props["secret"] = true if o["secret"] == true
  props["autofocus"] = true if o["take_focus"] == true
  # Not `editable()`: that restyles on focus, and a restyle that does not
  # carry `text_align` would put the caret back on the left of the box.
  # The shell already wears the live border.
  inner = {
    "k": "input",
    "t": ch,
    "key": key + ":in:" + i.to_s + ":" + (o["gen"] ?? 0).to_s,
    "s": {
      "width": 42,
      "height": 46,
      "pad": 0,
      "border": 0,
      "bg": "none",
      "text_align": "center",
      "size": 4,
      "weight": "semibold"
    },
    "p": props,
    "on": {
      "change": on_input,
      "text_input": on_input,
      "key_down": on_input
    }
  }
  {"k": "box", "s": shell, "c": [inner]}
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

# ---- Files -----------------------------------------------------------------
#
# The two things a server cannot do at all: reach a file on the person's
# machine, and show one back.
#
# A picker is never one thing. 03 §3.2 asks for three at once and a node that
# has only two of them opens nothing, silently: the `pick` prop, a **server**
# handler for `file_pick`, and a capability the person granted. That is the
# whole reason this is a builder — the prop and the handler are easy to write
# and easy to write only one of, and the failure is a button that does
# nothing with no diagnostic anywhere (08 §3, deliberately).
#
# The third is not ours to give. `fs.pick`, `camera` and `microphone` are
# three different powers and none implies another, so a component asks for
# what it uses in `eui_capabilities(...)` and the person still answers.

# `flags` in a `pick`. Bit 0 takes more than one file; bit 1 asks the camera
# for a picture that does not exist yet and bit 2 asks for a recording —
# which is why they need their own grants. Together they are a contradiction
# and a client resolves them as the camera, so do not send both.
PICK_MANY = 1
PICK_CAMERA = 2
PICK_MICROPHONE = 4

# `"png,jpg"` on its own, or `[accept, flags, max]` when anything else is
# asked for. `max` of 0 means the client's own default (16 MiB), which is
# also what it uses for a list that does not say.
def pick_prop(accept, o)
  pick_flags = o["flags"] ?? 0
  pick_flags = pick_flags + PICK_MANY if o["multiple"] == true
  return accept if pick_flags == 0 && o["max"].nil?

  [accept, pick_flags, o["max"] ?? 0]
end

# A control that opens the platform's open dialog.
#
# Everything `control` gives every other control — tones, sizes, the
# disabled and loading states, the a11y mapping — and two more options:
#
#   accept    "png,jpg,pdf", extensions without dots, empty for anything
#   flags     PICK_CAMERA or PICK_MICROPHONE; omit for a file already there
#   multiple  more than one file, which the two capture flags ignore
#   max       the largest one file may be, in bytes
#
# `on_pick` is the **server** handler name for `file_pick`, and what it
# receives is `[id, name, size]` — a name and a weight, never a path. The
# bytes arrive later and separately, as the server's own `file_upload`.
def file_field(label, accept, on_pick, o = {})
  ff_node = control(o.merge({
    "key": o["key"] ?? ("file:" + label),
    "on": {"file_pick": on_pick},
    "a11y": o["a11y"] ?? {"role": "button", "label": label},
    "c": o["c"] ?? [text_interned(label, {"size": o["text_size"] ?? 1})]
  }))
  ff_node["p"] = (ff_node["p"] ?? {}).merge({"pick": pick_prop(accept, o)})

  # A tool button is a glyph that lights rather than a surface that fills, so
  # it wants a hover the TONES table has no name for. `control` looks its tone
  # up by name, so the override is applied here, against the style `control`
  # settled on — and only when there are handlers to replace, since `disabled`
  # and `loading` mean there are deliberately none.
  unless ff_node["on"].nil? || (o["hover"].nil? && o["press"].nil?)
    ff_node["on"] = stateful(ff_node["s"], {
      "hover": o["hover"] ?? {},
      "press": o["press"] ?? {}
    }, {"file_pick": on_pick})
  end
  ff_node
end

# A surface a file can be let go over. `drop` is the same prop as `pick`
# (03 §3.2), so a file arrives as `file_pick` whether it was chosen in the
# dialog or dropped here. `o["on_drag"]` is `file_drag`, whose payload is
# `[over]`: the box lights when a file is over it and goes dark when it
# leaves. `o["pick"]` (default true) also opens the dialog on a click, so
# one box is both gestures.
#
# `o["over"]` is whether a file is over it *now* — the handler stores what
# `file_drag` said; this widget holds no state.
def file_drop(label, accept, on_pick, o = {})
  over = o["over"] == true
  hint = o["hint"] ?? ""
  # A ground and a foreground are one decision. `over` fills the box with
  # `accent.hover`, so the words on it have to be `accent.on` -- the theme
  # guarantees that pair and guarantees nothing about `text.default` on a
  # filled accent, which in the light theme is dark grey on blue and reads
  # as a box that has gone wrong rather than one that is ready.
  fd_ink = over ? "accent.on" : "text.default"
  resting = {
    "display": "column",
    "gap": 1,
    "align": "center",
    "justify": "center",
    "width": "100%",
    "pad": 5,
    "radius": 3,
    "border": 1,
    "border_color": over ? "accent.base" : "border.subtle",
    "bg": over ? "accent.hover" : "surface.sunken",
    "fg": fd_ink,
    "transition": "fast"
  }
  on = {"file_pick": on_pick}
  on["file_drag"] = o["on_drag"] unless o["on_drag"].nil?
  pick = o["pick"] != false
  props = {
    "drop": pick_prop(accept, o),
    "role": "button",
    "label": label
  }
  props["pick"] = pick_prop(accept, o) if pick
  n = {
    "k": "box",
    "key": o["key"] ?? ("drop:" + label),
    "s": resting,
    "p": props,
    "on": on,
    "c": [
      text(over ? (o["over_label"] ?? "Let go to add them") : label, {"weight": "semibold", "fg": fd_ink})
    ].concat(hint == "" ? [] : [text(hint, {"size": 1, "fg": over ? "accent.on" : "text.muted"})])
  }
  # A box that opens a dialog on a click and answered a pointer with nothing
  # was a button that did not look like one. The hover is local, so it
  # costs no round trip; while a file is actually over the box the two
  # states would fight, so it is left off then and `over` has the box.
  n["on"] = stateful(resting, {
    "hover": {"bg": "surface.raised", "border_color": "border.strong"},
    "press": {"bg": "surface.sunken"}
  }, on) if pick && !over
  n
end

# What was attached, drawn as a card.
#
# Three cards, because there are three things there can be to show.
#
#   o["src"]    a small square of the file itself — an asset from
#               `eui_asset(bytes)`, or a path under `public/`
#   o["badge"]  a node for the square when there is no picture to put in it,
#               usually the extension set in small bold type
#   neither     the name and the note, in a row padded where the square
#               would have been
#
# The last is not a fallback nobody reaches: it is what a picture whose bytes
# have gone gets, and the reason the card survives that at all. A `src` that
# names nothing is a view that cannot be encoded, which ends the session
# (01 §4) — so the caller resolves the bytes first and passes what it got.
#
# The name is not decoration either. A picture the client cannot decode is an
# error nowhere — the server puts bytes on the wire and the client fails to
# make an image of them — so a card that was only a picture drew an empty box
# and said nothing about what was in it.
ATTACHMENT_PX = 74

def attachment_card(name, note, o = {})
  ac_edge = o["size"] ?? ATTACHMENT_PX
  ac_square = {
    "width": ac_edge - 2,
    "height": ac_edge - 2,
    "shrink": 0,
    "overflow": "clip",
    "bg": "surface.sunken",
    "display": "row",
    "justify": "center",
    "align": "center"
  }

  ac_stamp = []
  unless o["badge"].nil?
    ac_stamp = [{
      "k": "box",
      "s": ac_square.merge({"width": 40, "height": 40, "radius": 1, "margin": [0, 0, 0, 3]}),
      "c": [o["badge"]]
    }]
  end
  unless o["src"].nil?
    # `image` does not scale a picture to its box: it draws at the size the
    # style asks for and anything larger is clipped, so what is handed here
    # is a square made on the way in and not the file squeezed at render.
    ac_stamp = [{"k": "box", "s": ac_square, "c": [image(o["src"], ac_edge - 2, ac_edge - 2)]}]
  end

  # A ternary's condition has to type as Bool and `.nil?` on a value out of
  # an untyped hash is Any, so this is an `if` and not `?:`.
  ac_pad = [0, 0, 0, 0]
  ac_pad = [0, 3, 0, 3] if o["src"].nil?

  row(
    {
      "gap": 3,
      "align": "center",
      "height": ac_edge,
      "pad": ac_pad,
      "radius": 2,
      "overflow": "clip",
      "border": 1,
      "border_color": "border.subtle",
      "bg": "surface.raised",
      "margin": [1, 0, 0, 0]
    },
    ac_stamp.concat([
      column(
        {"gap": 0, "grow": 1, "shrink": 1, "min_width": 0, "pad": [0, 3, 0, 0]},
        [
          text(name.to_s, {"weight": "semibold", "size": 1, "clamp": 1, "fg": "text.default"}),
          muted(note.to_s)
        ]
      )
    ])
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
  field_shell(label, open ? dropdown(anchor, [make()], true, 0, o["on_toggle"] ?? "") : anchor, o)
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
