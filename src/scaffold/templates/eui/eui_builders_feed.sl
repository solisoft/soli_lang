# EUI view builders, part 4: tags, dragging, and the feed.
#
# Three things that share a habit: the model is plain data with no node in
# it, so a spec can pin the behaviour without standing a client up.
#
# Part of the reference catalogue — `eui_builders.sl` has the header that
# explains the whole of it, and the primitives everything here calls.

# ---- Tags ------------------------------------------------------------------
#
# A list of short things somebody typed. Not a `multi_select`: that one keeps a
# selection over a fixed set of options, and a selection cannot hold a word
# nobody has an id for. A tag list is ordered, its members are strings, and the
# set it draws from — if it draws from one at all — is only a suggestion.
#
# The four functions below are the whole of the model and none of them touches
# a node, which is what lets `tests/tag_spec.sl` pin them without a client.

# What a typed line becomes. Blank is nothing, the ends are trimmed, a repeat
# is not a second tag — matched without case, because "Lyon" and "lyon" are the
# same label to everyone but the machine — and `o["max"]` is a ceiling.
def tag_add(tags, text, o = {})
  said = (text ?? "").strip()
  return tags if said == ""

  for t in tags
    return tags if t.downcase() == said.downcase()
  end
  cap = o["max"] ?? 0
  return tags if cap > 0 && tags.length() >= cap

  # `concat` grows the array it is called on, so copy before growing: the
  # caller still holds `tags`, and the state hash it came out of holds it too.
  tags.slice(0, tags.length()).concat([said])
end

def tag_remove(tags, at)
  return tags if at < 0 || at >= tags.length()

  tags.slice(0, at).concat(tags.slice(at + 1, tags.length()))
end

# What to offer. Anything already taken is not a suggestion, and neither is
# anything that does not contain what has been typed so far — matched without
# case for the same reason `tag_add` dedupes without it. An empty draft offers
# everything left, because a panel that appears only once you have typed is a
# panel most people never learn is there.
def tag_suggest(all, tags, draft, limit)
  said = (draft ?? "").strip().downcase()
  out = []
  for one in all
    if out.length() < limit
      taken = false
      for t in tags
        taken = true if t.downcase() == one.downcase()
      end
      fits = said == "" || one.downcase().index_of(said) >= 0
      out = out.concat([one]) if !taken && fits
    end
  end
  out
end

# Where the highlight goes. It wraps, because a list you can walk off the end
# of is a list you have to look at to use; `-1` is "nothing highlighted", and
# stepping from there lands on an end rather than nowhere.
def tag_highlight(count, at, step)
  return -1 if count <= 0

  return step > 0 ? 0 : count - 1 if at < 0

  next_at = at + step
  return count - 1 if next_at < 0
  return 0 if next_at >= count

  next_at
end

# ---- The tag field ---------------------------------------------------------

# How many words the panel offers at once. It is capped in pixels too
# (`DROPDOWN_MAX_PX`), but a panel that scrolls is one nobody reads to the end
# of, and the arrow keys have to walk it.
TAG_SUGGEST_MAX = 6

# The keys the field hands back to the server instead of using itself.
#
# `Backspace` is the interesting one, and naming it does not make the field
# undeletable: 03 §3.1's third tier sends a key only when the press moved
# nothing in the text, which for `Backspace` means the caret was at the start
# with nothing left to delete — which is exactly when it should take the last
# chip instead. The other three the field has no use for at all.
#
# `Enter` is deliberately absent. It arrives as `submit`, and claiming it would
# withhold that submit (03 §3.1, tier 2) — which is the one event this field
# cannot do without.
TAG_KEYS = ["Backspace", "ArrowDown", "ArrowUp", "Escape"]

# The index a chip's × names. `chip_remove` keys itself from the `id` it is
# given, so the id carries the field's key as well as the position — two tag
# fields on one page would otherwise hand their first × the same key, which is
# the bug `erp_filter_chips` has today.
def tag_at(said)
  tga_text = said.to_s
  tga_cut = tga_text.index_of("/")
  return -1 if tga_cut < 0

  int(tga_text.substring(tga_cut + 1, tga_text.length()))
end

# The line you type into. Borderless and growing, because the well around it
# is the field; this is only the last cell of it.
def tag_entry(draft, bad, o)
  tgn_style = {
    "grow": 1,
    "min_width": 80,
    "border": 0,
    "bg": "none",
    "fg": "text.default",
    "pad": [0, 0, 0, 0]
  }
  # A no-op restyle on all four states, so `editable_states` leaves them alone.
  # It lights the node it is on, and here that node is a bare line of text
  # between the last chip and the right edge — lighting *it* draws a pale bar
  # inside the well rather than a field that has the keyboard. The well takes
  # the focused border from the server instead, which it must do anyway:
  # focus is what opens the panel, so the round trip is already happening.
  tgn_flat = {"local": "self.style = @base", "styles": {"base": tgn_style}}
  tgn_on = {"pointer_enter": tgn_flat, "pointer_leave": tgn_flat}
  tgn_on["submit"] = o["on_submit"] unless o["on_submit"].nil?
  tgn_on["key_down"] = o["on_key"] unless o["on_key"].nil?
  tgn_on["focus"] = tgn_flat
  tgn_on["blur"] = tgn_flat
  tgn_on["focus"] = tgn_flat.merge({"then": o["on_focus"]}) unless o["on_focus"].nil?
  tgn_on["blur"] = tgn_flat.merge({"then": o["on_blur"]}) unless o["on_blur"].nil?

  tgn_props = {
    "keys": TAG_KEYS,
    "role": "combo_box",
    "expanded": o["open"] == true,
    "label": o["name"] ?? (o["label"] ?? "Tags")
  }
  tgn_note = o["error"] ?? ""
  tgn_note = o["hint"] ?? "" if tgn_note == ""
  tgn_props["description"] = tgn_note if tgn_note != ""
  tgn_props["invalid"] = true if bad
  tgn_props["required"] = true if o["required"] == true
  # Clicking a suggestion blurs the field, so the server that took the word
  # asks for the caret back on the batch that answers the click — once, and not
  # on every render, or the field would steal focus from whatever else the page
  # has since been given.
  tgn_props["autofocus"] = true if o["take_focus"] == true
  # 03 §6.1 rule 4. The field keeps the keyboard while the arrows walk the
  # panel, so the option is named rather than focused -- which is the only way
  # to say "3 of 8, Consignment" with the option's own role and place in its
  # set rather than as prose a `live` node would have had to spell out.
  tgn_props["active_descendant"] = o["active"] unless o["active"].nil?

  input(draft, o["on_change"], {
    "key": o["key"].to_s + ":entry",
    "style": tgn_style,
    "props": tgn_props,
    "on": tgn_on
  })
end

# The well: the chips and the line, wrapping.
#
# `wrap` is what makes it a well rather than a row — chips are `shrink: 0`, so
# without it a ninth tag pushes the line you type into out of the box.
def tag_well(tags, draft, bad, o)
  tgw_lit = o["open"] == true
  tgw_edge = tgw_lit ? "accent.base" : "border.default"
  tgw_edge = "danger.base" if bad == true
  tgw_kids = range(0, tags.length()).map(fn(i) {
    chip(tags[i], o["on_remove"], {"id": o["key"].to_s + "/" + str(i)})
  })
  {
    "k": "box",
    "key": o["key"].to_s + ":well",
    "s": {
      "display": "row",
      "wrap": "wrap",
      "align": "center",
      "gap": 1,
      "pad": [1, 2, 1, 2],
      "width": o["width"] ?? "100%",
      "min_height": field_height(o),
      "border": 1,
      "border_color": tgw_edge,
      "radius": 2,
      "bg": "surface.raised",
      "shadow": 1,
      "cursor": "text",
      "transition": "fast"
    },
    "c": tgw_kids.concat([tag_entry(draft, bad, o)])
  }
end

# One word in the panel. `lit` is where the arrows have walked to, which is not
# a selection — nothing is chosen until Enter or a click — so it is `selected`
# for the sake of an assistive technology reading the panel and a left border
# for the sake of everyone else.
def tag_option(word, lit, pos, total, o)
  tgo_lit = lit == true
  control({
    "key": o["key"].to_s + ":opt:" + word,
    "size": "sm",
    "tone": "quiet",
    "shape": {
      "justify": "start",
      "align": "center",
      "width": "auto",
      "min_width": o["min_width"] ?? 200,
      "gap": 2,
      "radius": 1,
      "pad": [1, 2, 1, 2],
      "bg": tgo_lit ? "surface.sunken" : "none",
      "border": [0, 0, 0, 3],
      "border_color": tgo_lit ? "accent.base" : "none"
    },
    "on": {"click": o["on_pick"]},
    "props": {"id": word},
    "a11y": {
      "role": "option",
      "selected": tgo_lit,
      "label": word,
      "pos_in_set": pos,
      "set_size": total
    },
    "c": [text(word, {"weight": tgo_lit ? "semibold" : "regular"})]
  })
end

# A line of chips you type into, and a panel of what is still worth choosing.
#
# `combo_box` has been on the widget list since the first draft and this is it.
# What makes it one rather than a `multi_select` is that the words are not a
# fixed set: `tag_add` takes whatever was typed, so the panel narrows the
# familiar ones rather than enumerating the only ones.
#
# The caller owns everything. This is a view of four decisions it has already
# made — the tags, the draft, the suggestions, and where the arrows are — and
# the widget makes none of them, which is what lets `tests/tag_spec.sl` pin
# them without a client.
#
# `o`, beyond the usual field keys (`hint`, `error`, `required`, `width`,
# `density`, `name`):
#
#   "key"         (required) names every node in here
#   "on_change"   the draft moved — narrow the suggestions
#   "on_submit"   Enter — take the highlight, else commit the draft
#   "on_key"      one of `TAG_KEYS`; `params["payload"][0]` says which
#   "on_remove"   a chip's × — `tag_at(params["props"]["id"])` is the position
#   "on_pick"     a word in the panel — `params["props"]["id"]` is the word
#   "on_focus" / "on_blur"
#   "suggest"     the words to offer, already narrowed by `tag_suggest`
#   "at"          which of them the arrows are on, or -1
#   "open"        whether the panel is shown
#   "take_focus"  ask for the caret back on this batch and no other
def tag_field(label, tags, draft, o = {})
  throw "tag_field: o[\"key\"] names every node inside it" if o["key"].blank?

  tgf_error = o["error"] ?? ""
  tgf_bad = field_bad(tgf_error, o)
  tgf_words = o["suggest"] ?? []
  tgf_at = o["at"] ?? -1
  tgf_open = o["open"] == true && tgf_words.length() > 0
  tgf_o = o.merge({"label": o["label"] ?? label})
  # The key of the row the arrows are on, for the entry to point at. It has to
  # be worked out before the well is built and cannot be worked out inside it,
  # because the row it names is in the panel and the panel is the well's
  # sibling, not its child -- which is the whole reason the prop exists.
  tgf_o["active"] = o["key"].to_s + ":opt:" + tgf_words[tgf_at] if tgf_open && tgf_at >= 0 && tgf_at < tgf_words.length()
  tgf_well = tag_well(tags, draft, tgf_bad, tgf_o)
  return field_shell(label, tgf_well, o) unless tgf_open

  tgf_rows = range(0, tgf_words.length()).map(fn(i) {
    tag_option(tgf_words[i], i == tgf_at, i + 1, tgf_words.length(), tgf_o)
  })
  # The rows ask for their own width and the panel takes its size from them,
  # for the reason `multi_select` gives: `width: 100%` inside an absolutely
  # positioned overlay resolves against the window, not against the panel.
  tgf_panel = column({"gap": 0}, tgf_rows)
  tgf_panel["p"] = {"role": "list_box", "label": (o["label"] ?? label).to_s + " suggestions"}
  field_shell(label, dropdown(tgf_well, [tgf_panel], true, DROPDOWN_MAX_PX, o["on_blur"] ?? ""), o)
end

# ---- Picking things up -----------------------------------------------------
#
# Spec 06 §6. The client owns the whole of the hand — how a press becomes a
# grab, what is under it, which slot it is in, when a list should scroll
# because the hand is at its edge — and tells the server three things: one
# `drag_start`, one `drag_over` a boundary crossed, one `drop`.
#
# Two props do the declaring, and the split between them is the design:
# **the prop says what a node *is*; the handler says who *hears*.** A card is
# draggable and the column is what hears the drop, and those are two different
# nodes. There is no "reorder me" flag either — reordering is the case where
# the card's own column is the target, so one `accepts` gives both.
#
# A draggable node MUST carry a key. It is what the client holds it by: a move
# between columns is a removal and an insertion, so the node is rebuilt under
# the hand and its id changes, and only the key survives that.

# A thing that can be picked up. `group` is what a column has to accept for it
# to land there; the key is not optional.
def draggable(key, group, style, children, opts = {})
  {
    "k": "box",
    "key": key,
    "s": style,
    "p": {"drag": group}.merge(opts["p"] ?? {}),
    "on": opts["on"] ?? {},
    "c": children
  }
end

# The grip. A press here grabs at once — no slop to cross on a mouse and no
# half-second to wait out on a finger — which is what lets a row be dragged out
# of a list a finger can otherwise only scroll (06 §5 step 2).
def drag_grip(label)
  {
    "k": "box",
    "s": {
      "display": "row",
      "align": "center",
      "justify": "center",
      "width": 20,
      "height": 24,
      "radius": 2,
      "cursor": "grab",
      "fg": "text.muted",
      "shrink": 0,
      "transition": "fast"
    },
    "p": {"drag_handle": true, "role": "button", "label": label},
    "on": {
      "pointer_enter": {"local": "self.style = @lit", "styles": {"lit": drag_grip_style(true)}},
      "pointer_leave": {"local": "self.style = @rest", "styles": {"rest": drag_grip_style(false)}}
    },
    "c": [{"k": "icon", "s": {"width": 18, "height": 18}, "p": {"name": "grip"}}]
  }
end

# The grip lit and at rest. A grip is a small target and an easy one to miss,
# so it answers the pointer before the pointer commits to it.
def drag_grip_style(lit)
  {
    "display": "row",
    "align": "center",
    "justify": "center",
    "width": 20,
    "height": 24,
    "radius": 2,
    "cursor": "grab",
    "bg": lit ? "surface.sunken" : "none",
    "fg": lit ? "text.default" : "text.muted",
    "shrink": 0,
    "transition": "fast"
  }
end

# A thing that takes what others carry. The handlers are what make it a target
# at all: a container marked `accepts` with nothing listening is not one.
def drop_zone(group, style, on_over, on_drop, children, props = {})
  {
    "k": "box",
    "s": style,
    "p": {"accepts": group}.merge(props),
    "on": {"drag_over": on_over, "drop": on_drop},
    "c": children
  }
end

# The slot a `drag_over` or a `drop` carries: the third number, and `-1` when
# the gesture was cancelled rather than finished.
def drag_slot(params)
  (params["payload"] ?? [])[2] ?? -1
end

# Move `id` to `slot` of `col` in a board — a hash of column name to a list of
# ids — taking it out of wherever it was first. This is the whole of what a
# reorder is on the server: the view renders the lists, the cards are keyed,
# and the diff turns the permutation into `MoveChild` ops on its own.
def board_move(board, id, col, slot)
  out = {}
  for name in board.keys()
    kept = []
    for it in board[name]
      kept = kept.concat([it]) if it != id
    end
    out[name] = kept
  end
  at = slot < 0 ? 0 : slot
  at = out[col].length() if at > out[col].length()
  before = out[col].slice(0, at)
  after = out[col].slice(at, out[col].length())
  out[col] = before.concat([id]).concat(after)
  out
end

# Where `id` sits now, as `[column, slot]`, so a cancelled drag can put it
# back exactly where it was rather than at the top of where it came from.
def board_at(board, id)
  for name in board.keys()
    i = 0
    for it in board[name]
      return [name, i] if it == id
      i = i + 1
    end
  end
  [board.keys()[0], 0]
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

# ---------------------------------------------------------------- the meter
#
# A level meter, the way a cassette deck had one: a row of segments that
# light up to the reading, green until it gets loud, amber where it is
# getting close, red past the line. `level` arrives four times a second
# (03 §7) carrying the loudest the sound has been since the last one, so
# the meter is a peak-reading instrument and not a sampler -- a transient
# between two readings still lights the red.
#
# Boxes rather than a canvas, for the same reason the heatmap uses boxes:
# a segment can be hit-tested, the strip carries real `progress` semantics
# to a reader, and nothing here needs a path. `transition: "fast"` turns
# the four-a-second steps into a glide, so it reads as an instrument
# settling rather than a bar chart being redrawn.

# Which of the three the i-th of n segments belongs to. Not `series.*`:
# those are categorical and must not be read as a ramp. These three say
# what they mean in any theme, and mean it in dark mode too.
def vu_zone(i, n)
  at = n > 1 ? (i * 100) / (n - 1) : 0
  return "danger.base" if at >= 88
  return "warning.base" if at >= 70
  "success.base"
end

# One segment. An unlit segment is its own colour turned down, not a grey
# one: the dark red waiting at the top of the strip is half of why this
# reads as hardware.
def vu_segment(i, n, lit, axis)
  thin = axis == "v" ? 4 : 3
  long = axis == "v" ? 3 : 4
  {
    "k": "box",
    "s": {
      "width": axis == "v" ? 18 : long,
      "height": axis == "v" ? long : 10,
      "radius": 1,
      "bg": vu_zone(i, n),
      "opacity": lit == true ? 255 : 38,
      "transition": "fast",
      "shrink": 0,
      # Both axes now: a segment shares the length the strip was given, the
      # way the horizontal one always has. Vertically it used to be three
      # pixels and no growth, so the bar came out the height of its contents
      # — 58 px — while the legend beside it stretched to 96 and no mark
      # stood against the segment it names.
      "grow": 1,
      "min_width": axis == "v" ? 0 : thin,
      "min_height": axis == "v" ? long : 0
    }
  }
end

# One strip: the segments for a single channel.
#
# `peak` is the peak-hold marker -- the segment that stays lit above the
# bar while the bar falls away under it. The decay belongs to whoever owns
# the state, which is the server: it is a max and a countdown, not a thing
# the client should be told how to do.
def vu_strip(level, o = {})
  axis = o["axis"] ?? "h"
  n = o["segments"] ?? 12
  now = (level ?? 0).clamp(0, 100)
  hold = (o["peak"] ?? -1).clamp(-1, 100)
  lit_to = (now * n) / 100
  hold_at = hold >= 0 ? (hold * n) / 100 : -1
  cells = range(0, n).map(fn(i) {
    vu_segment(i, n, i < lit_to || i == hold_at, axis)
  })
  cells = cells.reverse() if axis == "v"
  # The vertical strip needs a length to share out, exactly as the
  # horizontal one takes the full width. `height` names it; the legend is
  # given the same one, which is the whole of making the two line up.
  tall = o["height"] ?? 96
  strip = axis == "v"
    ? column({"gap": 0, "align": "center", "shrink": 0, "height": tall}, cells)
    : row({"gap": 0, "align": "center", "width": "100%"}, cells)
  strip["s"]["gap"] = 1
  strip["p"] = {
    "role": "progress",
    "label": o["label"] ?? "Level",
    "value_now": now,
    "value_min": 0,
    "value_max": 100
  }
  strip
end

# The legend an old deck printed under the segments. dB, because that is
# what the numbers on the front of the machine said, and because 0 is the
# line you are not supposed to cross rather than the top of the scale --
# the red is past it, not at the end of it.
def vu_scale(o = {})
  axis = o["axis"] ?? "h"
  marks = (o["marks"] ?? ["-20", "-10", "-6", "-3", "0", "+3"]).map(fn(m) {
    text(m, {"size": 0, "fg": "text.muted", "font": "mono"})
  })
  # The mirror of the horizontal case, and it was not one. Laid out at its
  # natural height, six marks of text stand about twice as tall as twelve
  # three-pixel segments, so the legend ran past the strip and no mark stood
  # beside the segment it names. `between` is what the row already does
  # across its width.
  #
  # And no `height`: the parent row is `align: "stretch"`, so this column is
  # already the height of the strips beside it. Asking for `100%` on top of
  # that resolved against an ancestor instead and laid the meter out 6 232
  # pixels tall — measured, after writing it.
  if axis == "v"
    tall = o["height"] ?? 96
    return column({"gap": 0, "justify": "between", "align": "end", "shrink": 0, "height": tall}, marks.reverse())
  end


  row({"gap": 0, "justify": "between", "width": "100%"}, marks)
end

# The meter itself. `level` is one reading, or `[left, right]` for the two
# a `level` event carries -- which is what the front of a deck showed, one
# strip a channel, and what this draws when handed a pair.
def vu_meter(level, o = {})
  axis = o["axis"] ?? "h"
  pair = level.is_a?("array") == true ? level : [level]
  vals = pair.filter(fn(v) { v != null })
  # One strip per reading, whatever the readings are. Two is the pair a
  # `level` event carries and the shape a deck showed, so two is named left
  # and right; one is named nothing, because there is nothing to tell it
  # apart from. Anything else — bands of a spectrum, a channel per voice —
  # is the same drawing and only wants its own words, so `labels` supplies
  # them. A reader who cannot see the bars is who this is for: without a
  # name each strip announces itself as "Level" and the screen reader says
  # the same thing five times.
  sides = o["labels"].is_a?("array") == true
    ? o["labels"].map(fn(l) { " " + l.to_s })
    : (vals.length() == 2 ? [" left", " right"] : [""])
  # The hold marker has the same shape as the reading it follows: one
  # number for one strip, a pair for two. A single number shared by both
  # would put the louder channel's marker over the quieter one, which is
  # the one thing a peak-hold must not do.
  holds = (o["peak"] ?? -1).is_a?("array") == true ? o["peak"] : [o["peak"] ?? -1, o["peak"] ?? -1]
  strips = range(0, vals.length()).map(fn(i) {
    vu_strip(vals[i], o.merge({
      "peak": holds[i] ?? -1,
      "label": (o["label"] ?? "Level") + (sides[i] ?? "")
    }))
  })
  body = axis == "v"
    ? row({"gap": 1, "align": "end", "shrink": 0}, strips)
    : column({"gap": 1, "width": "100%"}, strips)
  return body if o["scale"] == false
  axis == "v"
    ? row({"gap": 2, "align": "stretch", "shrink": 0}, [body, vu_scale(o)])
    : column({"gap": 1, "width": "100%"}, [body, vu_scale(o)])
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

# What happened, in the order it happened.
#
# The rail down the left is what makes it a timeline rather than a list with
# dates in it: the line is continuous and the dots are on it, so the eye
# reads one thread instead of eight separate rows. The last entry draws a dot
# and no line -- a thread that continues past the last thing that happened is
# a thread that says more is coming, and nothing is.
#
# An entry is `{"at", "title", "body"?, "tone"?, "icon"?}`. `at` is whatever
# the application calls a time; this draws it and judges none of it.
def timeline(entries, o = {})
  tl_count = entries.length()
  tl_rows = range(0, tl_count).map(fn(i) {
    e = entries[i]
    last = i == tl_count - 1
    mark = (e["icon"] ?? "") == ""
      ? node("box", {"width": 9, "height": 9, "radius": 4, "shrink": 0, "bg": e["tone"] ?? "accent.base", "border": 2, "border_color": "surface.base"}, [])
      : icon(e["icon"], {"width": 14, "height": 14, "shrink": 0, "fg": e["tone"] ?? "accent.base"})
    rail = column({"gap": 0, "align": "center", "width": 16, "shrink": 0}, [
      node("box", {"height": 4}, []),
      mark,
      last ? spacer() : node("box", {"width": 1, "grow": 1, "bg": "border.default"}, [])
    ])
    said = column({"gap": 1, "grow": 1, "pad": [0, 0, last ? 0 : 4, 0]}, [
      row({"gap": 2, "align": "center", "width": "100%"}, [
        text(e["title"].to_s, {"weight": "semibold", "grow": 1, "clamp": 1}),
        muted(e["at"].to_s)
      ])
    ].concat((e["body"] ?? "") == "" ? [] : [muted(e["body"])]))
    keyed((o["key"] ?? "timeline") + ":" + i.to_s, row({"gap": 3, "align": "stretch", "width": "100%"}, [rail, said]))
  })
  tl_list = column({"gap": 0, "width": "100%"}, tl_rows)
  tl_list["p"] = {"role": "list", "label": o["label"] ?? "Activity"}
  tl_list
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
