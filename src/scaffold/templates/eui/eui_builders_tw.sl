# EUI view builders, part 6: Tailwind-style class strings.
#
#   tw("flex items-center gap-3 rounded-lg bg-white px-4 py-2 shadow-sm
#       hover:bg-gray-50")
#
# answers
#
#   {"s": base, "hover": {...}, "press": {...}, "focus": {...},
#    "disabled": {...}, "props": {...}, "gaps": {}, "divide": {}}
#
# `s` is an ordinary EUI style hash. The four states are *deltas* over it, the
# same shape a `TONES` entry has, so a `tw()` result can be handed to
# `stateful()` as a tone. `props` carries what Tailwind says in a class and EUI
# says in a prop — only `grid-cols-N` so far. `gaps` and `divide` are what a
# class says about the children rather than the box (`space-y-4`, `divide-y`):
# they need the box's final direction and its children, so `node()` settles
# them, and a bare `tw()` settles `gaps` from the classes' own `flex`/`flex-col`
# and refuses `divide`.
#
# It is plain Soli and adds nothing to the wire: every class becomes the style
# keys and values spec 02 already has, colours become the roles of spec 05, and
# spacing becomes indices into the space scale, which the client multiplies by
# the viewer's density. A class with no honest equivalent raises, naming the
# class and saying why, rather than being dropped: a page that silently loses
# half its classes looks almost right, which is the expensive kind of wrong.
#
# `sm:` to `2xl:` need the viewport's width, which the view is given and tw()
# is not: `tw(classes, width)`, `tw_style(classes, false, width)`, or `"vw":
# width` beside `"tw"` on a node. Without it a breakpoint class raises.
#
# `doc/docs/eui/tailwind.md` has the table. Part of the reference catalogue —
# `eui_builders.sl` has the header that explains the whole of it; `node()`,
# `text()`, `stateful()` and `control()` there take a `"tw"` string or a
# `tw_style()` and call in here.
#
# Every local below carries its function's prefix. A bare assignment in a
# callee writes the caller's variable of the same name, and these functions
# call each other a dozen deep.

# The words of a class string. A list is taken as already split, so a view can
# build its classes conditionally without joining them first.
def tw_split(classes)
  return [] if classes.nil?

  sp_list = classes.class == "array" ? classes : classes.to_s.replace("\n", " ").replace("\t", " ").split(" ")
  sp_list.map(fn(w) { w.to_s.strip() }).filter(fn(w) { w != "" })
end

# Parsing a class string is a quarter of a millisecond, and a view says the
# same string on every row of every render; so each distinct string is parsed
# once per process and copied out after that. Copied, because a caller may
# write into what it is given -- `column()` sets `display` on its style -- and a
# shared hash would carry that write into every later node. The memo stops
# growing at `TW_MEMO_CAP` strings, so a view that builds its classes out of
# data (`"w-[" + str(px) + "px]"`) costs a parse each time and nothing more.
# A string with a breakpoint in it is kept once per breakpoint the width falls
# in, never per width: a window dragged across 300 widths is six entries.
TW_MEMO = {}
TW_MEMO_CAP = 1024

# The classes as a style, with `gaps` settled against the classes' own
# direction and nothing left that only a node can place.
def tw(classes, width = nil)
  tw_t = tw_raw(classes, width)
  tw_dv = tw_t["divide"]
  if tw_dv.keys().length() > 0
    throw tw_no(tw_dv["c"], "a divider borders the children, so it is written where they are: node(), row() or column() with \"tw\"")
  end
  if tw_t["gaps"].keys().length() > 0
    tw_dir = tw_t["s"]["display"]
    if tw_dir.nil?
      tw_first = tw_t["gaps"][tw_t["gaps"].keys()[0]]["c"]
      throw tw_no(tw_first, "spacing children depends on which way they run: write flex or flex-col with it, or put it on row() or column()")
    end
    tw_t["s"] = tw_gap_settle(tw_t["s"], tw_t["gaps"], tw_dir)
    tw_t["gaps"] = {}
  end
  tw_t
end

# What was parsed, memoised, before anything is settled against a node.
def tw_raw(classes, width)
  twr_key = classes.class == "array" ? classes.join(" ") : classes.to_s
  twr_rank = tw_screen(width)
  twr_key = twr_key + " @" + str(twr_rank) if tw_responsive?(twr_key)
  twr_hit = TW_MEMO[twr_key]
  return tw_copy(twr_hit) unless twr_hit.nil?

  twr_fresh = tw_parse(classes, twr_rank)
  TW_MEMO[twr_key] = twr_fresh if TW_MEMO.keys().length() < TW_MEMO_CAP
  tw_copy(twr_fresh)
end

def tw_copy(t)
  {
    "s": t["s"].merge({}),
    "hover": t["hover"].merge({}),
    "press": t["press"].merge({}),
    "focus": t["focus"].merge({}),
    "disabled": t["disabled"].merge({}),
    "props": t["props"].merge({}),
    "gaps": t["gaps"].merge({}),
    "divide": t["divide"].merge({})
  }
end

def tw_blank()
  {"s": {}, "hover": {}, "press": {}, "focus": {}, "disabled": {}, "props": {}, "gaps": {}, "divide": {}}
end

# ---- Breakpoints -------------------------------------------------------------

# Tailwind's screens, the same rungs as `BP`: sm 640, md 768, lg 1024, xl 1280,
# 2xl 1536. A rank is how many of them a width has passed; -1 is no width.
def tw_screen(width)
  return -1 if width.nil?
  return 5 if width >= 1536
  return 4 if width >= 1280
  return 3 if width >= 1024
  return 2 if width >= 768
  return 1 if width >= 640

  0
end

def tw_screen_rank(prefix)
  return 1 if prefix == "sm"
  return 2 if prefix == "md"
  return 3 if prefix == "lg"
  return 4 if prefix == "xl"
  return 5 if prefix == "2xl"

  0
end

def tw_screen_px(rank)
  return 640 if rank == 1
  return 768 if rank == 2
  return 1024 if rank == 3
  return 1280 if rank == 4

  1536
end

# Whether a string may hold a breakpoint, so its memo entry names the width's
# rank. A false positive costs an entry, never a wrong answer.
def tw_responsive?(key)
  key.index_of("sm:") >= 0 || key.index_of("md:") >= 0 || key.index_of("lg:") >= 0 || key.index_of("xl:") >= 0
end

# `md:hover:bg-gray-50` as its breakpoint rank, its state and its class. The
# prefixes may come in either order, as in Tailwind; one of each at most.
def tw_word(word)
  twwd_bits = word.split(":")
  twwd_bp = 0
  twwd_state = "s"
  twwd_said = ""
  for twwd_i in range(0, twwd_bits.length() - 1)
    twwd_pre = twwd_bits[twwd_i]
    twwd_rank = tw_screen_rank(twwd_pre)
    if twwd_rank > 0
      throw tw_no(word, "one breakpoint per class; the larger one alone says the same") if twwd_bp > 0
      twwd_bp = twwd_rank
    else
      throw tw_no(word, "one state per class; hover:focus: is two states at once") if twwd_state != "s"
      twwd_state = tw_variant(twwd_pre, word)
      twwd_said = twwd_pre
    end
  end
  twwd_name = twwd_bits[twwd_bits.length() - 1]
  {"bp": twwd_bp, "state": twwd_state, "name": twwd_name, "whole": word, "noop": tw_focus_ring?(twwd_said, twwd_name)}
end

# The classes that style a focus ring. The client draws its own -- 2 px in
# `focus.ring`, outside the border box, when focus came from the keyboard
# (03 §3) -- which is what `focus-visible:` asks for, so these say nothing it
# does not already do: `outline-none` anywhere, any `outline-*` under `focus:`
# or `focus-visible:`, and any `ring-*` under `focus-visible:`. A `focus:ring-2`
# is still a border, the way `ring-2` at rest is.
def tw_focus_ring?(said, name)
  return true if name == "outline-none"
  return true if (said == "focus" || said == "focus-visible") && name.starts_with?("outline")
  return true if said == "focus-visible" && name.starts_with?("ring")

  false
end

def tw_needs_width(whole, rank)
  "tw: '" + whole + "' needs the viewport width — a breakpoint class applies from " + str(tw_screen_px(rank)) + " px up, and tw() was not given one: write tw(classes, width), tw_style(classes, false, width), or \"vw\": width beside \"tw\" on a node"
end

# Mobile first, as Tailwind's stylesheet orders it: every resting class, the
# unprefixed ones and then each breakpoint the width has reached from the
# smallest up, then the states in the same order -- so `p-2 md:p-4` is 4 from
# 768 px, whatever order the two were written in, and `hover:pt-4` starts from
# the resting sides at this width. A breakpoint the width has not reached is
# still parsed, so a refused class raises at every width, not only on a wide
# screen.
def tw_parse(classes, rank)
  twp_words = tw_split(classes).map(fn(w) { tw_word(w) })
  if rank < 0
    for twp_w in twp_words
      throw tw_needs_width(twp_w["whole"], twp_w["bp"]) if twp_w["bp"] > 0
    end
  end
  twp_out = tw_blank()
  for twp_pass in ["rest", "state"]
    for twp_level in range(0, 6)
      for twp_w in twp_words
        twp_stated = twp_w["state"] != "s"
        if twp_w["noop"] != true && twp_w["bp"] == twp_level && twp_stated == (twp_pass == "state")
          if twp_level <= rank || twp_level == 0
            twp_out = tw_take(twp_out, twp_w["state"], twp_w["name"], twp_w["whole"])
          else
            tw_take(tw_blank(), twp_w["state"], twp_w["name"], twp_w["whole"])
          end
        end
      end
    end
  end
  twp_out
end

# Just the resting style, with the `disabled:` delta laid over it when asked.
# A text transform (`uppercase`) stays in it as `tw_case`, which `text()` reads
# and removes; it is not a style key, and the encoder would refuse it.
def tw_style(classes, disabled = false, width = nil)
  ts_all = tw(classes, width)
  return ts_all["s"].merge(ts_all["disabled"]) if disabled == true

  ts_all["s"]
end

# Whether a result declares any state at all. A node without one needs no
# handlers, and should not be given four that do nothing.
def tw_stateful?(t)
  st_n = t["hover"].keys().length() + t["press"].keys().length() + t["focus"].keys().length()
  st_n > 0
end

# What `node(kind, {"tw": ..., "vw": width}, children)` does with the string.
# The style's own keys win over the classes, so a builder can pin `display`
# after the fact the way `column` and `row` do; spacing is settled against that
# final direction, and a divider is laid onto the children.
def tw_node(n)
  tn_style = n["s"]
  tn_t = tw_raw(tn_style["tw"], tn_style["vw"])
  tn_rest = {}
  for tn_key in tn_style.keys()
    tn_rest[tn_key] = tn_style[tn_key] unless tn_key == "tw" || tn_key == "vw"
  end
  tn_base = tn_t["s"].merge(tn_rest)
  tn_case = tn_base["tw_case"]
  unless tn_case.nil?
    throw tw_no(tw_case_class(tn_case), "a text transform changes a string, and a box has none: put it on the text, text(s, tw_style(\"" + tw_case_class(tn_case) + "\"))")
  end
  tn_base = tw_gap_settle(tn_base, tn_t["gaps"], tn_base["display"] ?? "row")
  n["s"] = tn_base
  n["p"] = (n["p"] ?? {}).merge(tn_t["props"]) if tn_t["props"].keys().length() > 0
  n["c"] = tw_divide(n["c"] ?? [], tn_t["divide"]) if tn_t["divide"].keys().length() > 0
  n["on"] = tw_wire(tn_base, tn_t, n["on"] ?? {}) if tw_stateful?(tn_t)
  n
end

# Local handlers for the states a result declares, and none for the ones it
# does not. `self` names the node a chunk runs on (07 §1), so this needs no
# key; a node that is restyled from outside afterwards wants one anyway.
def tw_wire(base, t, on)
  tww_hover = base.merge(t["hover"] ?? {})
  tww_press = tww_hover.merge(t["press"] ?? {})
  tww_focus = base.merge(t["focus"] ?? {})
  tww_out = on.merge({})
  if (t["hover"] ?? {}).keys().length() > 0 || (t["press"] ?? {}).keys().length() > 0
    tww_out = tww_out.merge({
      "pointer_enter": {"local": "self.style = @hover", "styles": {"hover": tww_hover}},
      "pointer_leave": {"local": "self.style = @base", "styles": {"base": base}},
      "pointer_down": {"local": "self.style = @active", "styles": {"active": tww_press}},
      "pointer_up": {"local": "self.style = @hover", "styles": {"hover": tww_hover}}
    })
  end
  if (t["focus"] ?? {}).keys().length() > 0
    tww_out = tww_out.merge({
      "focus": {"local": "self.style = @focus", "styles": {"focus": tww_focus}},
      "blur": {"local": "self.style = @base", "styles": {"base": base}}
    })
  end
  tww_out
end

# ---- Errors ------------------------------------------------------------------

def tw_no(whole, why)
  "tw: '" + whole + "' has no EUI equivalent — " + why
end

def tw_unknown(whole)
  "tw: unknown class '" + whole + "' — not one tw() knows; the table is doc/docs/eui/tailwind.md"
end

def tw_variant(prefix, whole)
  return "hover" if prefix == "hover"
  return "press" if prefix == "active"
  return "focus" if prefix == "focus"
  return "disabled" if prefix == "disabled"
  # The client draws the keyboard ring itself; what else a focus-visible:
  # class changes is laid on whenever the node has focus, as focus: is.
  return "focus" if prefix == "focus-visible"

  tv_why = "a variant tw() does not know; hover:, active:, focus:, focus-visible: and disabled: are the states, sm: to 2xl: the breakpoints"
  tv_why = "breakpoints are mobile first: write the narrow classes bare and the wider ones under sm:, md:, lg:, xl: or 2xl:" if prefix.starts_with?("max-") || prefix.starts_with?("min-")
  tv_why = "colours are roles and already follow the viewer's theme; drop the dark: classes" if prefix == "dark"
  tv_why = "a local handler restyles the node it is on, not an ancestor; focus: on the field, and a key on the box if it must change too" if prefix == "focus-within"
  tv_why = "there are no group or peer states: a local handler restyles one node, by key" if prefix.starts_with?("group") || prefix.starts_with?("peer")
  tv_why = "there are no structural selectors: style the first or last child where it is built" if ["first", "last", "odd", "even", "only"].includes?(prefix)
  tv_why = "there are no pseudo-elements: build the node" if ["before", "after", "placeholder", "file", "marker", "selection"].includes?(prefix)
  throw tw_no(whole, tv_why)
end

# Families refused by prefix, with the reason. Only reached after the exact
# table, so `rounded-sm` is found there before `rounded-s` refuses it here.
def tw_refused(name)
  rf_why = {
    "tracking-": "EUI has no letter-spacing; the 64-byte style record has no byte for it",
    "leading-": "line height comes with the text size; there is no independent line-height",
    "bg-gradient": "EUI has no gradients",
    "from-": "EUI has no gradients",
    "via-": "EUI has no gradients",
    "to-": "EUI has no gradients",
    "ring-offset": "a ring is written as a border, and a border has no offset",
    "rounded-t": "radius is one byte for all four corners",
    "rounded-b": "radius is one byte for all four corners",
    "rounded-l": "radius is one byte for all four corners",
    "rounded-r": "radius is one byte for all four corners",
    "rounded-s": "radius is one byte for all four corners",
    "rounded-e": "radius is one byte for all four corners",
    "translate-": "EUI has no transforms",
    "rotate-": "EUI has no transforms",
    "scale-": "EUI has no transforms",
    "skew-": "EUI has no transforms",
    "origin-": "EUI has no transforms",
    "transform": "EUI has no transforms",
    "inset-": "there are no offsets; an absolute node is placed by the stack it is in",
    "top-": "there are no offsets; an absolute node is placed by the stack it is in",
    "bottom-": "there are no offsets; an absolute node is placed by the stack it is in",
    "left-": "there are no offsets; an absolute node is placed by the stack it is in",
    "right-": "there are no offsets; an absolute node is placed by the stack it is in",
    "outline": "the client draws its own focus ring; there is no outline",
    "ring-inset-": "a ring is written as a border",
    "blur": "there are no filters; backdrop-blur is the one blur",
    "drop-shadow": "there are no filters; shadow-sm, shadow-md and shadow-lg are the shadows",
    "brightness-": "there are no filters",
    "contrast-": "there are no filters",
    "grayscale": "there are no filters",
    "invert": "there are no filters",
    "saturate-": "there are no filters",
    "sepia": "there are no filters",
    "hue-rotate-": "there are no filters",
    "ease-": "the client has one easing curve; transition and duration-N are what there is",
    "delay-": "a transition is a duration and never a delay; stagger by grafting nodes over time",
    "order-": "children are drawn in the order they are given",
    "col-": "a grid places its children in order; there are no spans",
    "row-": "a grid places its children in order; there are no spans",
    "grid-rows-": "a grid has columns and rows follow; write grid-cols-N",
    "grid-flow-": "a grid fills by row",
    "auto-cols-": "a grid's columns are equal shares",
    "auto-rows-": "a grid's rows are as tall as their content",
    "place-": "write items-* and justify-* on the box",
    "content-": "a wrapped row packs its lines from the start",
    "justify-items-": "write items-* on the box",
    "justify-self-": "write self-* on the child",
    "whitespace-": "text wraps at its box's width; truncate keeps it to one line with an ellipsis, or give the box the width",
    "break-": "text wraps at the box edge, and clamp decides how far",
    "aspect-": "there is no aspect ratio; give the box a width and a height",
    "object-": "an image fills the box it is given",
    "pointer-events-": "the topmost node takes the pointer, handler or not, and an event walks up from it, never through it to a sibling below",
    "select-": "only editable nodes select text; select-none is what every other node already is",
    "appearance-": "there is no native appearance to reset",
    "resize": "a textarea grows with its content",
    "overflow-x-": "overflow is one value for both axes; scrolling is a scroll() node",
    "overflow-y-": "overflow is one value for both axes; scrolling is a scroll() node",
    "decoration-": "an underline takes the text's colour and weight",
    "underline-offset-": "an underline sits where the client draws it",
    "list-": "there are no list markers; build the bullet",
    "fill-": "an icon takes fg; write text-*",
    "stroke-": "an icon takes fg; write text-*",
    "sr-only": "there is no visually hidden text; put the label in props",
    "not-sr-only": "there is no visually hidden text; put the label in props",
    "backdrop-": "backdrop-blur is the one backdrop filter",
    "animate-": "the one animation is animate-spin; entrances are the enter animation",
    "font-": "the weights are font-normal, font-medium, font-semibold and font-bold, and the faces font-sans and font-mono",
    "shadow-": "shadow-sm, shadow, shadow-md, shadow-lg and shadow-xl are the shadows, and a shadow takes no colour",
    "rounded-": "rounded-none, -sm, rounded, -md, -lg, -xl, -2xl, -3xl and -full are the radii",
    "cursor-": "the cursors are pointer, default, text, wait, not-allowed, grab, grabbing, col-resize and row-resize",
    "overflow-": "overflow-hidden, overflow-clip and overflow-visible; scrolling is a scroll() node",
    "transition-": "transition, transition-colors, transition-all, transition-opacity, transition-shadow and transition-none"
  }
  for rf_prefix in rf_why.keys()
    return rf_why[rf_prefix] if name.starts_with?(rf_prefix)
  end
  ""
end

# Classes refused by exact name.
def tw_refused_exact(name)
  return "an auto margin along the parent's line pushes its siblings away, and there are no auto margins: put spacer() before the node (ml-auto, mt-auto) or after it (mr-auto, mb-auto), or justify-between on the parent" if ["mt-auto", "mr-auto", "mb-auto", "ml-auto"].includes?(name)
  return "an auto margin on both axes centres the node both ways: self-center on it, and justify-center on its parent" if name == "m-auto"
  return "there are no positioning schemes; absolute is the one there is, inside a stack" if name == "fixed" || name == "sticky"
  return "there is no inline flow; a box is flex, flex-col, block, grid or hidden, and text wraps inside its own node" if ["inline", "inline-block", "table", "contents", "flow-root"].includes?(name)
  return "the client ships no italic face" if name == "italic" || name == "not-italic"
  return "Inter's figures are drawn proportional and the client selects no OpenType feature; font-mono sets figures that line up" if name == "tabular-nums" || name == "proportional-nums" || name == "lining-nums" || name == "oldstyle-nums"
  return "children are drawn in the order they are given; reverse the list" if name == "flex-row-reverse" || name == "flex-col-reverse"
  return "the view is given the viewport's size; use it" if ["w-screen", "h-screen", "min-h-screen", "min-w-screen", "max-w-screen"].includes?(name)
  return "a bare ring is three pixels; write ring-1 or ring-2, which become a border" if name == "ring"
  return "there is no container query and no typography plugin; give the box a width" if name == "container" || name == "prose"
  return "a shadow is cast, never inset" if name == "shadow-inner"
  return "a border is always solid; border-0 removes it" if ["border-dashed", "border-dotted", "border-double", "border-solid", "border-none"].includes?(name)

  ""
end

# ---- Scales ------------------------------------------------------------------

# Tailwind's spacing steps, and the index of the space scale (05 §2) each one
# is. Only the steps both scales share: 1.5 is 6 px, and the space scale has 4
# and 8, so it is an error rather than a guess.
def tw_space_steps()
  {"0": 0, "0.5": 1, "1": 2, "2": 3, "3": 4, "4": 5, "5": 6, "6": 7, "8": 8, "10": 9, "12": 10, "16": 11, "24": 12}
end

def tw_text_sizes()
  {"xs": 0, "sm": 1, "base": 2, "lg": 3, "xl": 4, "2xl": 5, "3xl": 6, "4xl": 7}
end

def tw_max_widths()
  {"xs": 320, "sm": 384, "md": 448, "lg": 512, "xl": 576, "2xl": 672, "3xl": 768, "4xl": 896, "5xl": 1024, "6xl": 1152, "7xl": 1280}
end

def tw_space(value, whole)
  sc_steps = tw_space_steps()
  sc_ix = sc_steps[value]
  return sc_ix unless sc_ix.nil?

  throw tw_no(whole, "'" + value + "' is not on the space scale; " + tw_space_near(value) + "the steps are " + sc_steps.keys().join(", "))
end

# "the nearest are 1 (4 px) and 2 (8 px); " for a step between two the scale
# has, so a half step is one edit away from a class that is.
def tw_space_near(value)
  return "" unless tw_numeric?(value)

  sn_want = float(value)
  sn_below = ""
  sn_above = ""
  for sn_step in tw_space_steps().keys()
    sn_at = float(sn_step)
    sn_below = sn_step if sn_at < sn_want
    sn_above = sn_step if sn_at > sn_want && sn_above == ""
  end
  return "" if sn_below == "" && sn_above == ""
  return "the nearest is " + sn_below + " (" + str(int(float(sn_below) * 4.0)) + " px); " if sn_above == ""
  return "the nearest is " + sn_above + " (" + str(int(float(sn_above) * 4.0)) + " px); " if sn_below == ""

  "the nearest are " + sn_below + " (" + str(int(float(sn_below) * 4.0)) + " px) and " + sn_above + " (" + str(int(float(sn_above) * 4.0)) + " px); "
end

def tw_numeric?(value)
  nm_chars = value.chars()
  return false if nm_chars.length() == 0

  nm_dots = 0
  for nm_c in nm_chars
    if nm_c == "."
      nm_dots = nm_dots + 1
    elsif !("0123456789".index_of(nm_c) >= 0)
      return false
    end
  end
  nm_dots <= 1 && value != "."
end

# `[320px]`, `[50%]`: the arbitrary values tw() takes for a length.
def tw_bracket(value, whole)
  br_inner = value.substring(1, value.length() - 1)
  if br_inner.ends_with?("px")
    br_n = br_inner.substring(0, br_inner.length() - 2)
    return int(float(br_n).round()) if tw_numeric?(br_n)
  end
  if br_inner.ends_with?("%")
    br_p = br_inner.substring(0, br_inner.length() - 1)
    return br_p + "%" if tw_numeric?(br_p)
  end
  throw tw_no(whole, "an arbitrary length is [Npx] or [N%]")
end

# A width or a height: N x 4 px, a fraction, full, auto, px or [Npx].
def tw_length(value, whole)
  return "100%" if value == "full"
  return "auto" if value == "auto"
  return 1 if value == "px"
  return tw_bracket(value, whole) if value.starts_with?("[") && value.ends_with?("]")

  if value.index_of("/") > 0
    ln_parts = value.split("/")
    if ln_parts.length() == 2 && tw_numeric?(ln_parts[0]) && tw_numeric?(ln_parts[1]) && float(ln_parts[1]) > 0
      ln_pct = float(ln_parts[0]) * 100.0 / float(ln_parts[1])
      return str(int(ln_pct.round())) + "%"
    end
  end
  if tw_numeric?(value)
    ln_px = int((float(value) * 4.0).round())
    return ln_px if ln_px <= 65535
  end
  throw tw_no(whole, "a length is N (N x 4 px), a fraction, full, auto, px or [Npx]")
end

# ---- Colours -----------------------------------------------------------------

def tw_roles()
  [
    "surface.base", "surface.raised", "surface.sunken", "surface.overlay",
    "text.default", "text.muted", "text.inverted", "text.disabled",
    "accent.base", "accent.hover", "accent.active", "accent.on",
    "success.base", "success.subtle", "success.on",
    "warning.base", "warning.subtle", "warning.on",
    "danger.base", "danger.subtle", "danger.on",
    "info.base", "info.subtle", "info.on",
    "border.subtle", "border.default", "border.strong",
    "focus.ring",
    "series.1", "series.2", "series.3", "series.4", "series.5"
  ]
end

# The semantic spellings, which are the roles themselves: `bg-accent`,
# `text-muted`, `border-subtle`, `bg-danger-subtle`, `bg-surface-raised`.
def tw_role_alias(prefix, value)
  ra_dotted = value.replace("-", ".")
  return ra_dotted if tw_roles().includes?(ra_dotted)
  return value + ".base" if ["accent", "danger", "success", "warning", "info"].includes?(value)
  return "text." + value if ["muted", "disabled", "inverted"].includes?(value) && prefix == "text"
  return "surface." + value if ["raised", "sunken", "overlay"].includes?(value)
  return "surface.base" if value == "surface"
  return "text.default" if value == "default" && prefix == "text"
  return "border." + value if ["subtle", "default", "strong"].includes?(value) && (prefix == "border" || prefix == "ring")

  ""
end

def tw_neutral?(family)
  ["gray", "slate", "zinc", "neutral", "stone"].includes?(family)
end

def tw_status(family)
  return "danger" if family == "red" || family == "rose"
  return "success" if family == "green" || family == "emerald"
  return "warning" if family == "yellow" || family == "amber"
  return "info" if family == "blue" || family == "sky"

  ""
end

# The nearest role, for the error that refuses a palette colour.
def tw_nearest(family)
  return "accent (indigo-600), or series-1 to series-5 for data" if ["purple", "violet", "fuchsia", "pink"].includes?(family)
  return "warning (yellow-600)" if family == "orange"
  return "success (green-600) or info (blue-600)" if ["teal", "cyan", "lime"].includes?(family)

  "a role: accent, danger, success, warning, info, or the grays"
end

def tw_hex2(n)
  hx_digits = "0123456789ABCDEF"
  hx_n = n
  hx_n = 0 if hx_n < 0
  hx_n = 255 if hx_n > 255
  hx_digits[hx_n / 16] + hx_digits[hx_n % 16]
end

def tw_hex?(value)
  return false unless value.length() == 6 || value.length() == 8

  for hx_c in value.chars()
    return false unless "0123456789abcdefABCDEF".index_of(hx_c) >= 0
  end
  true
end

# `bg-black/50`, `ring-gray-900/5`: a colour with an opacity. EUI's roles are
# opaque, so only the two uses Tailwind UI makes of this are taken — a scrim
# behind a dialog, which is a literal, and the faint ring round a card, which
# is `border.subtle`.
def tw_alpha(prefix, value, whole)
  al_parts = value.split("/")
  al_base = al_parts[0]
  al_pct = al_parts.length() == 2 && tw_numeric?(al_parts[1]) ? float(al_parts[1]) : -1.0
  if al_pct >= 0.0 && al_pct <= 100.0
    al_byte = int((al_pct * 255.0 / 100.0).round())
    al_dash = al_base.index_of("-")
    al_family = al_dash > 0 ? al_base.substring(0, al_dash) : al_base
    if prefix == "border" || prefix == "ring"
      return "border.subtle" if (al_base == "black" || tw_neutral?(al_family)) && al_pct <= 20.0
      return tw_status(al_family) + ".subtle" if tw_status(al_family) != "" && al_pct <= 30.0
    end
    if prefix == "bg"
      return "#000000" + tw_hex2(al_byte) if al_base == "black"
      return "#FFFFFF" + tw_hex2(al_byte) if al_base == "white"
      return "#6B7280" + tw_hex2(al_byte) if al_base == "gray-500"
    end
  end
  throw tw_no(whole, "roles are opaque; an opacity is taken only on a ring (ring-gray-900/5 is border.subtle) and on a scrim (bg-black/50, bg-gray-500/75)")
end

# A colour class's value, for `bg`, `text`, `border` and `ring`.
def tw_colour(prefix, value, whole)
  return "none" if value == "transparent"
  if value.starts_with?("[#") && value.ends_with?("]")
    co_hex = value.substring(2, value.length() - 1)
    return "#" + co_hex if tw_hex?(co_hex)

    throw tw_no(whole, "a literal colour is [#RRGGBB] or [#RRGGBBAA]")
  end
  return tw_alpha(prefix, value, whole) if value.index_of("/") > 0

  co_role = tw_role_alias(prefix, value)
  return co_role if co_role != ""

  if value == "white"
    return "accent.on" if prefix == "text"

    return "surface.raised"
  end
  return "text.default" if value == "black" && prefix == "text"

  co_dash = value.index_of("-")
  if co_dash <= 0
    throw tw_no(whole, "'" + value + "' is not a colour tw() knows; write a role (bg-accent, text-muted) or a palette shade") if value != "black"

    throw tw_no(whole, "black is a literal, and a surface is a role; write bg-gray-900, or bg-black/50 for a scrim")
  end
  co_family = value.substring(0, co_dash)
  co_shade = value.substring(co_dash + 1, value.length())
  co_found = ""
  if tw_neutral?(co_family)
    co_found = tw_gray(prefix, co_shade)
  elsif co_family == "indigo"
    co_found = "accent.base" if co_shade == "600"
    co_found = "accent.hover" if co_shade == "500"
    co_found = "accent.active" if co_shade == "700"
    if co_found == ""
      throw tw_no(whole, "the accent has base, hover and active (indigo-600, -500, -700) and no tint; bg-info-subtle is the nearest wash")
    end
  elsif tw_status(co_family) != ""
    co_status = tw_status(co_family)
    co_found = co_status + ".subtle" if co_shade == "50" || co_shade == "100"
    co_found = co_status + ".base" if ["500", "600", "700", "800"].includes?(co_shade)
    if co_found == ""
      throw tw_no(whole, co_family + " is the " + co_status + " role, which has a subtle tint (-50, -100) and a base (-500 to -800)")
    end
  else
    throw tw_no(whole, "colours are theme roles and " + co_family + " is not one; nearest is " + tw_nearest(co_family) + ", or [#RRGGBB] for a literal")
  end
  if co_found == ""
    throw tw_no(whole, "the grays are roles: bg white/50/100 are the surfaces, 200/300/400 the borders; text 900-700, 600-500 and 400 are default, muted and disabled")
  end
  co_found
end

# A gray shade, which is a different role depending on what it paints.
def tw_gray(prefix, shade)
  if prefix == "bg"
    return "surface.base" if shade == "50"
    return "surface.sunken" if shade == "100"
    return "border.subtle" if shade == "200"
    return "border.default" if shade == "300"
    return "border.strong" if shade == "400"
    return "text.default" if ["800", "900", "950"].includes?(shade)
  elsif prefix == "text"
    return "text.default" if ["700", "800", "900", "950"].includes?(shade)
    return "text.muted" if shade == "500" || shade == "600"
    return "text.disabled" if shade == "300" || shade == "400"
  else
    return "border.subtle" if shade == "100" || shade == "200"
    return "border.default" if shade == "300"
    return "border.strong" if shade == "400" || shade == "500"
  end
  ""
end

# ---- Classes -----------------------------------------------------------------

# The classes that are one fixed patch, looked up by name.
def tw_exact()
  {
    "flex": {"display": "row"},
    "inline-flex": {"display": "row"},
    "flex-row": {"display": "row"},
    "flex-col": {"display": "column"},
    "grid": {"display": "grid"},
    "hidden": {"display": "none"},
    "flex-wrap": {"wrap": "wrap"},
    "flex-nowrap": {"wrap": "nowrap"},
    "flex-wrap-reverse": {"wrap": "wrap_reverse"},
    "items-start": {"align": "start"},
    "items-center": {"align": "center"},
    "items-end": {"align": "end"},
    "items-stretch": {"align": "stretch"},
    "items-baseline": {"align": "baseline"},
    "justify-start": {"justify": "start"},
    "justify-center": {"justify": "center"},
    "justify-end": {"justify": "end"},
    "justify-between": {"justify": "between"},
    "justify-around": {"justify": "around"},
    "justify-evenly": {"justify": "evenly"},
    "self-auto": {"self": "auto"},
    "self-start": {"self": "start"},
    "self-center": {"self": "center"},
    "self-end": {"self": "end"},
    "self-stretch": {"self": "stretch"},
    "self-baseline": {"self": "baseline"},
    "grow": {"grow": 1},
    "grow-0": {"grow": 0},
    "shrink": {"shrink": 1},
    "shrink-0": {"shrink": 0},
    "flex-1": {"grow": 1, "shrink": 1, "basis": 0},
    "flex-auto": {"grow": 1, "shrink": 1, "basis": "auto"},
    "flex-initial": {"grow": 0, "shrink": 1},
    "flex-none": {"grow": 0, "shrink": 0},
    "text-xs": {"size": 0},
    "text-sm": {"size": 1},
    "text-base": {"size": 2},
    "text-lg": {"size": 3},
    "text-xl": {"size": 4},
    "text-2xl": {"size": 5},
    "text-3xl": {"size": 6},
    "text-4xl": {"size": 7},
    "text-left": {"text_align": "start"},
    "text-start": {"text_align": "start"},
    "text-center": {"text_align": "center"},
    "text-right": {"text_align": "end"},
    "text-end": {"text_align": "end"},
    "text-justify": {"text_align": "justify"},
    "font-normal": {"weight": "regular"},
    "font-medium": {"weight": "medium"},
    "font-semibold": {"weight": "semibold"},
    "font-bold": {"weight": "bold"},
    "font-sans": {"font": "sans"},
    "font-mono": {"font": "mono"},
    "underline": {"underline": true},
    "no-underline": {"underline": false},
    "line-through": {"strike": true},
    "truncate": {"clamp": 1},
    "line-clamp-none": {"clamp": 0},
    "rounded-none": {"radius": 0},
    "rounded-sm": {"radius": 1},
    "rounded": {"radius": 2},
    "rounded-md": {"radius": 2},
    "rounded-lg": {"radius": 2},
    "rounded-xl": {"radius": 3},
    "rounded-2xl": {"radius": 3},
    "rounded-3xl": {"radius": 3},
    "rounded-full": {"radius": 4},
    "shadow-none": {"shadow": 0},
    "shadow-sm": {"shadow": 1},
    "shadow": {"shadow": 2},
    "shadow-md": {"shadow": 2},
    "shadow-lg": {"shadow": 3},
    "shadow-xl": {"shadow": 3},
    "shadow-2xl": {"shadow": 3},
    "cursor-auto": {"cursor": "default"},
    "cursor-default": {"cursor": "default"},
    "cursor-pointer": {"cursor": "pointer"},
    "cursor-text": {"cursor": "text"},
    "cursor-wait": {"cursor": "wait"},
    "cursor-not-allowed": {"cursor": "not_allowed"},
    "cursor-grab": {"cursor": "grab"},
    "cursor-grabbing": {"cursor": "grabbing"},
    "cursor-col-resize": {"cursor": "resize_h"},
    "cursor-ew-resize": {"cursor": "resize_h"},
    "cursor-row-resize": {"cursor": "resize_v"},
    "cursor-ns-resize": {"cursor": "resize_v"},
    "overflow-hidden": {"overflow": "clip"},
    "overflow-clip": {"overflow": "clip"},
    "overflow-visible": {"overflow": "visible"},
    "transition": {"transition": "fast"},
    "transition-colors": {"transition": "fast"},
    "transition-all": {"transition": "fast"},
    "transition-opacity": {"transition": "fast"},
    "transition-shadow": {"transition": "fast"},
    "transition-none": {"transition": "none"},
    "absolute": {"position": "absolute"},
    "animate-spin": {"animation": "spin"},
    "animate-none": {"animation": "none"},
    "backdrop-blur-none": {"blur": 0},
    "backdrop-blur-sm": {"blur": 4},
    "backdrop-blur": {"blur": 8},
    "backdrop-blur-md": {"blur": 12},
    "backdrop-blur-lg": {"blur": 16},
    "backdrop-blur-xl": {"blur": 24},
    "backdrop-blur-2xl": {"blur": 40},
    "backdrop-blur-3xl": {"blur": 64},
    # A ring is drawn inside the box, and so is an EUI border; see `ring-1`.
    "ring-inset": {},
    # A block's children stack down it at its full width, which is a column
    # stretching them; its margins never collapse, as no EUI margin does.
    "block": {"display": "column"},
    # In flow, placed by its parent. What `relative` adds in a browser -- a
    # box an absolute child is placed against -- is what a `stack` is (04 §5).
    "relative": {"position": "flow"},
    "static": {"position": "flow"},
    # Every box already paints its children above itself and z orders only
    # siblings (04: no z-index across containers), so there is no stacking
    # context to isolate.
    "isolate": {},
    # Only editable nodes select text (06 §3).
    "select-none": {},
    # An auto margin on the cross axis centres the node on it: mx-auto in a
    # column, which is where Tailwind writes it (a centred container in block
    # flow), and my-auto in a row. On the main axis it is a spacer instead.
    "mx-auto": {"self": "center"},
    "my-auto": {"self": "center"},
    # Text transforms, which text() applies to its string; see tw_text.
    "uppercase": {"tw_case": "upper"},
    "lowercase": {"tw_case": "lower"},
    "capitalize": {"tw_case": "capital"},
    "normal-case": {"tw_case": "none"}
  }
end

# Which sides a suffix names: "" all four, x, y, t, r, b, l.
def tw_sides(which)
  return [true, false, false, false] if which == "t"
  return [false, true, false, false] if which == "r"
  return [false, false, true, false] if which == "b"
  return [false, false, false, true] if which == "l"
  return [false, true, false, true] if which == "x"
  return [true, false, true, false] if which == "y"

  [true, true, true, true]
end

def tw_edge(key, which, v)
  {"edge": key, "sides": tw_sides(which), "v": v}
end

# Four sides, from whatever a style already holds: nothing, one index, a
# pair, or four.
def tw_edges(start, sides, v)
  ed_now = [0, 0, 0, 0]
  if start.nil?
    ed_now = [0, 0, 0, 0]
  elsif start.class == "array" && start.length() == 4
    ed_now = [start[0], start[1], start[2], start[3]]
  elsif start.class == "array" && start.length() == 2
    ed_now = [start[0], start[1], start[0], start[1]]
  else
    ed_now = [start, start, start, start]
  end
  ed_out = range(0, 4).map(fn(i) { sides[i] == true ? v : ed_now[i] })
  return ed_out[0] if ed_out[0] == ed_out[1] && ed_out[1] == ed_out[2] && ed_out[2] == ed_out[3]

  ed_out
end

def tw_border_width(value, whole)
  return 1 if value == ""
  return int(value) if ["0", "1", "2", "4", "8"].includes?(value)

  throw tw_no(whole, "a border is border, border-0, -2, -4 or -8")
end

# One class, without its variant, as a patch: `{"set": style}`, or
# `{"edge": key, "sides": [...], "v": n}` for a class that writes some sides of
# `pad`, `margin` or `border`, or `{"props": {...}}`.
def tw_class(name, whole)
  if name.index_of(":") >= 0
    throw tw_no(whole, "one variant per class; hover:focus: is two states at once")
  end
  if name.starts_with?("-")
    throw tw_no(whole, "margins are unsigned, a byte per side; there is no negative margin")
  end
  cl_fixed = tw_exact()[name]
  return {"set": cl_fixed} unless cl_fixed.nil?

  cl_exact = tw_refused_exact(name)
  throw tw_no(whole, cl_exact) if cl_exact != ""

  return tw_edge("border", "", 1) if name == "border"

  cl_family = tw_family(name, whole)
  return cl_family unless cl_family.nil?

  cl_why = tw_refused(name)
  throw tw_no(whole, cl_why) if cl_why != ""

  throw tw_unknown(whole)
end

# The classes that carry a value: spacing, lengths, colours, borders, and the
# numeric ones. `nil` when the name is none of them.
def tw_family(name, whole)
  fa_dash = name.index_of("-")
  return nil if fa_dash <= 0

  fa_head = name.substring(0, fa_dash)
  fa_rest = name.substring(fa_dash + 1, name.length())

  # p-4, px-2, mt-1, gap-3
  if ["p", "px", "py", "pt", "pr", "pb", "pl"].includes?(fa_head)
    return tw_edge("pad", fa_head.substring(1, fa_head.length()), tw_space(fa_rest, whole))
  end
  if ["m", "mx", "my", "mt", "mr", "mb", "ml"].includes?(fa_head)
    return tw_edge("margin", fa_head.substring(1, fa_head.length()), tw_space(fa_rest, whole))
  end
  return {"set": {"gap": tw_space(fa_rest, whole)}} if fa_head == "gap" && fa_rest.index_of("-") < 0

  # gap-x-4, space-y-2, divide-y, divide-gray-200: what a box says about the
  # space and the rules between its children. Settled by tw_gap_settle and
  # laid on by tw_divide, once the box's direction and children are known.
  if (fa_head == "gap" || fa_head == "space") && (fa_rest.starts_with?("x-") || fa_rest.starts_with?("y-"))
    fa_axis = fa_rest.substring(0, 1)
    fa_step = fa_rest.substring(2, fa_rest.length())
    throw tw_no(whole, "children are drawn in the order they are given; reverse the list") if fa_step == "reverse"

    fa_kind = fa_head == "gap" ? fa_axis : "space_" + fa_axis
    return {"set": {}, "gap": {"k": fa_kind, "v": tw_space(fa_step, whole)}}
  end
  return tw_divider(fa_rest, whole) if fa_head == "divide"

  # w-64, h-10, size-8, basis-1/2, min-w-0, max-w-md
  return {"set": {"width": tw_length(fa_rest, whole)}} if fa_head == "w"
  return {"set": {"height": tw_length(fa_rest, whole)}} if fa_head == "h"
  if fa_head == "size"
    fa_side = tw_length(fa_rest, whole)
    return {"set": {"width": fa_side, "height": fa_side}}
  end
  return {"set": {"basis": tw_length(fa_rest, whole)}} if fa_head == "basis"
  if fa_head == "min" || fa_head == "max"
    fa_axis = fa_rest.substring(0, 2)
    fa_value = fa_rest.substring(2, fa_rest.length())
    if fa_axis == "w-" || fa_axis == "h-"
      fa_key = fa_head + (fa_axis == "w-" ? "_width" : "_height")
      fa_named = tw_max_widths()[fa_value]
      fa_set = {}
      if fa_key == "max_width" && !fa_named.nil?
        fa_set[fa_key] = fa_named
      else
        fa_set[fa_key] = tw_length(fa_value, whole)
      end
      return {"set": fa_set}
    end
    return nil
  end

  # bg-*, text-* (sizes and aligns are exact), border-*, ring-*
  return {"set": {"bg": tw_colour("bg", fa_rest, whole)}} if fa_head == "bg" && !fa_rest.starts_with?("gradient")
  if fa_head == "text"
    if tw_text_sizes()[fa_rest].nil? && ["5xl", "6xl", "7xl", "8xl", "9xl"].includes?(fa_rest)
      throw tw_no(whole, "the text scale stops at text-4xl, index 7")
    end
    if fa_rest.index_of("/") > 0 && !tw_text_sizes()[fa_rest.split("/")[0]].nil?
      throw tw_no(whole, "line height comes with the text size; write " + name.split("/")[0])
    end
    return {"set": {"fg": tw_colour("text", fa_rest, whole)}}
  end
  if fa_head == "border"
    return tw_edge("border", "", tw_border_width(fa_rest, whole)) if tw_numeric?(fa_rest)
    if ["x", "y", "t", "r", "b", "l"].includes?(fa_rest.substring(0, 1)) && (fa_rest.length() == 1 || fa_rest.substring(1, 2) == "-")
      fa_width = fa_rest.length() == 1 ? "" : fa_rest.substring(2, fa_rest.length())
      return tw_edge("border", fa_rest.substring(0, 1), tw_border_width(fa_width, whole))
    end
    return {"set": {"border_color": tw_colour("border", fa_rest, whole)}}
  end
  if fa_head == "ring"
    return tw_edge("border", "", int(fa_rest)) if fa_rest == "1" || fa_rest == "2"
    return nil if fa_rest.starts_with?("offset") || fa_rest.starts_with?("inset")
    if tw_numeric?(fa_rest)
      throw tw_no(whole, "a ring is written as a border, and ring-1 and ring-2 are the widths that read as one")
    end
    return {"set": {"border_color": tw_colour("ring", fa_rest, whole)}}
  end

  # opacity-50, z-10, line-clamp-2, duration-150, grid-cols-3
  if fa_head == "opacity"
    if tw_numeric?(fa_rest) && float(fa_rest) <= 100.0
      return {"set": {"opacity": int((float(fa_rest) * 255.0 / 100.0).round())}}
    end
    throw tw_no(whole, "opacity is 0 to 100")
  end
  if fa_head == "z"
    return {"set": {"z": int(fa_rest)}} if tw_numeric?(fa_rest) && fa_rest.index_of(".") < 0 && int(fa_rest) <= 255

    throw tw_no(whole, "z is 0 to 255")
  end
  if name.starts_with?("line-clamp-")
    fa_lines = name.substring(11, name.length())
    return {"set": {"clamp": int(fa_lines)}} if tw_numeric?(fa_lines) && fa_lines.index_of(".") < 0 && int(fa_lines) <= 255

    throw tw_no(whole, "line-clamp takes a count of lines")
  end
  if fa_head == "duration"
    if tw_numeric?(fa_rest)
      fa_ms = float(fa_rest)
      return {"set": {"transition": "fast"}} if fa_ms <= 100.0
      return {"set": {"transition": "base"}} if fa_ms <= 200.0
      return {"set": {"transition": "slow"}} if fa_ms <= 350.0
      return {"set": {"transition": "slower"}} if fa_ms <= 700.0

      return {"set": {"transition": "slowest"}}
    end
    throw tw_no(whole, "a duration is a number of milliseconds")
  end
  if name.starts_with?("grid-cols-")
    fa_cols = name.substring(10, name.length())
    return {"set": {}, "props": {"columns": int(fa_cols)}} if tw_numeric?(fa_cols) && fa_cols.index_of(".") < 0 && int(fa_cols) > 0

    throw tw_no(whole, "a grid's columns are equal shares; grid-cols-N takes a count")
  end
  nil
end

# One class into the result, in the variant it names.
def tw_take(out, variant, name, whole)
  tk_patch = tw_class(name, whole)
  tk_style = out[variant]
  if tk_patch["edge"].nil?
    tk_style = tk_style.merge(tk_patch["set"] ?? {})
  else
    tk_key = tk_patch["edge"]
    tk_from = tk_style[tk_key]
    tk_from = out["s"][tk_key] if tk_from.nil? && variant != "s"
    tk_style = tk_style.merge({})
    tk_style[tk_key] = tw_edges(tk_from, tk_patch["sides"], tk_patch["v"])
  end
  tk_gap = tk_patch["gap"]
  tk_rule = tk_patch["divide"]
  if !tk_gap.nil? || !tk_rule.nil?
    throw tw_no(whole, "the space and the rules between children are laid out once and have no states; write it without the state") if variant != "s"

    out["gaps"][tk_gap["k"]] = {"v": tk_gap["v"], "c": whole} unless tk_gap.nil?
    out["divide"] = out["divide"].merge(tk_rule).merge({"c": whole}) unless tk_rule.nil?
  end
  if variant != "s" && !(tk_patch["set"] ?? {})["tw_case"].nil?
    throw tw_no(whole, "a text transform changes the string, which a state cannot; write it without the state")
  end
  tk_props = tk_patch["props"] ?? {}
  if tk_props.keys().length() > 0
    throw tw_no(whole, "a prop has no states; write grid-cols-N without a variant") if variant != "s"

    out["props"] = out["props"].merge(tk_props)
  end
  out[variant] = tk_style
  out
end

# ---- Between children ----------------------------------------------------------

# `divide-y`, `divide-x-2`, `divide-gray-200`, as `{"divide": {...}}`.
def tw_divider(rest, whole)
  if rest == "x" || rest == "y"
    dr_one = {}
    dr_one[rest] = 1
    return {"set": {}, "divide": dr_one}
  end
  if rest.starts_with?("x-") || rest.starts_with?("y-")
    dr_width = rest.substring(2, rest.length())
    throw tw_no(whole, "children are drawn in the order they are given; reverse the list") if dr_width == "reverse"

    dr_rule = {}
    dr_rule[rest.substring(0, 1)] = tw_border_width(dr_width, whole)
    return {"set": {}, "divide": dr_rule}
  end
  if ["solid", "dashed", "dotted", "double", "none"].includes?(rest)
    throw tw_no(whole, "a border is always solid; divide-y-0 removes the rules")
  end
  {"set": {}, "divide": {"color": tw_colour("border", rest, whole)}}
end

# `gap-x-*`, `gap-y-*`, `space-x-*` and `space-y-*` as the one `gap` a box
# has, against the direction it finally runs in -- or a refusal saying why
# the two are not the same. `gap-x` is the gap between columns and `gap-y`
# between rows, overriding `gap-N` on its axis, as in the stylesheet:
#
# - a row that does not wrap spaces its children with its gap-x, and a column
#   with its gap-y. The other one spaces nothing, and is taken only when it is
#   0 or the same, since writing it says the author expected it to show;
# - a wrapping row or column, and a grid, space both ways, so the two must be
#   equal;
# - `space-x-N` is a left margin on every child but the first, which is the
#   gap exactly on a row that does not wrap and has no gap of its own -- and
#   nothing like it across the line, on a wrapped one (whose next lines start
#   indented), or added to a gap, where the browser draws the sum. `-0` is
#   nothing anywhere.
def tw_gap_settle(style, gaps, display)
  return style if gaps.keys().length() == 0 || display == "none"

  gs_out = style.merge({})
  gs_wraps = style["wrap"] == "wrap" || style["wrap"] == "wrap_reverse"
  gs_flow = display == "row" || display == "column"
  gs_main = display == "column" ? "y" : "x"
  gs_cross = display == "column" ? "x" : "y"
  gs_gx = gaps["x"]
  gs_gy = gaps["y"]
  if !gs_gx.nil? || !gs_gy.nil?
    gs_some = (gs_gx ?? gs_gy)["c"]
    throw tw_no(gs_some, "a stack lays its children over each other and has no gap") if display == "stack"

    gs_col = gs_gx.nil? ? (style["gap"] ?? 0) : gs_gx["v"]
    gs_row = gs_gy.nil? ? (style["gap"] ?? 0) : gs_gy["v"]
    if gs_flow && !gs_wraps
      gs_along = gs_main == "x" ? gs_col : gs_row
      gs_other = gaps[gs_cross]
      if !gs_other.nil? && gs_other["v"] != 0 && gs_other["v"] != gs_along
        throw tw_no(gs_other["c"], "EUI has one gap, and this " + display + " runs along " + gs_main + ": gap-" + gs_cross + " spaces nothing on it that does not wrap; write gap-" + gs_main + "-N or gap-N")
      end
      gs_out["gap"] = gs_along
    else
      if gs_col != gs_row
        gs_what = gs_flow ? "a wrapping " + display + " spaces its lines as well as its children" : "a grid spaces its rows as well as its columns"
        throw tw_no(gs_some, "EUI has one gap for both axes, and " + gs_what + "; write gap-N, or gap-x and gap-y the same")
      end
      gs_out["gap"] = gs_col
    end
  end
  for gs_axis in ["x", "y"]
    gs_space = gaps["space_" + gs_axis]
    if !gs_space.nil? && gs_space["v"] != 0
      gs_cls = gs_space["c"]
      throw tw_no(gs_cls, "space-" + gs_axis + " is a margin on every child but the first, and a " + display + " does not lay its children in a line; write gap-N") unless gs_flow
      if gs_axis != gs_main
        throw tw_no(gs_cls, "on a " + display + ", space-" + gs_axis + " puts a margin across the line, not between the children along it; write space-" + gs_main + "-N, or turn the box with " + (gs_main == "x" ? "flex-col" : "flex"))
      end
      throw tw_no(gs_cls, "on a wrapping " + display + ", space-" + gs_axis + " leaves the wrapped lines unspaced and their first child indented; write gap-N") if gs_wraps
      if (gs_out["gap"] ?? 0) != 0
        throw tw_no(gs_cls, "a gap and a space add up, and EUI has one gap; write one of them")
      end
      gs_out["gap"] = gs_space["v"]
    end
  end
  gs_out
end

# `divide-y` and its kin, laid onto the children: every child but the first
# takes the rule on its leading edge (top for y, left for x) and none on the
# trailing one, which is Tailwind's `> * ~ *` rule -- each axis on its own, so
# `divide-y md:divide-y-0 md:divide-x` is a left rule and no top one from md
# -- and `divide-<colour>` as
# its border colour. With no colour the child keeps its own, or has
# `border.subtle` -- the gray-200 Tailwind's preflight gives every border --
# since an EUI border with no colour is not drawn at all.
#
# A child is copied, never written: the same hash may be a child somewhere
# else. Its key and handlers go with it, and a local handler that restyles
# the node itself (`self.style = @hover`, what tw_wire and stateful write)
# has the rule laid onto each style it can switch to, or the rule would
# vanish on hover.
def tw_divide(kids, rule)
  dv_out = []
  dv_seen = false
  for dv_kid in kids
    if dv_kid.nil? || dv_seen == false
      dv_out = dv_out.concat([dv_kid])
      dv_seen = true unless dv_kid.nil?
    else
      dv_out = dv_out.concat([tw_divide_kid(dv_kid, rule)])
    end
  end
  dv_out
end

def tw_divide_kid(kid, rule)
  dk_new = kid.merge({})
  dk_new["s"] = tw_divide_style(kid["s"] ?? {}, rule)
  dk_on = kid["on"]
  return dk_new if dk_on.nil? || dk_on.class != "hash"

  dk_handlers = {}
  for dk_event in dk_on.keys()
    dk_handlers[dk_event] = tw_divide_handler(dk_on[dk_event], rule)
  end
  dk_new["on"] = dk_handlers
  dk_new
end

def tw_divide_handler(h, rule)
  return h unless h.class == "hash"
  return h unless h["local"].class == "string" && h["styles"].class == "hash"
  return h unless h["local"].starts_with?("self.style = @")

  dh_styles = {}
  for dh_name in h["styles"].keys()
    dh_styles[dh_name] = tw_divide_style(h["styles"][dh_name], rule)
  end
  h.merge({"styles": dh_styles})
end

def tw_divide_style(st, rule)
  ds_out = st.merge({})
  unless rule["y"].nil?
    ds_ruled = tw_edges(ds_out["border"], [true, false, false, false], rule["y"])
    ds_out["border"] = tw_edges(ds_ruled, [false, false, true, false], 0)
  end
  unless rule["x"].nil?
    ds_ruled = tw_edges(ds_out["border"], [false, false, false, true], rule["x"])
    ds_out["border"] = tw_edges(ds_ruled, [false, true, false, false], 0)
  end
  ds_out["border_color"] = rule["color"] ?? (st["border_color"] ?? "border.subtle")
  ds_out
end

# ---- Text transforms ---------------------------------------------------------

def tw_case_class(mode)
  return "uppercase" if mode == "upper"
  return "lowercase" if mode == "lower"
  return "capitalize" if mode == "capital"

  "normal-case"
end

# CSS's transforms, applied to the string on the server: capitalize raises
# the first letter after each space and leaves the rest as written.
def tw_case_apply(content, mode)
  return content.upcase() if mode == "upper"
  return content.downcase() if mode == "lower"
  return content unless mode == "capital"

  ca_out = ""
  ca_prev = " "
  for ca_ch in content.chars()
    ca_start = ca_prev == " " || ca_prev == "\n" || ca_prev == "\t"
    ca_out = ca_out + (ca_start ? ca_ch.upcase() : ca_ch)
    ca_prev = ca_ch
  end
  ca_out
end

# What `text(content, tw_style("uppercase ..."))` builds: the string
# transformed, and the style without the marker, which is no style key.
def tw_text(content, style)
  twt_mode = style["tw_case"]
  twt_style = {}
  for twt_key in style.keys()
    twt_style[twt_key] = style[twt_key] unless twt_key == "tw_case"
  end
  twt_said = content.class == "string" ? tw_case_apply(content, twt_mode) : content
  {
    "k": "text",
    "t": twt_said,
    "s": twt_style
  }
end

# Every class tw() accepts, one of each shape. `tests/tw_spec.sl` sends each
# through the encoder, and the gallery prints them.
def tw_examples()
  [
    "p-0", "p-0.5", "p-1", "p-2", "p-3", "p-4", "p-5", "p-6", "p-8", "p-10", "p-12", "p-16", "p-24",
    "px-4", "py-2", "pt-1", "pr-2", "pb-3", "pl-6", "m-2", "mx-4", "my-1", "mt-2", "mr-3", "mb-4", "ml-1", "gap-3",
    "w-64", "h-10", "w-1.5", "w-full", "w-auto", "w-px", "w-1/2", "w-2/3", "w-[320px]", "h-[50%]", "size-8",
    "basis-0", "basis-1/3", "min-w-0", "min-h-0", "min-w-full", "max-w-sm", "max-w-7xl", "max-w-full", "max-h-96",
    "flex", "inline-flex", "flex-row", "flex-col", "grid", "grid-cols-3", "hidden",
    "flex-wrap", "flex-nowrap", "flex-wrap-reverse",
    "items-start", "items-center", "items-end", "items-stretch", "items-baseline",
    "justify-start", "justify-center", "justify-end", "justify-between", "justify-around", "justify-evenly",
    "self-auto", "self-start", "self-center", "self-end", "self-stretch", "self-baseline",
    "grow", "grow-0", "shrink", "shrink-0", "flex-1", "flex-auto", "flex-initial", "flex-none",
    "text-xs", "text-sm", "text-base", "text-lg", "text-xl", "text-2xl", "text-3xl", "text-4xl",
    "text-left", "text-center", "text-right", "text-justify",
    "font-normal", "font-medium", "font-semibold", "font-bold", "font-sans", "font-mono",
    "underline", "no-underline", "line-through", "truncate", "line-clamp-2", "line-clamp-none",
    "bg-white", "bg-gray-50", "bg-gray-100", "bg-gray-200", "bg-gray-300", "bg-gray-400", "bg-gray-900", "bg-slate-50",
    "text-gray-900", "text-gray-700", "text-gray-600", "text-gray-500", "text-gray-400", "text-white",
    "border-gray-200", "border-gray-300", "border-gray-400", "ring-gray-300",
    "bg-indigo-600", "bg-indigo-500", "bg-indigo-700", "text-indigo-600",
    "bg-red-50", "text-red-600", "bg-green-50", "text-green-700", "bg-yellow-50", "text-yellow-800",
    "bg-blue-50", "text-blue-600", "ring-red-600/10", "ring-gray-900/5", "ring-black/5",
    "bg-accent", "bg-accent-hover", "text-muted", "text-default", "bg-danger-subtle", "bg-surface-raised",
    "bg-raised", "border-subtle", "border-strong", "text-series-2", "bg-focus-ring",
    "bg-transparent", "bg-[#1E293B]", "bg-[#00000080]", "bg-black/50", "bg-gray-500/75", "bg-white/80",
    "border", "border-0", "border-2", "border-4", "border-x", "border-y", "border-t", "border-b-2", "border-l-4",
    "ring-1", "ring-2", "ring-inset",
    "rounded-none", "rounded-sm", "rounded", "rounded-md", "rounded-lg", "rounded-xl", "rounded-2xl", "rounded-full",
    "shadow-none", "shadow-sm", "shadow", "shadow-md", "shadow-lg", "shadow-xl",
    "opacity-0", "opacity-50", "opacity-100",
    "cursor-pointer", "cursor-default", "cursor-text", "cursor-wait", "cursor-not-allowed", "cursor-grab",
    "cursor-grabbing", "cursor-col-resize", "cursor-row-resize",
    "overflow-hidden", "overflow-clip", "overflow-visible",
    "transition", "transition-colors", "transition-none", "duration-150", "duration-300", "duration-1000",
    "absolute", "z-10", "animate-spin", "backdrop-blur", "backdrop-blur-sm", "backdrop-blur-lg",
    "hover:bg-gray-50", "active:bg-gray-100", "focus:border-indigo-600", "disabled:opacity-50",
    "focus-visible:bg-gray-50", "focus-visible:outline-2", "focus-visible:ring-2", "focus:outline-none", "outline-none",
    "sm:flex", "md:flex-row", "lg:px-8", "xl:max-w-7xl", "2xl:text-lg", "md:hover:bg-gray-50",
    "flex space-x-4", "flex-col space-y-2", "block space-y-4", "flex gap-x-4", "flex-col gap-y-2", "grid gap-x-4 gap-y-4",
    "block", "relative", "static", "isolate", "select-none", "mx-auto", "my-auto",
    "uppercase", "lowercase", "capitalize", "normal-case"
  ]
end
