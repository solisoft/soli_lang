# EUI view builders, part 3: charts.
#
# Every mark the catalogue draws — line, area, bar in its four
# arrangements, candlestick, gantt, heatmap, dumbbell, sparkline — with
# the axes they share and the hit-testing that answers the pointer. All of
# it is one `canvas` node with a `paths` prop, and the path format is the
# first thing documented below.
#
# Part of the reference catalogue — `eui_builders.sl` has the header that
# explains the whole of it, and the primitives everything here calls.

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

# How many of them got to the end.
#
# A funnel is a ranked bar chart that has given up its baseline: the bars are
# centred, so the eye reads the narrowing rather than the lengths, which is
# the question a funnel is asked. The drop between two stages is put beside
# the lower one, because that is the number anybody is actually looking for
# and computing it in your head from two absolutes is work.
#
# A stage is `{"label", "value"}`, in the order they happen.
def chart_funnel(id, stages, w, h)
  count = stages.length()
  return muted("nothing to funnel") if count == 0

  gutter = w / 3
  gutter = 96 if gutter < 96
  gutter = 180 if gutter > 180
  plot_w = w - gutter - 8
  ph = chart_plot_h(h)
  row_h = (ph - 8) / count
  bar_h = row_h * 3 / 5
  bar_h = 8 if bar_h < 8
  top = chart_max(stages.map(fn(st) { st["value"] }))
  bars = range(0, count).map(fn(i) {
    bw = (plot_w - 8) * stages[i]["value"] / top
    bw = 3 if bw < 3
    [1, i == count - 1 ? "success.base" : "series.1", 4 + (plot_w - 8 - bw) / 2, 4 + i * row_h + (row_h - bar_h) / 2, bw, bar_h, 2]
  })
  drawing = canvas(plot_w, ph, bars)
  names = column({"gap": 0, "width": gutter}, range(0, count).map(fn(i) {
    st = stages[i]
    kept = i == 0 || stages[0]["value"] == 0 ? "" : str(st["value"] * 100 / stages[0]["value"]) + " %"
    {
      "k": "box",
      "s": {"display": "column", "gap": 0, "justify": "center", "align": "end", "height": row_h, "width": "100%"},
      "c": [
        text(st["label"], {"size": 0, "fg": "text.muted", "clamp": 1}),
        text(str(st["value"]) + (kept == "" ? "" : " · " + kept), {"size": 0, "weight": "semibold"})
      ]
    }
  }))
  heights = range(0, count).map(fn(i) { row_h })
  labels = range(0, count).map(fn(i) { stages[i]["label"].to_s + " · " + str(stages[i]["value"]) })
  row({"gap": 2, "width": w, "align": "start"}, [
    column({"gap": 0, "width": gutter}, [names, {"k": "box", "s": {"height": CHART_AXIS_H}}]),
    column({"gap": 0, "align": "center", "width": plot_w}, [
      chart_row_layers(id, heights, labels, plot_w, ph, drawing),
      chart_value_axis(0, top, plot_w, CHART_TICKS - 1)
    ])
  ])
end

# Where the money went between one total and the next.
#
# Every bar starts where the last one ended, so the chart is a bridge rather
# than a comparison: the gap between the opening and closing columns is the
# sum of everything drawn between them, and that is visible instead of
# asserted. A step marked `{"total": true}` is measured from zero -- an
# opening or closing balance stands on the floor, and only the moves float.
#
# A step is `{"label", "delta"}`, or `{"label", "delta", "total": true}`.
def chart_waterfall(id, steps, w, h)
  count = steps.length()
  return muted("nothing to bridge") if count == 0

  # Walk it once for the extent: a bridge is only readable if the axis holds
  # every intermediate level, not just the ends.
  running = 0
  levels = [0]
  tops = range(0, count).map(fn(i) {
    st = steps[i]
    # Not `from`/`to`: both are keywords.
    starts = st["total"] == true ? 0 : running
    ends = st["total"] == true ? st["delta"] : running + st["delta"]
    running = ends
    levels = levels.concat([ends])
    [starts, ends]
  })
  high = chart_max(levels)
  low = 0
  for lv in levels
    low = lv if lv < low
  end
  span = high - low
  span = 1 if span == 0
  ticks = chart_scale_ticks(low, high)
  gutter = chart_gutter(ticks)
  pw = w - gutter
  ph = chart_plot_h(h)
  slot = (pw - 8) / count
  bar_w = slot * 3 / 5
  bar_w = 4 if bar_w < 4
  y_of = fn(v) { 4 + (high - v) * (ph - 8) / span }
  bars = range(0, count).map(fn(i) {
    st = steps[i]
    a = y_of(tops[i][0])
    b = y_of(tops[i][1])
    y = a < b ? a : b
    bh = a < b ? b - a : a - b
    bh = 2 if bh < 2
    tone = st["total"] == true ? "accent.base" : (st["delta"] < 0 ? "danger.base" : "success.base")
    [1, tone, 4 + i * slot + (slot - bar_w) / 2, y, bar_w, bh, 1]
  })
  # The dotted rails that carry each level across to the next bar are what
  # make it read as one walk instead of six unrelated columns.
  rails = range(0, count - 1).map(fn(i) {
    y = y_of(tops[i][1])
    [0, "border.strong", 1, 4 + i * slot + (slot + bar_w) / 2, y, 4 + (i + 1) * slot + (slot - bar_w) / 2, y]
  })
  drawing = canvas(pw, ph, chart_grid(pw, ph).concat(rails).concat(bars))
  spans = range(0, count).map(fn(i) { slot })
  labels = range(0, count).map(fn(i) {
    steps[i]["label"].to_s + " · " + (steps[i]["total"] == true ? str(steps[i]["delta"]) : chart_signed(steps[i]["delta"]))
  })
  layers = chart_layers(id, spans, labels, pw, ph, drawing)
  chart_framed(ticks, chart_x_axis_bands(steps.map(fn(st) { st["label"] }), spans), pw, ph, gutter, layers)
end

# The shape of a distribution, five numbers at a time.
#
# An average hides everything that makes a number worth looking at: two
# warehouses can pick the same mean lines per hour and one of them can be
# stopping every afternoon. The box is the middle half, the line in it is the
# median -- not the mean, which a single bad day drags -- and the whiskers
# are the ends.
#
# A group is `{"label", "min", "q1", "median", "q3", "max"}`. This draws the
# summary; computing it from a sample is the application's business.
def chart_box(id, groups, w, h)
  count = groups.length()
  return muted("nothing to summarise") if count == 0

  lows = groups.map(fn(g) { g["min"] })
  highs = groups.map(fn(g) { g["max"] })
  high = chart_max(highs)
  low = lows[0]
  for v in lows
    low = v if v < low
  end
  ticks = chart_scale_ticks(low, high)
  gutter = chart_gutter(ticks)
  pw = w - gutter
  ph = chart_plot_h(h)
  span = high - low
  span = 1 if span == 0
  slot = (pw - 8) / count
  box_w = slot * 3 / 5
  box_w = 6 if box_w < 6
  y_of = fn(v) { 4 + (high - v) * (ph - 8) / span }
  paths = []
  for i in range(0, count)
    g = groups[i]
    cx = 4 + i * slot + slot / 2
    left = cx - box_w / 2
    y3 = y_of(g["q3"])
    y1 = y_of(g["q1"])
    bh = y1 - y3
    bh = 2 if bh < 2
    paths = paths.concat([
      [0, "border.strong", 1, cx, y_of(g["max"]), cx, y3],
      [0, "border.strong", 1, cx, y1, cx, y_of(g["min"])],
      [0, "border.strong", 2, cx - box_w / 4, y_of(g["max"]), cx + box_w / 4, y_of(g["max"])],
      [0, "border.strong", 2, cx - box_w / 4, y_of(g["min"]), cx + box_w / 4, y_of(g["min"])],
      [1, "series.1", left, y3, box_w, bh, 1],
      [0, "surface.base", 2, left, y_of(g["median"]), left + box_w, y_of(g["median"])]
    ])
  end
  drawing = canvas(pw, ph, chart_grid(pw, ph).concat(paths))
  spans = range(0, count).map(fn(i) { slot })
  labels = range(0, count).map(fn(i) {
    g = groups[i]
    g["label"].to_s + " · " + str(g["min"]) + "–" + str(g["max"]) + ", median " + str(g["median"])
  })
  layers = chart_layers(id, spans, labels, pw, ph, drawing)
  chart_framed(ticks, chart_x_axis_bands(groups.map(fn(g) { g["label"] }), spans), pw, ph, gutter, layers)
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
