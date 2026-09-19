# Markdown, both ways: a document as a node tree, and an editor that writes
# one.
#
# `doc/docs/eui/widgets.md` has listed `markdown` in the Tier-1 catalogue
# since the beginning without anything behind it; this is that entry, and
# `spec/03-widgets.md` §4 names the family. The fifth file of the reference
# catalogue, kept byte for byte with its copy in the language repository by
# `scripts/sync-catalogue.sh`.
#
# Soli's own `Markdown.to_html` is no use here — it returns HTML, and
# turning HTML back into nodes is a worse parser than reading the markdown
# directly. So this reads the source.
#
# The one structural constraint worth knowing: a `text` node carries a
# single font family, size and weight for its whole run (02 §3), so a
# sentence with a bold word in it cannot be one node. Inline runs therefore
# become several nodes in a wrapping row. The same sentence is why the
# editor below is what it is; its own header says the rest.

# ---------------------------------------------------------------- inline

# The next occurrence of `mark` in `s` at or after `at`, or -1.
def md_find(s, mark, at)
  size = s.length()
  msize = mark.length()
  i = at
  while i + msize <= size
    return i if s.substring(i, i + msize) == mark
    i = i + 1
  end
  -1
end

# Does `s` begin with `prefix`? Bounds-safe, which `substring` is not: a
# real document has lines shorter than the prefix being tested for — a bare
# "}" inside a fence is one character, and asking it for its first three
# is an index error, not a false.
def md_starts(s, prefix)
  return false if s.length() < prefix.length()

  s.substring(0, prefix.length()) == prefix
end

# And the other end, for a quoted title that has to give its closing quote
# back.
def md_ends(s, suffix)
  return false if s.length() < suffix.length()

  s.substring(s.length() - suffix.length(), s.length()) == suffix
end

# The leading digits of `said`, or 0. Written out rather than reached for:
# `to_i` is not on every type and a title field is whatever someone typed,
# so this reads what is there and stops at the first thing that is not a
# digit instead of raising on it.
def md_int(said)
  mn_at = 0
  mn_out = 0
  while mn_at < said.length() && md_find("0123456789", said.substring(mn_at, mn_at + 1), 0) >= 0
    mn_out = mn_out * 10 + md_find("0123456789", said.substring(mn_at, mn_at + 1), 0)
    mn_at = mn_at + 1
  end
  mn_out
end

# One inline run: the text, and what it is.
def md_run(t, kind)
  {"t": t, "k": kind}
end

# ---------------------------------------------------------------- pictures
#
# What a markdown `src` names, and the one thing this renderer cannot work
# out for itself.
#
# A node with a width and no height is laid out against a loosened
# constraint and stretches; an `image` draws its texture across whatever box
# it ends up with, so a photograph given the wrong box is not cropped, it is
# squashed. And this process cannot decode a JPEG to ask -- the bytes are in
# the asset store, addressed by their hash, and the picture is the client's
# to fetch (01 §5).
#
# So the size travels in markdown's own title field, which every other
# reader shows as a tooltip and this one reads as a measure:
#
#     ![The plan](eui-asset:<64 hex> "1600x1200")
#
# Whoever put the file there knew its size -- `Image.new(path).width()` --
# and writing it down costs nothing. A picture that arrives without one gets
# a modest box and sits in it, which is what a wrong shape deserves.

# An address as a node's `src`. An application that puts a file in the asset
# store writes `eui-asset:<hex>` and the node carries the asset itself;
# anything else is a path under `public/`, which the server hashes on its
# way out (01 §5).
#
# A file is not an asset (01 §6): what the person picked lives in the
# session spool and dies with the socket. Putting its bytes in the asset
# store is the application's decision and `eui_asset` is where it is made --
# never here, because a renderer that wrote to the store would do it again
# on every frame.
def md_src(src)
  return {"asset": src.substring(10, src.length())} if md_starts(src, "eui-asset:")

  src
end

# The inside of a markdown `(...)`: the address, and the title if there is
# one. `"640x480"` is read as a measure and anything else is kept as a note,
# which is what an attachment's card puts under its name.
def md_target(inside)
  mt_said = inside.strip()
  mt_note = ""
  mt_cut = md_find(mt_said, " \"", 0)
  if mt_cut >= 0
    mt_note = mt_said.substring(mt_cut + 2, mt_said.length())
    mt_note = mt_note.substring(0, mt_note.length() - 1) if md_ends(mt_note, "\"")
    mt_said = mt_said.substring(0, mt_cut)
  end
  mt_by = md_find(mt_note, "x", 0)
  mt_w = mt_by > 0 ? md_int(mt_note) : 0
  mt_h = mt_by > 0 ? md_int(mt_note.substring(mt_by + 1, mt_note.length())) : 0
  {"src": mt_said, "note": mt_note, "w": mt_w, "h": mt_h}
end

# A box of at most `width`, keeping the shape the picture actually has.
# Never taller than 520 either: a portrait photograph at the measure is a
# screenful of one picture, and a document is not a slideshow.
def md_fit(w, h, width)
  mf_w = w
  mf_h = h
  if mf_w < 1 || mf_h < 1
    mf_w = 320
    mf_h = 200
  end
  if mf_w > width
    mf_h = int(mf_h * width / mf_w)
    mf_w = width
  end
  if mf_h > 520
    mf_w = int(mf_w * 520 / mf_h)
    mf_h = 520
  end
  [mf_w < 1 ? 1 : mf_w, mf_h < 1 ? 1 : mf_h]
end

# The picture itself. `role` and `label` are what an assistive technology is
# handed, and the alt text is the only thing anyone wrote down about it.
def md_picture(run, width)
  mp_box = md_fit(run["w"] ?? 0, run["h"] ?? 0, width)
  mp_alt = (run["t"] ?? "").to_s
  {
    "k": "image",
    "s": {"width": mp_box[0], "height": mp_box[1], "radius": 2},
    "p": {
      "src": md_src((run["src"] ?? "").to_s),
      "role": "image",
      "label": mp_alt == "" ? "Image" : mp_alt
    }
  }
end

# A picture on a line of its own, with its alt text under it as a caption.
def md_figure(run, width)
  mg_alt = (run["t"] ?? "").to_s
  mg_kids = [md_picture(run, width)]
  mg_kids = mg_kids.concat([text(mg_alt, {"size": 1, "fg": "text.muted"})]) unless mg_alt == ""
  column({"gap": 1, "align": "start", "pad": [1, 0, 1, 0]}, mg_kids)
end

# `![alt](src)` alone on a line, or `{}`. The closing bracket has to be the
# last character: a line with prose after the picture is a paragraph, and
# its picture is an inline run.
def md_image_of(line)
  return {} unless md_starts(line, "![")

  mi_shut = md_find(line, "](", 2)
  return {} if mi_shut < 0

  mi_fin = md_find(line, ")", mi_shut + 2)
  return {} unless mi_fin == line.length() - 1

  {"t": line.substring(2, mi_shut), "k": "image"}.merge(md_target(line.substring(mi_shut + 2, mi_fin)))
end

# An address that names a file this application put somewhere, as against
# a place on the web: an asset, or a path with no scheme on it.
#
# The distinction has to be drawn somewhere, because in markdown an
# attachment and a link are the same three characters. A line that is only
# a link to `https://...` is a paragraph with a link in it, and turning
# that into a download card would surprise everyone; a line that is only a
# link to a file beside the application is what an attachment is.
def md_file_src?(src)
  return true if md_starts(src, "eui-asset:")
  return false if md_find(src, "://", 0) >= 0
  return false if md_starts(src, "mailto:")
  return false if md_starts(src, "#")

  src != ""
end

# `[name](<a file> "note")` alone on a line, or `{}` -- a file that is not
# a picture, which is a card and not a figure.
def md_file_of(line)
  return {} unless md_starts(line, "[")

  mo_shut = md_find(line, "](", 1)
  return {} if mo_shut < 0

  mo_fin = md_find(line, ")", mo_shut + 2)
  return {} unless mo_fin == line.length() - 1

  mo_aim = md_target(line.substring(mo_shut + 2, mo_fin))
  return {} unless md_file_src?((mo_aim["src"] ?? "").to_s)

  {"t": line.substring(1, mo_shut)}.merge(mo_aim)
end

# The extension, in small bold caps, is what goes in the square of a card
# that has no picture to put there.
def md_extension(name)
  me_at = name.length() - 1
  while me_at > 0
    return name.substring(me_at + 1, name.length()).upcase() if name.substring(me_at, me_at + 1) == "."

    me_at = me_at - 1
  end
  "FILE"
end

# A file as the catalogue's own card. The bytes are in the asset store and
# the card shows none of them: a PDF has no thumbnail a server can make
# without opening it, and opening it is not a renderer's business.
def md_attachment(link)
  ma_name = (link["t"] ?? "").to_s
  attachment_card(ma_name, (link["note"] ?? "").to_s, {
    "badge": text(md_extension(ma_name), {"size": 0, "weight": "bold", "fg": "text.muted"})
  })
end

# Is this line one of the two above? The paragraph walk asks, because a
# picture swallowed into a paragraph is a picture nobody sees.
def md_media?(line)
  return true unless md_image_of(line)["src"].nil?
  return true unless md_file_of(line)["src"].nil?

  false
end

# Cut a line into runs: code, strong, emphasis, pictures, links and the
# plain text between them. Unclosed markers are left as literal text rather than
# swallowing the rest of the line.
def md_runs(line)
  runs = []
  plain = ""
  i = 0
  size = line.length()
  while i < size
    c = line.substring(i, i + 1)
    two = i + 2 <= size ? line.substring(i, i + 2) : ""

    if two == "**"
      close = md_find(line, "**", i + 2)
      if close >= 0
        runs = runs.concat([md_run(plain, "plain")]) unless plain == ""
        plain = ""
        runs = runs.concat([md_run(line.substring(i + 2, close), "strong")])
        i = close + 2
        next
      end
    end

    if two == "~~"
      close = md_find(line, "~~", i + 2)
      if close >= 0
        runs = runs.concat([md_run(plain, "plain")]) unless plain == ""
        plain = ""
        runs = runs.concat([md_run(line.substring(i + 2, close), "strike")])
        i = close + 2
        next
      end
    end

    # `<u>` is not markdown: markdown has no underline at all, and `__x__` is
  # already bold. Nothing in this editor writes one. It is read because a
  # document arrives from somewhere else as often as it is written here,
  # and a tag the renderer drops is a word that quietly changes meaning.
  if c == "<" && md_starts(line.substring(i, line.length()), "<u>")
      close = md_find(line, "</u>", i + 3)
      if close >= 0
        runs = runs.concat([md_run(plain, "plain")]) unless plain == ""
        plain = ""
        runs = runs.concat([md_run(line.substring(i + 3, close), "under")])
        i = close + 4
        next
      end
    end

    if c == "`"
      close = md_find(line, "`", i + 1)
      if close >= 0
        runs = runs.concat([md_run(plain, "plain")]) unless plain == ""
        plain = ""
        runs = runs.concat([md_run(line.substring(i + 1, close), "code")])
        i = close + 1
        next
      end
    end

    if c == "*"
      close = md_find(line, "*", i + 1)
      if close >= 0
        runs = runs.concat([md_run(plain, "plain")]) unless plain == ""
        plain = ""
        runs = runs.concat([md_run(line.substring(i + 1, close), "em")])
        i = close + 1
        next
      end
    end

    if c == "!" && i + 2 <= size && line.substring(i + 1, i + 2) == "["
      shut = md_find(line, "](", i + 2)
      if shut >= 0
        fin = md_find(line, ")", shut + 2)
        if fin >= 0
          runs = runs.concat([md_run(plain, "plain")]) unless plain == ""
          plain = ""
          shown = {"t": line.substring(i + 2, shut), "k": "image"}
          runs = runs.concat([shown.merge(md_target(line.substring(shut + 2, fin)))])
          i = fin + 1
          next
        end
      end
    end

    if c == "["
      shut = md_find(line, "](", i + 1)
      if shut >= 0
        fin = md_find(line, ")", shut + 2)
        if fin >= 0
          runs = runs.concat([md_run(plain, "plain")]) unless plain == ""
          plain = ""
          runs = runs.concat([md_run(line.substring(i + 1, shut), "link")])
          i = fin + 1
          next
        end
      end
    end

    plain = plain + c
    i = i + 1
  end
  runs = runs.concat([md_run(plain, "plain")]) unless plain == ""
  runs
end

# A run as a node. `code` gets the mono face on a sunken ground; a link is
# accent-coloured and goes nowhere here. It could: a node carrying `open`
# hands its address to the person's browser when *they* click it (03 §3.5).
# But that wants the `net.open` capability, which this application does not
# ask for, and asking for one so that a paragraph of markdown can contain a
# link is the wrong way round. A builder does not decide that.
def md_run_node(run, size, em_font = "")
  kind = run["k"]
  # A picture in the middle of a sentence is a small one, and small is a
  # number this has to pick: a run carries no width to fit itself to. A
  # picture that wants the measure goes on a line of its own, where
  # `md_figure` has one.
  return md_picture(run, 240) if kind == "image"
  return text(run["t"], {"font": "mono", "size": size - 1, "bg": "surface.sunken", "fg": "accent.base", "pad": [0, 1, 0, 1], "radius": 1}) if kind == "code"
  return text(run["t"], {"size": size, "weight": "bold"}) if kind == "strong"
  # **There is no italic in EUI.** The 64-byte style record has a family, a
  # size and a weight, and no slant at all (02 §3) -- and its last reserved
  # byte went to `motion_kind`, so there will not be one. What a `*mark*`
  # can become is therefore a weight or a face, and nothing else.
  #
  # A weight by default, because the old answer was `text.muted` and grey
  # reads as *de*-emphasis: it said the opposite of what the mark meant.
  #
  # A face when the caller has one. `eui_font(name, paths)` ships an italic
  # with the application and its faces travel as content-addressed assets,
  # so an application that wants real italics has them for two lines of
  # `routes.sl` -- and one that does not gets a weight rather than a lie.
  return text(run["t"], {"size": size, "font": em_font, "weight": "medium"}) if kind == "em" && em_font != ""
  return text(run["t"], {"size": size, "weight": "medium"}) if kind == "em"
  return text(run["t"], {"size": size, "strike": true}) if kind == "strike"
  return text(run["t"], {"size": size, "underline": true}) if kind == "under"
  return text(run["t"], {"size": size, "fg": "accent.base"}) if kind == "link"

  text(run["t"], {"size": size})
end

# A line of inline markdown as a wrapping row of nodes.
#
# A plain run is split at spaces and each word given its own node. One node
# per run would be tidier, but a row wraps between its children, so a whole
# sentence would drop to the next line the moment it did not fit beside a
# bold word. Words wrap the way prose is supposed to.
def md_line(line, size, em_font = "")
  nodes = []
  for r in md_runs(line)
    if r["k"] == "plain"
      # A run following code or bold begins with the space that separated
      # them, and splitting on " " turns that into an empty first element.
      # Dropping it as empty is what glued "`code`" to the word after it.
      lead = md_starts(r["t"], " ") ? " " : ""
      for w in r["t"].split(" ")
        nodes = nodes.concat([text(lead + w + " ", {"size": size})]) unless w == ""
        lead = "" unless w == ""
      end
    else
      nodes = nodes.concat([md_run_node(r, size, em_font)])
    end
  end
  row({"wrap": "wrap", "align": "baseline", "gap": 0}, nodes)
end

# ---------------------------------------------------------------- blocks

def md_heading_level(line)
  return 0 unless md_starts(line, "#")

  n = 0
  while n < 6 && n < line.length() && line.substring(n, n + 1) == "#"
    n = n + 1
  end
  return 0 unless n < line.length() && line.substring(n, n + 1) == " "

  n
end

# A heading's size on the text scale, floored so `######` still outranks
# the prose around it.
def md_heading_size(level)
  size = 6 - level
  size < 2 ? 2 : size
end

# The cells of a table row, the outer pipes dropped. Counted in characters:
# `length()` counts bytes, and a row with an em dash in it kept its last
# pipe, split into one cell too many, and drew a third column.
def md_table_cells(line)
  trimmed = line.strip()
  cells_wide = trimmed.chars().length()
  if md_starts(trimmed, "|")
    trimmed = trimmed.substring(1, cells_wide)
    cells_wide = cells_wide - 1
  end
  trimmed = trimmed.substring(0, cells_wide - 1) if cells_wide > 0 && trimmed.substring(cells_wide - 1, cells_wide) == "|"
  trimmed.split("|").map(fn(c) { c.strip() })
end

def md_table(rows)
  return column({"gap": 0}, []) if rows.length() == 0

  head = md_table_cells(rows[0])
  # An explicit share per column. `grow` alone sizes each cell to its own
  # content plus a share of what is left, so a header and its body land on
  # different edges; `basis: 0` would equalise them but makes the measure
  # pass size text against zero width — the word "Kind" then reports three
  # lines and every row is 144 px tall. A percentage does neither.
  pct = str(int(100 / head.length())) + "%"
  body = rows.length() > 2 ? range(2, rows.length()).map(fn(i) { md_table_cells(rows[i]) }) : []
  # A grid: every cell but the first in a row carries a hairline on its
  # left, every row one underneath, and the body is striped — one row in
  # two on the raised surface — so a wide table can be read across.
  header = row(
    {"gap": 0, "pad": [0, 0, 0, 0], "bg": "surface.sunken", "border": [0, 0, 1, 0], "border_color": "border.default"},
    range(0, head.length()).map(fn(i) {
      text(head[i], {"weight": "semibold", "size": 1, "width": pct, "pad": [1, 2, 1, 2], "border": [0, 0, 0, i == 0 ? 0 : 1], "border_color": "border.subtle"})
    })
  )
  # A cell wraps: it has a width now, the percentage above, so its words
  # can flow into it the way a paragraph's do. What they must not do is
  # shrink — a flex child's default — because a row that does not fit then
  # squeezes every word below its own width and each one breaks letter by
  # letter, which is what a cell full of "Ho / w ma / ny ro / ws" was. A
  # run wider than the cell overflows and is clipped instead.
  lines = range(0, body.length()).map(fn(r) {
    cells = body[r]
    row(
      {
        "gap": 0,
        "pad": [0, 0, 0, 0],
        "bg": r % 2 == 1 ? "surface.raised" : "none",
        "border": [0, 0, 1, 0],
        "border_color": "border.subtle"
      },
      range(0, cells.length()).map(fn(i) {
        cell = md_line(cells[i], 1)
        cell["s"] = cell["s"].merge({
          "width": pct,
          "overflow": "clip",
          "pad": [1, 2, 1, 2],
          "border": [0, 0, 0, i == 0 ? 0 : 1],
          "border_color": "border.subtle"
        })
        cell["c"] = cell["c"].map(fn(run) {
          run["s"] = run["s"].merge({"shrink": 0})
          run
        })
        cell
      })
    )
  })
  column({"gap": 0, "border": 1, "border_color": "border.default", "radius": 2, "overflow": "clip"}, [header].concat(lines))
end

def md_quote(lines)
  {
    "k": "box",
    "s": {
      "display": "column",
      "gap": 1,
      "pad": [2, 3, 2, 3],
      "bg": "surface.sunken",
      "border": [0, 0, 0, 3],
      "border_color": "accent.base",
      "radius": 1
    },
    "c": lines.map(fn(l) { md_line(l, 2) })
  }
end

# A list, and a task list, which markdown makes the same thing: `- [ ] x`
# is a bullet whose first three characters are a box. So the marker is
# decided per item rather than per list, because nothing stops a document
# mixing them and every reader that matters draws them mixed.
#
# The box is `check_mark`, the same one `checkbox` is built from — drawn
# here and not pressable, because this is the document being *read*. The
# editor's own task blocks carry a real `checkbox`; a tick in a rendered
# page would be a control with nowhere to send itself.
def md_list(items, ordered)
  column({"gap": 1}, range(0, items.length()).map(fn(i) {
    row({"gap": 2, "align": "start"}, [
      md_list_marker(items[i], ordered, i),
      {"k": "box", "s": {"display": "column", "grow": 1}, "c": [md_line(md_task_bare(items[i]), 2)]}
    ])
  }))
end

def md_list_marker(item, ordered, at)
  return text(str(at + 1) + ".", {"fg": "text.muted", "size": 2, "width": 24}) if ordered
  # The box sits in a line-tall box so it lands on the words beside it
  # rather than at the top of a row that wrapped.
  unless md_task_of(item) == ""
    return {
      "k": "box",
      "s": {"display": "row", "align": "center", "height": 22, "width": 22, "shrink": 0},
      "c": [check_mark(md_task_of(item) == "x", false, false, "sm")]
    }
  end

  text("•", {"fg": "text.muted", "size": 2, "width": 14})
end

# `"x"` for a ticked box, `" "` for an empty one, and `""` for a list item
# that carries no box at all.
def md_task_of(item)
  return "x" if md_starts(item, "[x] ") || md_starts(item, "[X] ")
  return " " if md_starts(item, "[ ] ")

  ""
end

def md_task_bare(item)
  md_task_of(item) == "" ? item : item.substring(4, item.length())
end

# How many characters an ordered-list marker takes ("12. " -> 4), or 0.
def md_ordered_marker(line)
  i = 0
  size = line.length()
  while i < size && "0123456789".includes?(line.substring(i, i + 1))
    i = i + 1
  end
  return 0 if i == 0
  return 0 unless i + 2 <= line.length() && line.substring(i, i + 2) == ". "

  i + 2
end

# Does this line start a block of its own? Which is the same question as
# "does the paragraph above it end here", and the reason it is a function:
# a paragraph is whatever is left once every other block has said no, so the
# list has to be read from both ends and read the same way twice.
def md_breaks?(said)
  return true if said == ""
  return true if md_heading_level(said) > 0
  return true if md_starts(said, ">") || md_starts(said, "|")
  return true if md_starts(said, "```") || md_starts(said, "- ")

  md_media?(said)
end

# The block walk. Markdown is line-oriented at this level: a line either
# starts a block or continues the one before it.
#
# `width` is the measure a picture is fitted to, and the only thing here
# that needs one. It has a default because most callers render into a
# column whose width they never said out loud, and 640 is a readable one;
# a caller that knows better passes it.
def md_blocks(source, width = 640, em_font = "")
  out = []
  lines = source.split("\n")
  n = lines.length()
  i = 0
  while i < n
    line = lines[i]
    trimmed = line.strip()

    if trimmed == ""
      i = i + 1
      next
    end

    # A fence runs to its closing fence, or to the end of the document if
    # the author forgot one. It goes through the code viewer, which now
    # sizes itself to the lines it was given.
    if md_starts(trimmed, "```")
      lang = trimmed.substring(3, trimmed.length()).strip()
      body = []
      i = i + 1
      while i < n && !md_starts(lines[i].strip(), "```")
        body = body.concat([lines[i]])
        i = i + 1
      end
      i = i + 1
      code = body.join("\n")
      out = out.concat([code_viewer(code, {
        "line_numbers": false,
        "spans": code_spans(code, lang),
        "language": lang
      })])
      next
    end

    level = md_heading_level(line)
    if level > 0
      size = md_heading_size(level)
      # `piece`, not `node`: a bare assignment to a name that is also a
      # top-level def rebinds that def for the whole process, and `node` is
      # the builder every `row` and `column` calls.
      heads = md_runs(line.substring(level + 1, line.length())).map(fn(r) {
        piece = md_run_node(r, size, em_font)
        piece["s"] = piece["s"].merge({"weight": "bold"})
        piece
      })
      out = out.concat([row({"wrap": "wrap", "gap": 0, "pad": [level == 1 ? 2 : 1, 0, 0, 0]}, heads)])
      i = i + 1
      next
    end

    if trimmed == "---" || trimmed == "***" || trimmed == "___"
      out = out.concat([divider()])
      i = i + 1
      next
    end

    if md_starts(trimmed, "> ")
      body = []
      while i < n && md_starts(lines[i].strip(), ">")
        body = body.concat([lines[i].strip().substring(1, lines[i].strip().length()).strip()])
        i = i + 1
      end
      out = out.concat([md_quote(body)])
      next
    end

    if md_starts(trimmed, "|")
      rows = []
      while i < n && md_starts(lines[i].strip(), "|")
        rows = rows.concat([lines[i]])
        i = i + 1
      end
      out = out.concat([md_table(rows)])
      next
    end

    shown = md_image_of(trimmed)
    unless shown["src"].nil?
      out = out.concat([md_figure(shown, width)])
      i = i + 1
      next
    end

    kept = md_file_of(trimmed)
    unless kept["src"].nil?
      out = out.concat([md_attachment(kept)])
      i = i + 1
      next
    end

    if md_starts(trimmed, "- ") || md_starts(trimmed, "* ")
      items = []
      while i < n && (md_starts(lines[i].strip(), "- ") || md_starts(lines[i].strip(), "* "))
        items = items.concat([lines[i].strip().substring(2, lines[i].strip().length())])
        i = i + 1
      end
      out = out.concat([md_list(items, false)])
      next
    end

    if md_ordered_marker(trimmed) > 0
      items = []
      while i < n && md_ordered_marker(lines[i].strip()) > 0
        cut = md_ordered_marker(lines[i].strip())
        items = items.concat([lines[i].strip().substring(cut, lines[i].strip().length())])
        i = i + 1
      end
      out = out.concat([md_list(items, true)])
      next
    end

    # Anything else is a paragraph: consecutive non-blank lines that start
    # no other block, joined, because a hard wrap in the source is not a
    # line break in the prose.
    para = []
    while i < n && !md_breaks?(lines[i].strip())
      para = para.concat([lines[i].strip()])
      i = i + 1
    end
    out = out.concat([md_line(para.join(" "), 2, em_font)]) if para.length() > 0
    i = i + 1 if para.length() == 0
  end
  out
end

# A document read from disk, parsed once.
#
# The cache is keyed on the path and the measure, because between them they
# are the whole input: the same file at the same width gives the same nodes
# every time, and only a picture cares about the second. This is the
# memoisation `lazy` deliberately does not do — `lazy` cannot know what a
# subtree reads, but a document knows it reads one file.
#
# `File.read` is jailed to the application root, so the path is relative to
# it and must live inside the app: `../..` is refused, and so is a symlink.
# A `.md` under the app is carried into a packaged artifact verbatim
# (`lang/src/bundle.rs`), and the artifact serves from its extraction dir,
# so this same path works packaged and unpackaged.
MD_DOCS = {}

def markdown_file(path, opts)
  said = path + "@" + str((opts ?? {})["width"] ?? 640)
  cached = MD_DOCS[said]
  return cached unless cached.nil?

  built = markdown(File.read(path), opts)
  MD_DOCS[said] = built
  built
end

# A markdown document as rows for a windowed `list` (04 §7.1), so a long
# page costs the client one window of blocks rather than the page: parsed
# once per path and cached, with a height guessed for every block. The
# guess sets the scroll extent and the placeholders until a row arrives; it
# errs tall, because a row taller than its slot is clipped and a row shorter
# than it leaves air, and air is the lesser fault.
MD_DOC_ROWS = {}

def md_doc_rows(path, width)
  # Keyed by the width too: the guesses are for a width, and a window that
  # was resized asks for the same document at another one.
  doc_key = path + "@" + str(width)
  cached = MD_DOC_ROWS[doc_key]
  return cached unless cached.nil?

  blocks = md_blocks(File.read(path), width)
  built = {"rows": blocks, "heights": blocks.map(fn(b) { md_guess_height(b, width) })}
  MD_DOC_ROWS[doc_key] = built
  built
end

# How tall a block will probably be at `width`, from its nodes alone: text
# is 8 px a character and a line of its size tall, a wrapping row of words
# is the characters it holds divided into lines, a code viewer is 18 px a
# line, and a column is its children stacked with their gap. Not a layout —
# a guess a layout replaces — and one that errs tall on purpose: a row
# taller than its slot overlaps the next, a row shorter leaves air.
#
# Its locals are named for it alone: a bare assignment in a callee writes
# the caller's variable of that name, and this recurses.
MD_LINE_PX = [16, 18, 22, 24, 28, 32, 38, 46]
MD_SPACE_PX = [0, 2, 4, 8, 12, 16, 20, 24, 32, 40, 48, 64, 96]

# A `pad` is one space index for every side or a list of four; either way,
# the px of side `side` (0 top, 2 bottom).
def md_pad_px(pad, side)
  return 0 if pad.nil?
  return MD_SPACE_PX[pad] ?? 0 if pad.class() == "int"

  MD_SPACE_PX[pad[side] ?? 0] ?? 0
end

def md_line_px(style)
  MD_LINE_PX[(style ?? {})["size"] ?? 2] ?? 22
end

def md_guess_height(node, width)
  guess_kind = node["k"] ?? "box"
  guess_style = node["s"] ?? {}
  return md_guess_text(node["t"] ?? "", guess_style, width) if guess_kind == "text"
  return 1 if guess_kind == "divider"
  # A picture is the one node whose height is not a guess: `md_fit` already
  # settled it, and it is in the style.
  return md_guess_px(guess_style["height"], 200) if guess_kind == "image"

  guess_kids = node["c"] ?? []
  return 0 if guess_kids.length() == 0

  guess_pad = md_pad_px(guess_style["pad"], 0) + md_pad_px(guess_style["pad"], 2)
  if guess_style["display"] == "row" && guess_style["wrap"] == "wrap"
    guess_chars = 0
    guess_line = 22
    for kid in guess_kids
      guess_chars = guess_chars + (kid["t"] ?? "").length()
      guess_line = md_line_px(kid["s"]) if md_line_px(kid["s"]) > guess_line
    end
    return guess_pad + guess_line * (int(guess_chars * 8 / width) + 1)
  end
  # The children first, as a list, and only then a fold over it: this
  # recurses, and a recursive call writes the same names — an accumulator
  # kept across the calls would be reset by each of them (the table under
  # "Key in `event_data`" came out 14 px short that way).
  guess_each = guess_kids.map(fn(k) { md_guess_height(k, width) })
  if guess_style["display"] == "row"
    guess_tallest = 0
    for guess_h in guess_each
      guess_tallest = guess_h if guess_h > guess_tallest
    end
    return guess_pad + guess_tallest + 4
  end
  guess_gap = MD_SPACE_PX[guess_style["gap"] ?? 0] ?? 0
  guess_pad + int(guess_each.sum()) + guess_gap * (guess_kids.length() - 1) + 8
end

# A style dimension in pixels, or `fallback` when it is a percentage, a
# string or absent. `md_guess_height` only ever wants a number.
def md_guess_px(said, fallback)
  return fallback if said.nil?
  return said if said.class() == "int"

  fallback
end

def md_guess_text(content, style, width)
  guess_rows = content.split("\n")
  guess_line = (style ?? {})["font"] == "mono" ? 18 : md_line_px(style)
  guess_lines = 0
  for guess_row in guess_rows
    guess_lines = guess_lines + int(guess_row.length() * 8 / width) + 1
  end
  guess_line * guess_lines
end

# A markdown document as a node. `opts` may carry {"gap": n} and the
# {"width": px} a picture is fitted to.
def markdown(source, opts)
  opts = opts ?? {}
  column({"gap": opts["gap"] ?? 3}, md_blocks(source, opts["width"] ?? 640, (opts["em_font"] ?? "").to_s))
end

# ============================================================== the editor
#
# The catalogue could show a document; this is what writes one.
#
# **Why it is built from blocks, and not out of a rich-text field.** Two
# rules of the protocol decide the shape of this widget, and no amount of
# work gets around either:
#
#   1. A `text` node carries one family, one size and one weight for its
#      whole run (02 §3). A field cannot show a bold word inside a sentence,
#      because the field is one node and one node is one style.
#   2. The client owns the caret and the selection and reports neither
#      (03 §3, 08 §7.1). `change` carries the whole settled value and
#      `text_input` the committed text; nothing carries an offset. So no
#      button here can wrap "what is selected", because nothing here knows
#      what is selected.
#
# Between them those rule out a WYSIWYG of the kind a browser has. What is
# left is better than it sounds, because 03 §3.1 rule 3 says a key in a
# position where it does nothing — `Backspace` with nothing before the
# caret, an arrow at the end of the text — is never the client's, and is
# reported if `keys` asked for it. That is the whole mechanism of a block
# editor, written into the specification:
#
#   Enter            `change` then `submit` on an `input` (rule 2) — a block
#   Backspace at 0   nothing for the client to do — blocks merge
#   Up / Down        never a single-line field's — the caret changes block
#   focus_to         an explicit `Focus` op: the server puts the caret in
#                    the block it just made (01 §4, `diff.rs`)
#
# So a block is an `input`, which wraps and grows because an editable node
# is measured by the same shaper a `text` is. What is lost against one big
# `textarea` is that `Enter` cannot split a block at the caret — there is no
# caret to split at — so it adds an empty block after the one being typed
# in, which is where `Enter` is pressed nearly every time. A code block is a
# `textarea`, where a newline is the whole point.
#
# The marks stay visible while a block is being edited: `**bold**` reads as
# `**bold**`. It becomes bold in the preview and everywhere the document is
# read afterwards. That is the compromise, and it is structural rather than
# unfinished — so `B` and `I` here mark the **block**, not a selection, and
# unmark it when pressed again. A button that promised anything else would
# be lying about what the server can know.
#
# The widget holds no state, exactly as `file_drop` holds none. The document
# is the caller's; `markdown_editor_step` is what moves it.

# ---------------------------------------------------------------- the model

# The line a block begins with, by kind. An ordered item is numbered by the
# serialiser, which is the only thing that knows how far down the list it is.
def md_edit_mark(kind)
  return "# " if kind == "h1"
  return "## " if kind == "h2"
  return "### " if kind == "h3"
  return "> " if kind == "quote"
  return "- " if kind == "bullet"

  ""
end

# The same table read the other way. Longest first, or "## " is read as a
# "# " with a "#" after it.
def md_edit_kind_of(said)
  return "h3" if md_starts(said, "### ")
  return "h2" if md_starts(said, "## ")
  return "h1" if md_starts(said, "# ")
  # A quote line with nothing on it is `>` and not `> `, which is what a
  # quoted mail is mostly made of: every blank line of the letter being
  # answered comes back as one. Reading it as a paragraph puts a visible
  # ">" in the middle of the quote, once per blank line.
  return "quote" if md_starts(said, "> ") || said == ">"
  # Before the bullet, because a task *is* a bullet with a box on it and
  # reading it as one would leave the box in the text.
  return "task" if md_starts(said, "- [ ] ") || md_starts(said, "- [x] ") || md_starts(said, "- [X] ")
  return "bullet" if md_starts(said, "- ") || md_starts(said, "* ")
  return "number" if md_ordered_marker(said) > 0

  "p"
end

# A line with its marker taken off.
# Is a task line ticked? `[x]` either case; anything else is not.
def md_edit_done?(said)
  md_starts(said, "- [x] ") || md_starts(said, "- [X] ")
end

def md_edit_bare(said)
  mb_kind = md_edit_kind_of(said)
  return said.substring(6, said.length()) if mb_kind == "task"
  return "" if said == ">"
  return said.substring(md_ordered_marker(said), said.length()) if mb_kind == "number"
  return said.substring(md_edit_mark(mb_kind).length(), said.length()) unless mb_kind == "p"

  said
end

# Is this a kind someone types into? A picture, a file and a rule are not:
# they have no field, so no key of theirs is ever reported.
def md_edit_text?(kind)
  kind != "image" && kind != "file" && kind != "rule" && kind != "table"
end

# A table's cells are its text, and each one is its own field. Nothing that
# takes a whole block's value applies to it.
def md_edit_grid?(kind)
  kind == "table"
end

# A kind whose text is kept exactly as it arrives — no marker is read out of
# it, and a newline in it is a newline and not a second block.
def md_edit_verbatim?(kind)
  kind == "code" || kind == "image" || kind == "file"
end

# One block, and where the line after it starts: `[block, at]`. A fence runs
# to its closing fence -- or to the end of the document, if whoever wrote it
# forgot one -- and everything else is a line.
#
# Written as a reader that returns where it stopped rather than as a walk
# with a `next` in it, because this is one of the functions the specs copy
# and `soli check` reads a copied file on its own, where `next` in a `while`
# is a name it cannot see.
def md_edit_read(lines, at)
  mp2_said = lines[at].strip()
  if md_starts(mp2_said, "```")
    mp2_lang = mp2_said.substring(3, mp2_said.length()).strip()
    mp2_body = []
    mp2_to = at + 1
    while mp2_to < lines.length() && !md_starts(lines[mp2_to].strip(), "```")
      mp2_body = mp2_body.concat([lines[mp2_to]])
      mp2_to = mp2_to + 1
    end
    return [{"kind": "code", "t": mp2_body.join("\n"), "lang": mp2_lang}, mp2_to + 1]
  end

  mp2_shot = md_image_of(mp2_said)
  unless mp2_shot["src"].nil?
    return [{
      "kind": "image", "t": mp2_shot["t"] ?? "", "src": mp2_shot["src"],
      "w": mp2_shot["w"] ?? 0, "h": mp2_shot["h"] ?? 0
    }, at + 1]
  end

  mp2_kept = md_file_of(mp2_said)
  unless mp2_kept["src"].nil?
    return [{
      "kind": "file", "t": mp2_kept["t"] ?? "", "src": mp2_kept["src"],
      "note": mp2_kept["note"] ?? ""
    }, at + 1]
  end

  return [{"kind": "rule", "t": ""}, at + 1] if mp2_said == "---" || mp2_said == "***" || mp2_said == "___"

  # A run of lines beginning with a pipe is one block, not one a line: a
  # table is the only thing here whose shape is two-dimensional, and a row
  # of it on its own means nothing.
  if md_starts(mp2_said, "|")
    mp2_rows = []
    mp2_to = at
    while mp2_to < lines.length() && md_starts(lines[mp2_to].strip(), "|")
      mp2_cells = md_table_cells(lines[mp2_to])
      mp2_rows = mp2_rows.concat([mp2_cells]) unless md_edit_ruler?(mp2_cells)
      mp2_to = mp2_to + 1
    end
    return [{"kind": "table", "t": "", "rows": mp2_rows}, mp2_to]
  end

  mp2_kind = md_edit_kind_of(mp2_said)
  mp2_out = {"kind": mp2_kind, "t": md_edit_bare(mp2_said)}
  mp2_out["done"] = md_edit_done?(mp2_said) if mp2_kind == "task"
  [mp2_out, at + 1]
end

# The `---|---` under a table's head. It carries the alignment markdown uses
# and this renderer does not, so it is read and dropped rather than kept as
# a row of dashes nobody asked to edit.
def md_edit_ruler?(cells)
  return false if cells.length() == 0

  for mr2_cell in cells
    return false unless md_edit_dashes?(mr2_cell)
  end
  true
end

def md_edit_dashes?(said)
  mr3_at = 0
  return false if said.length() == 0

  while mr3_at < said.length()
    mr3_ch = said.substring(mr3_at, mr3_at + 1)
    return false unless mr3_ch == "-" || mr3_ch == ":" || mr3_ch == " "

    mr3_at = mr3_at + 1
  end
  md_find(said, "-", 0) >= 0
end

# Markdown in, blocks out. One line is one block: a paragraph in an editor
# is a thing you put a caret in, not a run of lines the renderer will join.
#
# An empty document still answers one empty paragraph. An editor with
# nothing to put the caret in is an editor nobody can start typing in.
def md_edit_parse(source)
  mk_out = []
  mk_lines = source.split("\n")
  mk_at = 0
  while mk_at < mk_lines.length()
    if mk_lines[mk_at].strip() == ""
      mk_at = mk_at + 1
    else
      mk_got = md_edit_read(mk_lines, mk_at)
      mk_out = mk_out.concat([{"id": mk_out.length() + 1}.merge(mk_got[0])])
      mk_at = mk_got[1]
    end
  end
  mk_out.length() == 0 ? [{"id": 1, "kind": "p", "t": ""}] : mk_out
end

# A picture's or a file's title field: the measure for one, the note for the
# other. `md_target` reads both back.
def md_edit_title(one)
  mq_w = one["w"] ?? 0
  mq_h = one["h"] ?? 0
  return " \"" + str(mq_w) + "x" + str(mq_h) + "\"" if mq_w > 0 && mq_h > 0

  mq_note = (one["note"] ?? "").to_s
  return " \"" + mq_note + "\"" unless mq_note == ""

  ""
end

# One block as markdown. `n` is how far down an ordered list it is, and 0
# everywhere else.
def md_edit_line(one, n)
  ml_kind = (one["kind"] ?? "p").to_s
  ml_said = (one["t"] ?? "").to_s
  return "---" if ml_kind == "rule"
  return "```" + (one["lang"] ?? "").to_s + "\n" + ml_said + "\n```" if ml_kind == "code"
  return "![" + ml_said + "](" + (one["src"] ?? "").to_s + md_edit_title(one) + ")" if ml_kind == "image"
  return "[" + ml_said + "](" + (one["src"] ?? "").to_s + md_edit_title(one) + ")" if ml_kind == "file"
  return md_edit_grid(one) if ml_kind == "table"
  return "- [" + (one["done"] == true ? "x" : " ") + "] " + ml_said if ml_kind == "task"
  return str(n) + ". " + ml_said if ml_kind == "number"
  # `">"` and not `"> "`: an empty quote line is what a quoted letter is
  # mostly made of, and a trailing space on every one of them is what a
  # reader shows as a ragged right edge.
  return ">" if ml_kind == "quote" && ml_said == ""

  md_edit_mark(ml_kind) + ml_said
end

# A table as the lines it came from, the ruler under the head put back.
# Every row is padded to the widest so a document that is read as text is
# still a table; the renderer does not care and a person does.
def md_edit_grid(one)
  mg2_rows = one["rows"] ?? []
  return "" if mg2_rows.length() == 0

  mg2_wide = 0
  for mg2_row in mg2_rows
    mg2_wide = mg2_row.length() if mg2_row.length() > mg2_wide
  end
  mg2_out = []
  mg2_at = 0
  while mg2_at < mg2_rows.length()
    mg2_out = mg2_out.concat([md_edit_grid_row(mg2_rows[mg2_at], mg2_wide)])
    mg2_out = mg2_out.concat([md_edit_grid_rule(mg2_wide)]) if mg2_at == 0
    mg2_at = mg2_at + 1
  end
  mg2_out.join("\n")
end

# `mg3_at < cells.length()` and not `cells[mg3_at] ?? ""`: indexing an array
# past its end **raises** in Soli, where a missing hash key answers nil. The
# `??` never runs. A ragged table is the ordinary case here — markdown does
# not require the rows to agree — so this is the common path, not the odd one.
def md_edit_grid_row(cells, wide)
  mg3_out = "|"
  mg3_at = 0
  while mg3_at < wide
    mg3_out = mg3_out + " " + (mg3_at < cells.length() ? cells[mg3_at].to_s : "") + " |"
    mg3_at = mg3_at + 1
  end
  mg3_out
end

def md_edit_grid_rule(wide)
  mg4_out = "|"
  mg4_at = 0
  while mg4_at < wide
    mg4_out = mg4_out + " --- |"
    mg4_at = mg4_at + 1
  end
  mg4_out
end

# Blocks out, markdown in — the document as it will be stored, sent or read
# by anything that is not this editor.
#
# A blank line between blocks, except between two items of the same list:
# `md_blocks` groups consecutive markers into one list, and a blank line
# between them would make three lists of one item each.
def md_edit_source(blocks)
  ms_out = []
  ms_at = 0
  ms_n = 0
  while ms_at < blocks.length()
    ms_kind = (blocks[ms_at]["kind"] ?? "p").to_s
    ms_n = ms_kind == "number" ? ms_n + 1 : 0
    ms_out = ms_out.concat([md_edit_line(blocks[ms_at], ms_n)])
    ms_after = ms_at + 1 < blocks.length() ? (blocks[ms_at + 1]["kind"] ?? "p").to_s : ""
    ms_run = ms_after == ms_kind && (ms_kind == "bullet" || ms_kind == "number" || ms_kind == "task")
    ms_out = ms_out.concat([""]) unless ms_run
    ms_at = ms_at + 1
  end
  ms_out.join("\n").strip()
end

# Where a block is, or -1.
def md_edit_index(blocks, id)
  mz_at = 0
  while mz_at < blocks.length()
    return mz_at if blocks[mz_at]["id"] == id

    mz_at = mz_at + 1
  end
  -1
end

# The block itself, or `{}`.
def md_edit_at(blocks, id)
  mz2 = md_edit_index(blocks, id)
  mz2 < 0 ? {} : blocks[mz2]
end

# An id nothing in the document is using. Derived rather than carried, so a
# caller never has to keep a counter in step with the list.
def md_edit_fresh(blocks)
  mj_top = 0
  for mj_one in blocks
    mj_top = mj_one["id"] ?? 0 if (mj_one["id"] ?? 0) > mj_top
  end
  mj_top + 1
end

# The id `delta` blocks away, or `id` itself at either end — where the caret
# goes when an arrow is reported.
def md_edit_step(blocks, id, delta)
  mv_at = md_edit_index(blocks, id)
  return id if mv_at < 0

  mv_to = mv_at + delta
  return id if mv_to < 0 || mv_to >= blocks.length()

  blocks[mv_to]["id"]
end

# `put` in place of `drop` blocks at `at`, as a new list.
#
# Every mutator below goes through this, and the reason is one line of it:
# `concat` **writes into the array it is called on** and `slice` copies. So
# the first call has to be on a slice, or the caller's document is edited in
# place and the handler's `state` has changed before it decided to change
# it. Nothing about that fails loudly; it shows up as a block that came back
# after it was deleted.
def md_edit_splice(blocks, at, drop, put)
  blocks.slice(0, at).concat(put).concat(blocks.slice(at + drop, blocks.length()))
end

# One block, retyped by what was typed into it.
#
# A marker at the head changes what the block is and comes off the text,
# which is the autoformat everyone expects: "## " makes a heading, "- " a
# bullet, "```" a code block. It does not fire on a block that is already
# that kind — "- " at the head of a bullet is a bullet whose text begins
# with a dash — and never on a picture or a file.
def md_edit_retype(one, id, said)
  mr_was = (one["kind"] ?? "p").to_s
  mr_out = {"id": id, "kind": mr_was, "t": said}
  mr_out["lang"] = one["lang"] unless one["lang"].nil?
  mr_out["src"] = one["src"] unless one["src"].nil?
  mr_out["rows"] = one["rows"] unless one["rows"].nil?
  mr_out["done"] = one["done"] unless one["done"].nil?
  return mr_out unless md_edit_text?(mr_was)

  if md_starts(said, "```")
    mr_out["kind"] = "code"
    mr_out["lang"] = said.substring(3, said.length()).strip()
    mr_out["t"] = ""
    return mr_out
  end

  mr_kind = md_edit_kind_of(said)
  if mr_kind != "p" && mr_kind != mr_was
    mr_out["kind"] = mr_kind
    mr_out["t"] = md_edit_bare(said)
    mr_out["done"] = md_edit_done?(said) if mr_kind == "task"
  end
  mr_out
end

# What `change` does. The whole settled value arrives (06 §2), never a
# keystroke, so this is the one place a block's text is written.
#
# A value with newlines in it can only be a paste — `Enter` is a `submit`
# here — and becomes one block a line rather than one block holding a
# document. `next_id` names the first of the extra blocks; pass
# `md_edit_fresh(blocks)`.
def md_edit_set(blocks, id, said, next_id)
  mw_at = md_edit_index(blocks, id)
  return blocks if mw_at < 0

  mw_one = blocks[mw_at]
  if md_edit_verbatim?((mw_one["kind"] ?? "p").to_s)
    mw_kept = mw_one.merge({"t": said})
    return md_edit_splice(blocks, mw_at, 1, [mw_kept])
  end

  mw_lines = said.split("\n")
  mw_made = []
  mw_n = 0
  while mw_n < mw_lines.length()
    mw_made = mw_made.concat([md_edit_retype(mw_one, mw_n == 0 ? id : next_id + mw_n - 1, mw_lines[mw_n])])
    mw_n = mw_n + 1
  end
  md_edit_splice(blocks, mw_at, 1, mw_made)
end

# What `submit` does: `Enter` at the end of a block.
#
# An empty list item leaves the list instead of making another one, which is
# what every editor does and what everyone reaches for. Anything else gets a
# new block after it, carrying a list on and starting a paragraph after
# anything else — a heading is followed by prose, never by another heading.
def md_edit_split(blocks, id, next_id)
  mu_at = md_edit_index(blocks, id)
  return blocks if mu_at < 0

  mu_one = blocks[mu_at]
  mu_kind = (mu_one["kind"] ?? "p").to_s
  mu_list = mu_kind == "bullet" || mu_kind == "number"
  if mu_list && (mu_one["t"] ?? "").to_s == ""
    return md_edit_splice(blocks, mu_at, 1, [mu_one.merge({"kind": "p"})])
  end

  md_edit_splice(blocks, mu_at + 1, 0, [{"id": next_id, "kind": mu_list ? mu_kind : "p", "t": ""}])
end

# What `Backspace` at the head of a block does, in three steps, because one
# key has three jobs here and they have an order:
#
#   a block that is not a paragraph  loses its marker and becomes one
#   a picture or a file above it     is removed; nothing about a picture
#                                    can be merged into a sentence, and
#                                    there is no other key for deleting one
#   anything else                    joins what is above it
def md_edit_merge(blocks, id)
  mh_at = md_edit_index(blocks, id)
  return blocks if mh_at < 0

  mh_one = blocks[mh_at]
  mh_kind = (mh_one["kind"] ?? "p").to_s
  return blocks unless md_edit_text?(mh_kind)

  if mh_kind != "p"
    return md_edit_splice(blocks, mh_at, 1, [mh_one.merge({"kind": "p"})])
  end
  return blocks if mh_at < 1

  mh_prev = blocks[mh_at - 1]
  return md_edit_splice(blocks, mh_at - 1, 1, []) unless md_edit_text?((mh_prev["kind"] ?? "p").to_s)

  mh_joined = mh_prev.merge({"t": (mh_prev["t"] ?? "").to_s + (mh_one["t"] ?? "").to_s})
  md_edit_splice(blocks, mh_at - 1, 2, [mh_joined])
end

# The toolbar's block buttons. Pressing the kind a block already is puts it
# back to a paragraph, so one button both sets and clears.
def md_edit_kind(blocks, id, kind)
  mc_at = md_edit_index(blocks, id)
  return blocks if mc_at < 0

  mc_one = blocks[mc_at]
  return blocks unless md_edit_text?((mc_one["kind"] ?? "p").to_s)

  mc_to = (mc_one["kind"] ?? "p").to_s == kind ? "p" : kind
  md_edit_splice(blocks, mc_at, 1, [mc_one.merge({"kind": mc_to})])
end

# `B` and `I`: the mark goes round the whole block, and comes off again when
# it is already there. Not round a selection, because there is no selection
# a server can see (08 §7.1) — and a button that pretended otherwise would
# put its marks somewhere nobody asked for.
def md_edit_wrap(blocks, id, mark)
  my_at = md_edit_index(blocks, id)
  return blocks if my_at < 0

  my_one = blocks[my_at]
  return blocks unless md_edit_text?((my_one["kind"] ?? "p").to_s)

  my_said = (my_one["t"] ?? "").to_s
  return blocks if my_said == ""

  my_wide = mark.length() * 2
  my_on = md_starts(my_said, mark) && md_ends(my_said, mark) && my_said.length() > my_wide
  my_to = my_on ? my_said.substring(mark.length(), my_said.length() - mark.length()) : mark + my_said + mark
  md_edit_splice(blocks, my_at, 1, [my_one.merge({"t": my_to})])
end

# A block after `id`, or at the end when nothing has that id — what an
# upload, a rule and a caller building its own block all need.
def md_edit_put(blocks, id, one)
  mj_at = md_edit_index(blocks, id)
  return md_edit_splice(blocks, blocks.length(), 0, [one]) if mj_at < 0

  md_edit_splice(blocks, mj_at + 1, 0, [one])
end

# And taking one out again — the × on a picture or a file.
def md_edit_drop(blocks, id)
  mj2 = md_edit_index(blocks, id)
  return blocks if mj2 < 0
  return [{"id": md_edit_fresh(blocks), "kind": "p", "t": ""}] if blocks.length() == 1

  md_edit_splice(blocks, mj2, 1, [])
end

# ---- what a document can be asked to do to itself ----------------------

# Tick a task, and only a task.
def md_edit_check(blocks, id)
  mk2_at = md_edit_index(blocks, id)
  return blocks if mk2_at < 0

  mk2_one = blocks[mk2_at]
  return blocks unless (mk2_one["kind"] ?? "p").to_s == "task"

  md_edit_splice(blocks, mk2_at, 1, [mk2_one.merge({"done": mk2_one["done"] != true})])
end

# One cell of a table. Rows are ragged on the way in -- a markdown table
# whose rows disagree is not an error anywhere -- so the row is padded to
# the column being written rather than refused.
def md_edit_cell(blocks, id, row, col, said)
  mc2_at = md_edit_index(blocks, id)
  return blocks if mc2_at < 0

  mc2_one = blocks[mc2_at]
  mc2_rows = mc2_one["rows"] ?? []
  return blocks if row < 0 || row >= mc2_rows.length()

  mc2_row = mc2_rows[row]
  while mc2_row.length() <= col
    mc2_row = md_edit_splice(mc2_row, mc2_row.length(), 0, [""])
  end
  md_edit_splice(blocks, mc2_at, 1, [mc2_one.merge({
    "rows": md_edit_splice(mc2_rows, row, 1, [md_edit_splice(mc2_row, col, 1, [said])])
  })])
end

# How wide a table is: its widest row, because that is what it will be
# written out as.
def md_edit_cols(one)
  mw2_wide = 0
  for mw2_row in one["rows"] ?? []
    mw2_wide = mw2_row.length() if mw2_row.length() > mw2_wide
  end
  mw2_wide
end

# A row on the end, and a column on the end. Both answer the document, not
# the table, so a caller has one shape to hold.
def md_edit_row_add(blocks, id)
  mn2_at = md_edit_index(blocks, id)
  return blocks if mn2_at < 0

  mn2_one = blocks[mn2_at]
  mn2_rows = mn2_one["rows"] ?? []
  mn2_new = []
  mn2_col = 0
  mn2_wide = md_edit_cols(mn2_one)
  while mn2_col < mn2_wide
    mn2_new = mn2_new.concat([""])
    mn2_col = mn2_col + 1
  end
  md_edit_splice(blocks, mn2_at, 1, [mn2_one.merge({"rows": md_edit_splice(mn2_rows, mn2_rows.length(), 0, [mn2_new])})])
end

def md_edit_col_add(blocks, id)
  mo2_at = md_edit_index(blocks, id)
  return blocks if mo2_at < 0

  mo2_one = blocks[mo2_at]
  mo2_wide = md_edit_cols(mo2_one)
  mo2_rows = (mo2_one["rows"] ?? []).map(fn(r) {
    md_edit_splice(r, r.length(), 0, range(r.length(), mo2_wide + 1).map(fn(i) { "" }))
  })
  md_edit_splice(blocks, mo2_at, 1, [mo2_one.merge({"rows": mo2_rows})])
end

# An empty table, which is what the toolbar makes: a head and one row, two
# columns, because a table of one column is a list and markdown already has
# one of those.
def md_edit_table(id)
  # The space after the outer bracket is load-bearing. `[["a", "b"], …]`
  # — an array of arrays whose first element is a *string* — lexes as a
  # string in Soli 2.3.7 and comes out as one, silently:
  #
  #   [[1, 2], [3, 4]]   -> array          [["x", "y"], ["z"]] -> string
  #   [ ["x"], ["y"] ]   -> array          [[], []]            -> string
  #
  # A space, a pair of parentheses or an intermediate variable all avoid it.
  {"id": id, "kind": "table", "t": "", "rows": [ ["", ""], ["", ""] ]}
end

# ---- moving a block ----------------------------------------------------

# `id` to `slot`, taken out of wherever it was first. `slot` is counted in
# the document as it stands, which is what a `drop` reports (06 §6) -- so
# moving a block down by one is a slot two further on, and the correction
# is here rather than in every caller.
def md_edit_move_to(blocks, id, slot)
  mv2_at = md_edit_index(blocks, id)
  return blocks if mv2_at < 0

  mv2_rest = md_edit_splice(blocks, mv2_at, 1, [])
  mv2_to = slot > mv2_at ? slot - 1 : slot
  mv2_to = 0 if mv2_to < 0
  mv2_to = mv2_rest.length() if mv2_to > mv2_rest.length()
  md_edit_splice(mv2_rest, mv2_to, 0, [blocks[mv2_at]])
end

# One place up or down, which is what Alt and an arrow do. At either end it
# is a no-op rather than a wrap: a block that reappears at the other end of
# a document is a block nobody can find again.
def md_edit_shift(blocks, id, delta)
  ms2_at = md_edit_index(blocks, id)
  return blocks if ms2_at < 0

  ms2_to = ms2_at + delta
  return blocks if ms2_to < 0 || ms2_to >= blocks.length()

  md_edit_splice(md_edit_splice(blocks, ms2_at, 1, []), ms2_to, 0, [blocks[ms2_at]])
end

# ---- undoing -----------------------------------------------------------
#
# A stack of block lists, which is the cheapest thing that works: a document
# is a list of small hashes, so keeping the last forty costs less than the
# frame that drew them once.
#
# Typing is coalesced, and has to be. `change` arrives when a field goes
# quiet (06 §2), so an uncoalesced stack would hold one entry per pause and
# Ctrl+Z would walk back through a sentence a breath at a time. Consecutive
# changes to the same block replace the top of the stack instead of pushing.

# How far back it goes is a default argument and not a constant, because the
# specs copy these definitions one at a time and a constant beside them does
# not travel. Forty is where a stack stops being something you think about
# and starts being something you would have to scroll.
def md_edit_remember(doc, why, id, cap = 40)
  md_past = doc["past"] ?? []
  md_top = md_past.length() == 0 ? {} : md_past[md_past.length() - 1]
  md_same = why == "change" && (md_top["why"] ?? "") == "change" && (md_top["id"] ?? -1) == id
  return doc.merge({"future": []}) if md_same

  md_kept = md_edit_splice(md_past, md_past.length(), 0, [{
    "blocks": doc["blocks"] ?? [], "why": why, "id": id, "focus": doc["focus"] ?? 0
  }])
  md_kept = md_edit_splice(md_kept, 0, md_kept.length() - cap, []) if md_kept.length() > cap
  doc.merge({"past": md_kept, "future": []})
end

def md_edit_undo(doc)
  mu2_past = doc["past"] ?? []
  return doc if mu2_past.length() == 0

  mu2_step = mu2_past[mu2_past.length() - 1]
  doc.merge({
    "blocks": mu2_step["blocks"],
    "focus": mu2_step["focus"] ?? 0,
    "take": true,
    "slash": 0,
    "past": md_edit_splice(mu2_past, mu2_past.length() - 1, 1, []),
    "future": md_edit_splice(doc["future"] ?? [], 0, 0, [{
      "blocks": doc["blocks"] ?? [], "why": "undo", "id": 0, "focus": doc["focus"] ?? 0
    }])
  })
end

def md_edit_redo(doc)
  mr4_next = doc["future"] ?? []
  return doc if mr4_next.length() == 0

  mr4_step = mr4_next[0]
  doc.merge({
    "blocks": mr4_step["blocks"],
    "focus": mr4_step["focus"] ?? 0,
    "take": true,
    "slash": 0,
    "future": md_edit_splice(mr4_next, 0, 1, []),
    "past": md_edit_splice(doc["past"] ?? [], (doc["past"] ?? []).length(), 0, [{
      "blocks": doc["blocks"] ?? [], "why": "redo", "id": 0, "focus": doc["focus"] ?? 0
    }])
  })
end

# Whether a key with these modifiers is the accelerator. Control on a
# keyboard and ⌘ on a Mac, and the server cannot tell which machine it is
# talking to (06 §5) -- so it takes either, which is what every
# cross-platform application does.
def md_edit_accel?(mods)
  mods == 2 || mods == 8 || mods == 3 || mods == 9
end

# ---- the names the handlers go by --------------------------------------
#
# Seventeen gestures is seventeen handler names, and an application that
# wrote them out twice — once in the options hash, once in its reducer —
# would have thirty-four places for a typo that fails silently, because a
# handler nobody declared is simply a press that does nothing (08 §3).
#
# So the names come from one prefix. `md_edit_events("note")` is the whole
# options hash, and `md_edit_mine?`/`md_edit_what` take the events back
# apart, which makes the reducer one line:
#
#   return set_doc(state, markdown_editor_step(doc, md_edit_what("note", event), params)) if md_edit_mine?("note", event)

# Every `what` `markdown_editor_step` answers.
def md_edit_whats()
  [
    "change", "submit", "key", "focus", "tool", "pick", "drag", "drop",
    "preview", "check", "cell", "grid", "over", "move", "slash", "ask",
    "link"
  ]
end

def md_edit_events(prefix)
  me2_out = {}
  for me2_what in md_edit_whats()
    me2_out["on_" + me2_what] = prefix + "_" + me2_what
  end
  me2_out
end

def md_edit_what(prefix, event)
  me3_said = event.to_s
  me3_cut = prefix.length() + 1
  me3_cut > me3_said.length() ? "" : me3_said.substring(me3_cut, me3_said.length())
end

def md_edit_mine?(prefix, event)
  return false unless md_starts(event.to_s, prefix + "_")

  md_edit_whats().includes?(md_edit_what(prefix, event))
end

# --------------------------------------------------------------- the widget

# The block buttons, in the order they are drawn. A list rather than a
# literal in the view, so a caller can read it — a shortcut sheet naming the
# same tools should not have to guess them.
def md_edit_tools()
  [
    {"tool": "h1", "glyph": "H1", "label": "Heading"},
    {"tool": "h2", "glyph": "H2", "label": "Subheading"},
    {"tool": "h3", "glyph": "H3", "label": "Sub-subheading"},
    {"tool": "bullet", "glyph": "•", "label": "Bullet list"},
    {"tool": "number", "glyph": "1.", "label": "Numbered list"},
    {"tool": "quote", "glyph": "“", "label": "Quote"},
    {"tool": "task", "icon": "check", "label": "Task list"},
    {"tool": "table", "icon": "grid", "label": "Table"},
    {"tool": "code", "glyph": "</>", "label": "Code"},
    {"tool": "strong", "glyph": "B", "label": "Bold this block"},
    {"tool": "em", "glyph": "I", "label": "Italic this block"},
    {"tool": "rule", "glyph": "—", "label": "Divider"},
    {"tool": "link", "glyph": "Link", "label": "Make this block a link"}
  ]
end

# How wide the gutter to the left of a block is. A bullet and a number need
# room for their marker; a quote gets a bar and the rest get nothing.
def md_edit_gutter(kind)
  return 32 if kind == "task"
  return 16 if kind == "bullet"
  return 26 if kind == "number"
  return 12 if kind == "quote"

  0
end

# What a block is called to an assistive technology. An editable node with
# no label is an unnamed field, and a document of them is unreadable.
def md_edit_label(kind)
  return "Heading" if kind == "h1"
  return "Subheading" if kind == "h2"
  return "Sub-subheading" if kind == "h3"
  return "Quote" if kind == "quote"
  return "List item" if kind == "bullet" || kind == "number"
  return "Code" if kind == "code"

  "Paragraph"
end

# The style of the field itself. The sizes are `md_heading_size`'s, so a
# heading is the same size while it is being written as it is once it is
# read — which is most of what makes this feel like a document rather than a
# form with a big font.
#
# `width` is a number of pixels and has to be: `max_width` does not
# constrain measurement, so a paragraph that will wrap is measured as though
# it will not, comes out one line tall, and overlaps the block beneath it.
def md_edit_look(kind, width, lit)
  el_base = {
    "width": width,
    "pad": [1, 2, 1, 2],
    # `lit == true` and not `lit`: a bare `Any` parameter cannot be a
    # ternary's condition, and the type checker is right to say so.
    "bg": lit == true ? "surface.sunken" : "none",
    "fg": "text.default",
    "border": 0,
    "radius": 1,
    "size": 2,
    "transition": "fast"
  }
  return el_base.merge({"size": 5, "weight": "bold"}) if kind == "h1"
  return el_base.merge({"size": 4, "weight": "bold"}) if kind == "h2"
  return el_base.merge({"size": 3, "weight": "semibold"}) if kind == "h3"
  return el_base.merge({"fg": "text.muted"}) if kind == "quote"
  return el_base.merge({"font": "mono", "size": 1, "bg": "surface.sunken"}) if kind == "code"

  el_base
end

# The handlers every block carries, and only the ones the caller named.
# What a block asks of the keyboard, and the whole of why any of this works
# (03 §3.1): a node with a `key_down` handler and no `keys` hears every key,
# including every letter typed into it.
#
# `z` and `y` are here for undo, and they are the odd pair: a printable
# character is never withheld and cannot be (rule 1), so asking for them
# gets the key report *and* leaves the letter in the field. The handler
# looks at the modifiers and does nothing without them — which is right, and
# is why `md_edit_accel?` exists rather than a comparison inline.
def md_edit_keys()
  ["Backspace", "ArrowUp", "ArrowDown", "Escape", "z", "y", "Z"]
end

def md_edit_hands(o)
  eh_on = {}
  eh_on["change"] = o["on_change"] unless o["on_change"].nil?
  eh_on["submit"] = o["on_submit"] unless o["on_submit"].nil?
  eh_on["key_down"] = o["on_key"] unless o["on_key"].nil?
  eh_on["focus"] = o["on_focus"] unless o["on_focus"].nil?
  eh_on
end

# One editable block.
#
# `keys` is the whole of why this works, and leaving it off is the whole of
# why it would not: a node with a `key_down` handler and no `keys` hears
# every key, including every letter typed into it (03 §3.1). Naming three
# means three, and the client keeps the rest — including every printable
# character, which never reaches a handler at all.
def md_edit_field(one, o, focus, take, width)
  ef_id = one["id"] ?? 0
  ef_kind = (one["kind"] ?? "p").to_s
  ef_lit = ef_id == focus
  ef_wide = width - md_edit_gutter(ef_kind)
  ef_wide = 80 if ef_wide < 80
  ef_props = {
    "block": ef_id,
    "label": md_edit_label(ef_kind),
    "keys": md_edit_keys()
  }
  # Only on the batch that asked for it. A `focus_to` left standing is
  # harmless — it fires on the rising edge alone — but one sent every frame
  # is a `Focus` op every frame for a caret that is already there.
  ef_props["focus_to"] = true if ef_lit && take == true
  ef_look = md_edit_look(ef_kind, ef_wide, ef_lit)
  ef_look = ef_look.merge({"height": md_edit_tall((one["t"] ?? "").to_s)}) if ef_kind == "code"
  {
    "k": ef_kind == "code" ? "textarea" : "input",
    "key": (o["key"] ?? "mdedit") + ":b" + str(ef_id),
    "t": (one["t"] ?? "").to_s,
    "s": ef_look,
    "p": ef_props,
    "on": md_edit_hands(o)
  }
end

# A code block is the one field that has to be told how tall it is: a
# `textarea` does not scroll to its own caret (03 §3), so text past the
# bottom of a fixed box is text nobody can look at. It grows with what is in
# it instead.
def md_edit_tall(said)
  et_lines = said.split("\n").length()
  et_lines = 3 if et_lines < 3
  et_lines * 18 + 16
end

# The marker beside a list item, and the bar beside a quote. Neither is part
# of the field: a marker inside the text would be typed over, and a quote's
# bar as a border on the field would move the field two pixels every time it
# was focused.
#
# A marker sits in a box a control tall rather than on its own, because an
# editable node is laid out at least one control tall and centres its line
# in that (03 §3, `eui-layout`). A bare `text` beside it starts at the top of
# the row, which puts every bullet a few pixels above the word it belongs
# to -- visible, and exactly the sort of thing nobody can name when they say
# a page looks unfinished.
def md_edit_gutter_node(kind, n)
  return md_edit_marker(str(n) + ".", 26) if kind == "number"
  return md_edit_marker("•", 16) if kind == "bullet"
  if kind == "quote"
    return {"k": "box", "s": {"width": 3, "margin": [0, 2, 0, 0], "bg": "accent.base", "radius": 1, "shrink": 0}}
  end

  {"k": "box", "s": {"width": 0, "height": 0}}
end

def md_edit_marker(glyph, width)
  {
    "k": "box",
    "s": {"width": width, "height": 36, "display": "row", "align": "center", "shrink": 0},
    "c": [text(glyph, {"size": 2, "fg": "text.muted"})]
  }
end

# A picture in the document. The caption is a field of its own, so alt text
# can be written where the picture is rather than in a dialog somewhere; the
# × is the only way a picture leaves, besides a `Backspace` in the block
# under it.
def md_edit_figure(one, o, focus, take, width)
  eg_id = one["id"] ?? 0
  eg_wide = width - 40
  eg_wide = 120 if eg_wide < 120
  column({"gap": 1, "width": width, "pad": [1, 0, 1, 0]}, [
    row({"gap": 2, "align": "start", "width": width}, [
      md_picture(one, eg_wide),
      md_edit_remove(one, o)
    ]),
    md_edit_caption(one, o, focus, take, eg_wide, "Caption")
  ])
end

# A file that is not a picture: the catalogue's own card, and its name is
# the field — renaming an attachment where it sits is the same gesture as
# captioning a photograph.
def md_edit_card(one, o, focus, take, width)
  ec_wide = width - 40
  ec_wide = 120 if ec_wide < 120
  column({"gap": 1, "width": width, "pad": [1, 0, 1, 0]}, [
    row({"gap": 2, "align": "center", "width": width}, [
      {"k": "box", "s": {"grow": 1, "shrink": 1, "min_width": 0, "display": "column"}, "c": [md_attachment(one)]},
      md_edit_remove(one, o)
    ]),
    md_edit_caption(one, o, focus, take, ec_wide, "Name")
  ])
end

# The one-line field under a picture or a card. Its kind is `image` or
# `file`, so `md_edit_set` writes it verbatim and no marker typed into it
# turns a photograph into a heading.
def md_edit_caption(one, o, focus, take, width, label)
  ep_id = one["id"] ?? 0
  ep_props = {"block": ep_id, "label": label, "keys": md_edit_keys()}
  ep_props["focus_to"] = true if ep_id == focus && take == true
  {
    "k": "input",
    "key": (o["key"] ?? "mdedit") + ":c" + str(ep_id),
    "t": (one["t"] ?? "").to_s,
    "s": {
      "width": width, "pad": [1, 2, 1, 2], "size": 1, "fg": "text.muted",
      "bg": ep_id == focus ? "surface.sunken" : "none", "border": 0, "radius": 1
    },
    "p": ep_props,
    "on": md_edit_hands(o)
  }
end

def md_edit_remove(one, o)
  control({
    "key": (o["key"] ?? "mdedit") + ":x" + str(one["id"] ?? 0),
    "tone": "ghost",
    "size": "sm",
    "shape": {"min_width": 28, "width": 28, "height": 28, "pad": [0, 0, 0, 0]},
    "props": {"block": one["id"] ?? 0},
    "a11y": {"role": "button", "label": "Remove"},
    "on": o["on_drop"].nil? ? {} : {"click": o["on_drop"]},
    "c": [icon("close", {"size": 1, "fg": "text.muted"})]
  })
end

# A divider is a block like any other: it can be focused, so `Backspace` in
# the block under it removes it, and it has nothing to type into.
def md_edit_rule(one, o, width)
  row({"gap": 2, "align": "center", "width": width, "pad": [2, 0, 2, 0]}, [
    {"k": "box", "s": {"grow": 1, "shrink": 1, "height": 1, "bg": "border.default"}},
    md_edit_remove(one, o)
  ])
end

# A task's box. A real `checkbox`, not a glyph: a tick you can press is the
# whole difference between a note with a list in it and a note with a list
# you keep.
def md_edit_box(one, o)
  checkbox("", one["done"] == true, o["on_check"] ?? "", {"block": one["id"] ?? 0}, {
    "key": (o["key"] ?? "mdedit") + ":k" + str(one["id"] ?? 0),
    "name": "Done",
    "size": "sm"
  })
end

# A table, as a grid of fields.
#
# Every cell is its own `input` and carries where it is (03 §4), so one
# handler serves a table of any size. `Tab` walks them in reading order for
# free: the focus order is the nodes holding handlers (03 §3), and nothing
# here claims `Tab`, which it could not anyway.
def md_edit_table_node(one, o, width)
  eg2_id = one["id"] ?? 0
  eg2_rows = one["rows"] ?? []
  eg2_wide = md_edit_cols(one)
  eg2_wide = 1 if eg2_wide < 1
  eg2_cell = int((width - 8 * eg2_wide) / eg2_wide)
  eg2_cell = 60 if eg2_cell < 60
  eg2_lines = range(0, eg2_rows.length()).map(fn(r) {
    md_edit_table_row(one, o, r, eg2_wide, eg2_cell)
  })
  column({"gap": 1, "width": width, "pad": [1, 0, 1, 0]}, eg2_lines.concat([
    row({"gap": 2, "align": "center", "width": width}, [
      md_edit_grid_button("row", "Add a row", o, eg2_id),
      md_edit_grid_button("col", "Add a column", o, eg2_id),
      spacer,
      md_edit_remove(one, o)
    ])
  ]))
end

def md_edit_table_row(one, o, r, wide, cell)
  eg3_id = one["id"] ?? 0
  eg3_row = (one["rows"] ?? [])[r]
  row({"gap": 2, "align": "start"}, range(0, wide).map(fn(c) {
    {
      "k": "input",
      "key": (o["key"] ?? "mdedit") + ":g" + str(eg3_id) + "-" + str(r) + "-" + str(c),
      "t": c < eg3_row.length() ? eg3_row[c].to_s : "",
      "s": {
        "width": cell, "pad": [1, 2, 1, 2], "size": 1, "radius": 1,
        "bg": r == 0 ? "surface.sunken" : "none",
        "weight": r == 0 ? "semibold" : "regular",
        "fg": "text.default",
        "border": [0, 0, 1, 0], "border_color": "border.subtle"
      },
      "p": {"block": eg3_id, "row": r, "col": c, "label": r == 0 ? "Column heading" : "Cell"},
      "on": o["on_cell"].nil? ? {} : {"change": o["on_cell"]}
    }
  }))
end

def md_edit_grid_button(what, label, o, id)
  control({
    "key": (o["key"] ?? "mdedit") + ":" + what + str(id),
    "tone": "ghost",
    "size": "sm",
    "shape": {"min_width": 0, "pad": [1, 2, 1, 2]},
    "props": {"block": id, "what": what},
    "a11y": {"role": "button", "label": label},
    "on": o["on_grid"].nil? ? {} : {"click": o["on_grid"]},
    "c": [text(label, {"size": 0, "fg": "text.muted"})]
  })
end

# The grip.
#
# It is drawn on every block and shows its icon on the focused one, because
# **a tree that changes shape under a field loses what is in it**. A grip
# that appeared on focus made the block a different node, the client
# replaced it rather than patching it, and the fresh node arrived with an
# empty `Edit`: the first character typed after clicking in went nowhere at
# all. Nothing said a word. So the box is always here and what changes is
# its contents and one prop.
#
# `drag` is that prop, and it is only on the focused block because a node
# carrying it takes focus on `Tab` (03 §3.4) — one per document rather than
# one per block. The `drag_handle` child is the stop that picks it up, so
# the keyboard path is `Tab` to the grip, `Space`, the arrows, `Space`.
def md_edit_grip(o, lit)
  {
    "k": "box",
    "s": {
      "display": "row", "align": "center", "justify": "center",
      "width": 18, "height": 36, "shrink": 0,
      "cursor": lit == true ? "grab" : "default",
      "fg": "text.muted", "radius": 1
    },
    "p": {"drag_handle": true, "role": "button", "label": "Move this block"},
    "c": lit == true ? [{"k": "icon", "s": {"width": 14, "height": 14}, "p": {"name": "grip"}}] : []
  }
end

# One group name per editor, so two editors on one page cannot take each
# other's blocks.
def md_edit_group(o)
  "mdblock:" + (o["key"] ?? "mdedit")
end

# The `/` panel.
#
# Typing `/` at the head of a block opens the list of kinds **where the
# caret is**, which is the gesture people reach for and the one the bar
# cannot be: a bar is somewhere else on the screen, and a block editor is a
# thing you use without looking up. It is a `dropdown`, so the client
# decides where the panel lands and it is never clipped by the scroller
# (03 §1, the top layer).
def md_edit_slash_panel(anchor, one, o)
  es2_open = (o["slash"] ?? 0) == (one["id"] ?? 0)
  es2_hits = es2_open ? md_edit_slash_hits((one["t"] ?? "").to_s) : []
  # The stack is here whether the panel is or not, and the panel's place in
  # it is an empty box when it is shut. Adding a sibling over a field is a
  # change of shape, and a change of shape under a field is a field that
  # loses what is in it -- the same rule `md_edit_grip` is written for.
  es2_panel = {"k": "box", "s": {"width": 0, "height": 0}}
  unless es2_hits.length() == 0
    es2_at = o["at"] ?? 0
    es2_panel = {
      "k": "overlay",
      "s": {
        "position": "absolute", "margin": [2, 0, 0, 0], "pad": 1,
        "radius": 2, "shadow": 2, "bg": "surface.overlay",
        "border": 1, "border_color": "border.subtle",
        "display": "column", "max_width": 280, "z": 5
      },
      "c": [scroll({"gap": 0, "max_height": 260}, range(0, es2_hits.length()).map(fn(i) {
        md_edit_slash_row(es2_hits[i], i == es2_at, o)
      }))]
    }
    es2_panel["on"] = {"blur": o["on_slash"]} unless o["on_slash"].nil?
  end
  stack({"gap": 0}, [anchor, es2_panel])
end

# What is still worth offering under what has been typed. An empty query
# offers everything, for the reason `combo_filter` does: a panel that
# appears only once you have typed is a panel most people never learn is
# there.
def md_edit_slash_hits(said)
  return [] unless md_starts(said, "/")

  es3_q = said.substring(1, said.length()).strip().downcase()
  return md_edit_tools() if es3_q == ""

  md_edit_tools().filter(fn(t) {
    md_find(t["label"].to_s.downcase(), es3_q, 0) >= 0 || md_find(t["tool"].to_s, es3_q, 0) >= 0
  })
end

def md_edit_slash_row(spec, lit, o)
  {
    "k": "box",
    "key": (o["key"] ?? "mdedit") + ":s" + spec["tool"].to_s,
    "s": {
      "display": "row", "gap": 2, "align": "center",
      "pad": [1, 3, 1, 3], "radius": 1, "min_width": 200,
      "bg": lit == true ? "surface.sunken" : "none",
      "cursor": "pointer"
    },
    "p": {"tool": spec["tool"], "role": "button", "label": spec["label"]},
    "on": o["on_slash"].nil? ? {} : {"click": o["on_slash"]},
    "c": [md_edit_tool_face(spec), text(spec["label"].to_s, {"size": 1})]
  }
end

# Where a link's address is asked for.
#
# A link needs two things a block does not have: an address, and somewhere
# to put it. The block is the text; this is the address. It is a `dialog`
# rather than a field in the bar because a bar that grows a text box when
# you press one of its buttons is a bar that moves under the hand.
def md_edit_ask(doc, o)
  ea2_ask = doc["asking"] ?? {}
  return [] if (ea2_ask["block"] ?? 0) == 0

  dialog("Link", [
    text_field("Address", (ea2_ask["url"] ?? "").to_s, (o["on_ask"] ?? "").to_s, {
      "key": (o["key"] ?? "mdedit") + ":ask",
      "hint": "https://… , or a path this application serves",
      "autofocus": true
    })
  ], [
    md_edit_ask_button("Cancel", false, o),
    md_edit_ask_button("Link it", true, o)
  ], {"key": (o["key"] ?? "mdedit") + ":asking", "width": 420})
end

def md_edit_ask_button(label, go, o)
  control({
    "key": (o["key"] ?? "mdedit") + ":ask" + (go == true ? "go" : "no"),
    "tone": go == true ? "accent" : "neutral",
    "props": {"go": go},
    "a11y": {"role": "button", "label": label},
    "on": o["on_link"].nil? ? {} : {"click": o["on_link"]},
    "c": [text(label, {"weight": "semibold"})]
  })
end

# One block, whichever kind it is: the grip, the block, and the panel that
# may hang under it. The shape never changes — see `md_edit_grip` for what
# happens when it does.
def md_edit_block(one, o, focus, take, width, n)
  eo_id = one["id"] ?? 0
  eo_lit = eo_id == focus
  eo_props = {"block": eo_id}
  eo_props["drag"] = md_edit_group(o) if eo_lit
  eo_held = {
    "k": "box",
    "key": (o["key"] ?? "mdedit") + ":d" + str(eo_id),
    "s": {"display": "row", "align": "start", "width": width},
    "p": eo_props,
    "c": [md_edit_grip(o, eo_lit), md_edit_body(one, o, focus, take, width - 18, n)]
  }
  md_edit_slash_panel(eo_held, one, o)
end

# What a block is, before the grip and the panel are put round it.
def md_edit_body(one, o, focus, take, width, n)
  eo2_kind = (one["kind"] ?? "p").to_s
  return md_edit_figure(one, o, focus, take, width) if eo2_kind == "image"
  return md_edit_card(one, o, focus, take, width) if eo2_kind == "file"
  return md_edit_rule(one, o, width) if eo2_kind == "rule"
  return md_edit_table_node(one, o, width) if eo2_kind == "table"

  # `stretch`, so a quote's bar is as tall as the quote it marks. The marker
  # boxes carry a height of their own and keep it.
  row({"gap": 0, "align": "stretch", "width": width}, [
    eo2_kind == "task" ? md_edit_box(one, o) : md_edit_gutter_node(eo2_kind, n),
    md_edit_field(one, o, focus, take, width)
  ])
end

# One toolbar button. `selected` is what the focused block already is, so
# the bar says where you are as well as what you can do.
def md_edit_tool(spec, o, kind)
  control({
    "key": (o["key"] ?? "mdedit") + ":t" + spec["tool"].to_s,
    "tone": "ghost",
    "size": "sm",
    "shape": {"min_width": 32, "pad": [1, 2, 1, 2]},
    "selected": spec["tool"] == kind,
    "props": {"tool": spec["tool"]},
    "a11y": {"role": "button", "label": spec["label"]},
    "on": o["on_tool"].nil? ? {} : {"click": o["on_tool"]},
    "c": [md_edit_tool_face(spec)]
  })
end

# A tool is a glyph or an icon. The icons are the client's own set (03 §2),
# which is why a task is a tick and a table is a grid: nothing in the two
# faces a client carries draws a checkbox or a table, and a glyph it has no
# outline for is a gap of the right size and nothing else.
def md_edit_tool_face(spec)
  return {"k": "icon", "s": {"width": 16, "height": 16}, "p": {"name": spec["icon"]}} unless spec["icon"].nil?

  text(spec["glyph"].to_s, {"size": 1, "weight": "semibold"})
end

# The bar, on its own, for a caller that wants it somewhere else.
#
# **A bar cannot be made to stick to the top of a document that scrolls.**
# Sticky positioning is not in version 1 (04 §9), and the server never
# learns a scroll offset it did not ask a handler for (06 §8), so there is
# nothing to follow the document with. What there is, is putting the bar
# **outside the scroller**: a `column` of [bar, `scroll`] leaves it where it
# is for as long as the surface is up, which is what every masthead in this
# repository already does.
#
# So `markdown_editor(doc, o.merge({"bar": false}))` draws the document
# alone, and this draws the bar to put above the scroller holding it. Both
# read the same `doc`, so the buttons still light for the block the caret is
# in.
def markdown_editor_bar(doc, o = {})
  eb_blocks = doc["blocks"] ?? []
  md_edit_bar(doc, o, (md_edit_at(eb_blocks, doc["focus"] ?? 0)["kind"] ?? "").to_s)
end

# `files` comes off when `fs.pick` was not granted — the browser build cuts
# file pick and save on purpose, and an editor that draws a dead attach
# button there is worse than one that draws none.
def md_edit_bar(doc, o, kind)
  eu_tools = md_edit_tools().map(fn(t) { md_edit_tool(t, o, kind) })
  unless o["files"] == false || o["on_pick"].nil?
    eu_tools = eu_tools.concat([file_field("Attach", o["accept"] ?? MD_EDIT_ACCEPT, o["on_pick"], {
      "key": (o["key"] ?? "mdedit") + ":pick",
      "tone": "ghost",
      "size": "sm",
      "max": o["max"] ?? MD_EDIT_MAX,
      "shape": {"min_width": 32, "pad": [1, 2, 1, 2]},
      "text_size": 1
    })])
  end
  unless o["on_preview"].nil?
    eu_tools = eu_tools.concat([spacer, control({
      "key": (o["key"] ?? "mdedit") + ":eye",
      "tone": "ghost",
      "size": "sm",
      "selected": doc["preview"] == true,
      "shape": {"min_width": 32, "pad": [1, 2, 1, 2]},
      "a11y": {"role": "button", "label": doc["preview"] == true ? "Back to writing" : "Preview"},
      "on": {"click": o["on_preview"]},
      "c": [text(doc["preview"] == true ? "Write" : "Preview", {"size": 1, "weight": "semibold"})]
    })])
  end
  toolbar(eu_tools)
end

# What the picker takes by default: the pictures a client can decode (03
# §1 — PNG, JPEG and WebP, and nothing else draws) and the documents an
# attachment usually is.
MD_EDIT_ACCEPT = "png,jpg,jpeg,webp,pdf,txt,md,csv,zip"

# And how big one may be. Four megabytes rather than the client's own 16
# (`DEFAULT_UPLOAD_BYTES`), because keeping a file here means `slurp`ing it
# into an array of one boxed integer a byte before `eui_asset` takes it —
# a sixteen-megabyte photograph is several hundred megabytes for the length
# of that call. An application with somewhere better to put the bytes
# passes its own `max` and does not use `md_edit_attach`.
MD_EDIT_MAX = 4194304

# The editor.
#
# `doc` is the caller's document and the caller's to keep:
#
#   blocks    the list `md_edit_parse` makes
#   focus     the id of the block the caret is in, 0 for none
#   take      ask for the caret on this batch and no other
#   over      a file is over the drop zone, from `file_drag`
#   preview   the document is being read rather than written
#   trouble   what went wrong with the last upload, "" for nothing
#
# and `o` names the measure and the handlers:
#
#   width     the measure, in pixels. Derive it from the viewport; a fixed
#             one is what reads as unfinished on someone else's screen.
#   on_change / on_submit / on_key / on_focus
#             one handler each, for every block. Which block fired is
#             `params["props"]["block"]` (03 §4) — that is what makes one
#             handler serve a document of any length.
#   on_tool   a toolbar button; which one is `params["props"]["tool"]`
#   on_pick   the **server** handler for `file_pick`. Without it, and
#             without the `fs.pick` capability, the dialog opens for nobody
#             and says nothing (08 §3) — so the attach button is left out
#             rather than drawn dead.
#   on_drag   `file_drag`, so the drop zone can light as a file crosses it
#   on_drop   the × on a picture, a file or a rule
#   on_preview   the Write/Preview toggle; omit it for an editor with none
#   bar       false leaves the toolbar out, for a caller drawing
#             `markdown_editor_bar` above the scroller instead — which is
#             the only way a bar stays put while a long document moves
#   em_font   a font role for `*emphasis*`, from `eui_font(name, paths)`.
#             EUI has no italic at all — the style record has a family, a
#             size and a weight and no slant (02 §3) — so a real italic is
#             a face an application ships, and without one a mark is drawn
#             a weight heavier.
#   accept / max / files   as `file_field`'s
#   key       unique, as every control's is
#   placeholder   what an empty document says
def markdown_editor(doc, o = {})
  ew_wide = o["width"] ?? 640
  # The editor's own edge, and the room inside it.
  #
  # A block is a field with no border and no ground -- which is right for
  # the block, and leaves the editor itself with nothing to show for
  # itself: a column of bare text on the page's own colour, ending
  # wherever the last paragraph happens to end. Somebody writing a letter
  # could not see where the letter was.
  #
  # The frame stays exactly the width it was given, because callers derive
  # that from the viewport and lay other things out to match, so the
  # padding comes out of the inside: everything below is measured against
  # `ew_inner`, not `ew_wide`. Too narrow to take the inset and it keeps
  # the whole width and goes without, which is better than a document two
  # words wide.
  ew_inset = ew_wide > 160 ? 3 : 0
  ew_inner = ew_wide > 160 ? ew_wide - 16 : ew_wide
  ew_blocks = doc["blocks"] ?? []
  ew_focus = doc["focus"] ?? 0
  ew_take = doc["take"] == true
  # The two pieces of the document a block needs and a caller never sets:
  # which block has the `/` panel open, and where the arrows have walked in
  # it. Folded into `o` once rather than threaded through six signatures.
  o = o.merge({"slash": doc["slash"] ?? 0, "at": doc["at"] ?? 0})
  ew_kind = (md_edit_at(ew_blocks, ew_focus)["kind"] ?? "").to_s
  ew_body = []
  if doc["preview"] == true
    ew_body = [markdown(md_edit_source(ew_blocks), {"width": ew_inner, "gap": 3, "em_font": o["em_font"] ?? ""})]
  else
    ew_n = 0
    ew_at = 0
    while ew_at < ew_blocks.length()
      ew_n = (ew_blocks[ew_at]["kind"] ?? "p").to_s == "number" ? ew_n + 1 : 0
      ew_body = ew_body.concat([md_edit_block(ew_blocks[ew_at], o, ew_focus, ew_take, ew_inner, ew_n)])
      ew_at = ew_at + 1
    end
  end

  ew_said = (o["placeholder"] ?? "").to_s
  ew_empty = ew_blocks.length() == 1 && (ew_blocks[0]["t"] ?? "").to_s == ""
  # One drop zone for the whole document, not one per block: a `drop`
  # reports the slot it landed in (06 §6), which is exactly the number
  # `md_edit_move_to` wants, and a zone per block would be a hundred
  # subscriptions where one does. A caller that named no `on_move` gets an
  # ordinary column and no target at all, because a `drop_zone` with
  # nothing listening is not one.
  ew_page = column({"gap": 1, "width": ew_inner}, ew_body)
  unless o["on_move"].nil?
    ew_page = drop_zone(md_edit_group(o), {"display": "column", "gap": 1, "width": ew_inner},
                        (o["on_over"] ?? "").to_s, o["on_move"], ew_body)
  end
  # The placeholder goes *behind* the first block rather than under the
  # editor, because a document with nothing in it has nothing to look at:
  # a block is a field with no border and no ground, which is right once
  # there are words in it and invisible before there are. The field is
  # drawn over this and its ground covers it the moment it is focused,
  # which is the moment it has stopped being needed.
  if ew_empty && ew_said != "" && doc["preview"] != true
    ew_page = stack({"width": ew_inner}, [
      {
        "k": "box",
        "s": {"display": "row", "align": "center", "height": 36, "width": ew_inner, "pad": [0, 2, 0, 2]},
        "c": [text(ew_said, {"size": 2, "fg": "text.muted", "shrink": 1})]
      },
      ew_page
    ])
  end
  # `bar: false` for a caller drawing `markdown_editor_bar` itself, above
  # the scroller, so that it does not go up with the document.
  ew_rows = o["bar"] == false ? [ew_page] : [md_edit_bar(doc, o, ew_kind), ew_page]
  unless o["files"] == false || o["on_pick"].nil? || doc["preview"] == true
    ew_rows = ew_rows.concat([file_drop("Drop a picture or a file here", o["accept"] ?? MD_EDIT_ACCEPT, o["on_pick"], {
      "key": (o["key"] ?? "mdedit") + ":drop",
      "over": doc["over"] == true,
      "on_drag": o["on_drag"],
      "max": o["max"] ?? MD_EDIT_MAX,
      "hint": "or click to choose one"
    })])
  end
  ew_trouble = (doc["trouble"] ?? "").to_s
  ew_rows = ew_rows.concat([text(ew_trouble, {"size": 1, "fg": "danger.base"})]) unless ew_trouble == ""
  ew_rows = ew_rows.concat(md_edit_ask(doc, o))
  ew_frame = {
    "k": "box",
    "key": (o["key"] ?? "mdedit") + ":frame",
    "s": {
      "display": "column",
      "gap": 3,
      "width": ew_wide,
      "pad": [ew_inset, ew_inset, ew_inset, ew_inset],
      "border": 1,
      "border_color": "border.default",
      "radius": 2
    },
    "c": ew_rows
  }
  # Undo is claimed here as well as on every block, because focus is not
  # always in one: a tick on a task, a press on Add a row, and the caret is
  # on a control that hears nothing. Dispatch walks up from whatever has
  # focus to the nearest handler (06 §2), so a block answers its own keys
  # and everything else in the editor reaches this one.
  unless o["on_key"].nil?
    ew_frame["p"] = {"keys": ["z", "y", "Z"]}
    ew_frame["on"] = {"key_down": o["on_key"]}
  end
  ew_frame
end

# Keeping an uploaded file, the simple way: the bytes go in the asset store
# and the block names them `eui-asset:<hex>`.
#
# **An `eui-asset:` address means something to a client talking to this
# server and to nothing else.** An asset is addressed by its content and
# served from `/_eui/asset/<hex>` (01 §5); it is not a URL on the web, and a
# document that leaves the application — a mail that is sent, a page that is
# published — has to turn its assets into whatever that destination
# understands. This is the default because it is the one thing that works
# with no infrastructure at all, not because it is the right answer
# everywhere.
#
# An application with somewhere better to put the bytes — an uploader,
# SoliDB, a directory under `public/` — does not call this. It handles
# `file_upload` itself and calls `md_edit_put` with the block it built,
# which is the whole of what this does.
def md_edit_attach(doc, payload)
  ea_name = (payload["name"] ?? "").to_s
  ea_why = (payload["error"] ?? "").to_s
  return doc.merge({"trouble": ea_name + " did not arrive: " + ea_why}) unless ea_why == ""

  ea_path = (payload["path"] ?? "").to_s
  ea_bytes = slurp(ea_path, "binary") rescue null
  return doc.merge({"trouble": ea_name + ": the upload was gone before it could be kept"}) if ea_bytes.nil?

  ea_asset = eui_asset(ea_bytes) rescue null
  return doc.merge({"trouble": ea_name + ": the bytes could not be kept"}) if ea_asset.nil?

  ea_src = "eui-asset:" + (ea_asset["asset"] ?? "").to_s
  # A picture is measured here and nowhere else. The client fetches the
  # bytes by hash and never says how big they were; the server has the file
  # open at this moment and will not again -- and a picture whose size is
  # not written down gets a modest box and sits in it (`md_fit`).
  ea_shot = Image.new(ea_path) rescue null
  ea_one = {
    "id": md_edit_fresh(doc["blocks"] ?? []), "kind": "file",
    "t": ea_name, "src": ea_src, "note": md_edit_weight(payload["size"] ?? 0)
  }
  unless ea_shot.nil?
    ea_one = {
      "id": ea_one["id"], "kind": "image", "t": ea_name, "src": ea_src,
      "w": ea_shot.width() rescue 0,
      "h": ea_shot.height() rescue 0
    }
  end
  doc.merge({
    "blocks": md_edit_put(doc["blocks"] ?? [], doc["focus"] ?? 0, ea_one),
    "focus": ea_one["id"],
    "take": true,
    "over": false,
    "trouble": "",
    "awaiting": null
  })
end

# Is this `file_upload` the one this editor asked for?
#
# `file_upload` is the **server's** own event and it names no node: it says
# which transfer landed, not which picker opened. An application with two
# pickers — and any application with this widget in it has at least two,
# because the editor's is never the only one — has to tell them apart, and
# the id in `file_pick` is the only thing that can: it is the same id the
# upload carries back.
#
# So the routing is one line at the top of the reducer, before whatever else
# claims `file_upload`.
def md_edit_wants?(doc, params)
  ez_want = doc["awaiting"]
  return false if ez_want.nil?

  ez_want == (params["payload"] ?? {})["upload"]
end

# A file's weight, for the note under its name.
def md_edit_weight(size)
  return str(int(size / 1048576)) + " MB" if size >= 1048576
  return str(int(size / 1024)) + " KB" if size >= 1024

  str(size) + " B"
end

# One handler for the whole editor.
#
# `what` is what happened, not what the caller called its event: the caller
# maps its own event names onto these, so an application may name them
# anything and still route them here in one line.
#
#   "change"   a block's value settled (06 §2)
#   "submit"   Enter at the end of a block
#   "key"      a key the block asked for: Backspace at 0, or an arrow
#   "focus"    the caret arrived in a block
#   "tool"     a toolbar button
#   "upload"   the server's own `file_upload` — see `md_edit_attach`
#   "drag"     `file_drag`, whose payload is `[over]`
#   "drop"     the × on a picture, a file or a rule
#   "preview"  the Write/Preview toggle
def markdown_editor_step(doc, what, params)
  es_blocks = doc["blocks"] ?? []
  es_props = params["props"] ?? {}
  es_id = es_props["block"] ?? (doc["focus"] ?? 0)
  es_next = md_edit_fresh(es_blocks)

  # The ones that change nothing about the document.
  return doc.merge({"preview": doc["preview"] != true}) if what == "preview"
  return doc.merge({"over": (params["payload"] ?? [false])[0] == true}) if what == "drag"
  return doc.merge({"trouble": "", "awaiting": (params["payload"] ?? [0])[0]}) if what == "pick"
  return doc.merge({"focus": es_id, "take": false, "slash": 0}) if what == "focus"
  # `drag_over` is reported so a target can show it would take what is being
  # carried. Nothing here shows anything yet, and the handler exists because
  # a `drop_zone` with nothing listening is not a target at all.
  return doc if what == "over"
  return doc.merge({"asking": (doc["asking"] ?? {}).merge({"url": (params["payload"] ?? "").to_s})}) if what == "ask"

  # Everything below changes the blocks, so the blocks as they stand go on
  # the stack first. `md_edit_remember` is what decides whether that is a
  # new step or the same one continued.
  return md_edit_step_upload(doc, params) if what == "upload"
  return md_edit_step_key(doc, params, es_id, es_next) if what == "key"
  return md_edit_step_slash(doc, params, es_props, es_id) if what == "slash"
  return md_edit_step_link(doc, params, es_props) if what == "link"

  es_was = md_edit_remember(doc, what, es_id)
  if what == "check"
    return es_was.merge({"blocks": md_edit_check(es_blocks, es_id), "take": false})
  end

  if what == "cell"
    es_cell = md_edit_cell(es_blocks, es_id, es_props["row"] ?? 0, es_props["col"] ?? 0, (params["payload"] ?? "").to_s)
    return es_was.merge({"blocks": es_cell, "take": false})
  end

  if what == "grid"
    es_grown = (es_props["what"] ?? "").to_s == "col" ? md_edit_col_add(es_blocks, es_id) : md_edit_row_add(es_blocks, es_id)
    return es_was.merge({"blocks": es_grown, "take": false})
  end

  if what == "move"
    es_slot = (params["payload"] ?? [])[2] ?? -1
    return doc if es_slot < 0

    return es_was.merge({"blocks": md_edit_move_to(es_blocks, es_id, es_slot), "take": true})
  end

  if what == "drop"
    es_back = md_edit_step(es_blocks, es_id, -1)
    return es_was.merge({"blocks": md_edit_drop(es_blocks, es_id), "focus": es_back, "take": true})
  end

  if what == "change"
    return md_edit_step_change(es_was, params, es_id, es_next)
  end

  if what == "submit"
    # While the `/` panel is up, `Enter` takes what is highlighted in it
    # rather than starting a block: the panel is what the person is looking
    # at, and a key means what the thing in front of you does with it.
    return md_edit_step_pick(es_was, es_id) if (doc["slash"] ?? 0) == es_id

    es_made = md_edit_split_focus(es_blocks, es_id, es_next)
    return es_was.merge({"blocks": md_edit_split(es_blocks, es_id, es_next), "focus": es_made, "take": true})
  end

  if what == "tool"
    return md_edit_step_tool(es_was, (es_props["tool"] ?? "").to_s, es_id, es_next)
  end

  doc
end

# `change`, and the one place `/` is noticed: a block whose whole value is a
# slash and a query is a block asking for the panel, and it goes on being an
# ordinary block until one is picked.
def md_edit_step_change(doc, params, id, next_id)
  ec2_said = (params["payload"] ?? "").to_s
  # Only a block the panel could act on. A picture's caption and a file's
  # name are fields too, and a slash typed into one of those is a slash.
  ec2_kind = (md_edit_at(doc["blocks"] ?? [], id)["kind"] ?? "p").to_s
  ec2_open = md_edit_text?(ec2_kind) && md_starts(ec2_said, "/") && md_edit_slash_hits(ec2_said).length() > 0
  doc.merge({
    "blocks": md_edit_set(doc["blocks"] ?? [], id, ec2_said, next_id),
    "take": false,
    "slash": ec2_open ? id : 0,
    "at": ec2_open ? 0 : 0
  })
end

# A key the block asked for (03 §3.1). Which ones reach here at all is the
# whole mechanism: `Backspace` only with nothing before the caret, the
# arrows only where a single-line field had no use for them, and `z` always
# — because a printable character cannot be withheld, which is why the
# modifiers are checked and not assumed.
def md_edit_step_key(doc, params, id, next_id)
  ek2_said = params["payload"] ?? [""]
  ek2_key = ek2_said[0].to_s
  ek2_mods = ek2_said.length() > 1 ? ek2_said[1] : 0
  ek2_blocks = doc["blocks"] ?? []
  ek2_open = (doc["slash"] ?? 0) == id

  if md_edit_accel?(ek2_mods)
    return md_edit_redo(doc) if ek2_key == "y" || ek2_key == "Z"
    return md_edit_undo(doc) if ek2_key == "z"

    return doc
  end
  # Undo is the only thing a bare letter is wanted for, so every other one
  # goes back where it came from.
  return doc if ek2_key == "z" || ek2_key == "y" || ek2_key == "Z"
  return doc.merge({"slash": 0}) if ek2_key == "Escape"

  if ek2_open
    ek2_hits = md_edit_slash_hits((md_edit_at(ek2_blocks, id)["t"] ?? "").to_s)
    return doc.merge({"at": md_edit_walk(doc["at"] ?? 0, ek2_hits.length(), -1)}) if ek2_key == "ArrowUp"
    return doc.merge({"at": md_edit_walk(doc["at"] ?? 0, ek2_hits.length(), 1)}) if ek2_key == "ArrowDown"
  end

  # Alt and an arrow moves the block instead of the caret.
  if ek2_mods == 4 && (ek2_key == "ArrowUp" || ek2_key == "ArrowDown")
    ek2_by = ek2_key == "ArrowUp" ? -1 : 1
    return md_edit_remember(doc, "key", id).merge({
      "blocks": md_edit_shift(ek2_blocks, id, ek2_by), "take": true
    })
  end

  if ek2_key == "Backspace"
    ek2_joined = md_edit_merge_focus(ek2_blocks, id)
    return md_edit_remember(doc, "key", id).merge({
      "blocks": md_edit_merge(ek2_blocks, id), "focus": ek2_joined, "take": true
    })
  end
  return doc.merge({"focus": md_edit_step(ek2_blocks, id, -1), "take": true}) if ek2_key == "ArrowUp"
  return doc.merge({"focus": md_edit_step(ek2_blocks, id, 1), "take": true}) if ek2_key == "ArrowDown"

  doc
end

# Where the highlight lands, wrapping at both ends, which is what a short
# panel wants and what `tag_highlight` already does for the other one.
def md_edit_walk(at, count, delta)
  return 0 if count <= 0

  ew2_to = at + delta
  return count - 1 if ew2_to < 0
  return 0 if ew2_to >= count

  ew2_to
end

# A row of the `/` panel, clicked or taken with `Enter`. No tool named is
# the panel being dismissed — a `blur`, which is what a press outside it
# reports (03 §2).
def md_edit_step_slash(doc, params, props, id)
  es4_tool = (props["tool"] ?? "").to_s
  return doc.merge({"slash": 0}) if es4_tool == ""

  md_edit_step_tool(md_edit_remember(doc.merge({"slash": 0}), "slash", id), es4_tool, id, md_edit_fresh(doc["blocks"] ?? []))
end

# `Enter` in the panel: whatever the arrows have walked to.
def md_edit_step_pick(doc, id)
  ep2_hits = md_edit_slash_hits((md_edit_at(doc["blocks"] ?? [], id)["t"] ?? "").to_s)
  return doc.merge({"slash": 0}) if ep2_hits.length() == 0

  ep2_at = doc["at"] ?? 0
  ep2_at = 0 if ep2_at < 0 || ep2_at >= ep2_hits.length()
  md_edit_step_tool(doc.merge({"slash": 0}), ep2_hits[ep2_at]["tool"].to_s, id, md_edit_fresh(doc["blocks"] ?? []))
end

# Take the `/query` back out of a block the panel was opened from.
def md_edit_unslash(blocks, id)
  eu2_one = md_edit_at(blocks, id)
  return blocks if eu2_one["id"].nil?
  return blocks unless md_starts((eu2_one["t"] ?? "").to_s, "/")

  md_edit_splice(blocks, md_edit_index(blocks, id), 1, [eu2_one.merge({"t": ""})])
end

# One toolbar button, whether it was pressed on the bar or picked out of the
# panel. A tool that arrived through the panel clears the `/` and the query
# with it: they were the gesture, not the text.
def md_edit_step_tool(doc, tool, id, next_id)
  et2_blocks = doc["blocks"] ?? []
  et2_blocks = md_edit_unslash(et2_blocks, id)
  return doc.merge({"blocks": md_edit_wrap(et2_blocks, id, "**"), "take": true}) if tool == "strong"
  return doc.merge({"blocks": md_edit_wrap(et2_blocks, id, "*"), "take": true}) if tool == "em"
  if tool == "rule"
    return doc.merge({"blocks": md_edit_put(et2_blocks, id, {"id": next_id, "kind": "rule", "t": ""}), "take": false})
  end
  if tool == "table"
    return doc.merge({"blocks": md_edit_put(et2_blocks, id, md_edit_table(next_id)), "focus": next_id, "take": false})
  end
  if tool == "link"
    return doc.merge({"blocks": et2_blocks, "asking": {"block": id, "url": ""}})
  end

  doc.merge({"blocks": md_edit_kind(et2_blocks, id, tool), "take": true})
end


# The dialog's two buttons. `go` writes the link round the block's text and
# anything else simply shuts it.
def md_edit_step_link(doc, params, props)
  el2_ask = doc["asking"] ?? {}
  el2_id = el2_ask["block"] ?? 0
  return doc.merge({"asking": {}}) unless props["go"] == true

  el2_url = (el2_ask["url"] ?? "").to_s.strip()
  return doc.merge({"asking": {}}) if el2_url == "" || el2_id == 0

  md_edit_remember(doc, "link", el2_id).merge({
    "blocks": md_edit_pair_wrap(doc["blocks"] ?? [], el2_id, el2_url),
    "asking": {},
    "take": true
  })
end

# `[the block](the address)`, and off again when it is already that.
def md_edit_pair_wrap(blocks, id, url)
  ew3_at = md_edit_index(blocks, id)
  return blocks if ew3_at < 0

  ew3_one = blocks[ew3_at]
  return blocks unless md_edit_text?((ew3_one["kind"] ?? "p").to_s)

  ew3_said = (ew3_one["t"] ?? "").to_s
  return blocks if ew3_said == ""

  md_edit_splice(blocks, ew3_at, 1, [ew3_one.merge({"t": "[" + ew3_said + "](" + url + ")"})])
end

# The bytes of an upload, kept the simple way. Its own function because the
# step above is long enough.
def md_edit_step_upload(doc, params)
  md_edit_attach(md_edit_remember(doc, "upload", doc["focus"] ?? 0), params["payload"] ?? {})
end

# Where the caret goes after `Enter`: into the block that was just made, or
# back into the one that just left a list, since no new block was made then.
def md_edit_split_focus(blocks, id, next_id)
  ek_one = md_edit_at(blocks, id)
  ek_kind = (ek_one["kind"] ?? "p").to_s
  ek_list = ek_kind == "bullet" || ek_kind == "number"
  return id if ek_list && (ek_one["t"] ?? "").to_s == ""

  next_id
end

# And after `Backspace` at the head of a block: the block itself while it is
# only losing its marker, and otherwise the one above, which is where the
# text it carried has gone. Asked **before** the merge, because after it the
# block above may be the one that went.
def md_edit_merge_focus(blocks, id)
  em_one = md_edit_at(blocks, id)
  em_kind = (em_one["kind"] ?? "p").to_s
  return id unless md_edit_text?(em_kind)
  return id if em_kind != "p"

  em_before = md_edit_step(blocks, id, -1)
  em_above = md_edit_at(blocks, em_before)
  # A picture above is removed rather than merged into, and the caret stays
  # where it was.
  return id unless md_edit_text?((em_above["kind"] ?? "p").to_s)

  em_before
end
