import "./species.sl"

export def species_colour(kind)
  match kind {
    "radish" => "danger.base",
    "sunflower" => "warning.base",
    "strawberry" => "success.base",
    _ => "accent.base"
  }
end

# The chart's width: the window's, less its padding and the card's, up to 880.
export def chart_width(state)
  width = (state["width"] ?? 1100) - 88
  width > 880 ? 880 : width
end

# One bar per species, in four equal slots, to scale: the tallest bar is the
# most harvested species. The legend under it uses the same four slots.
export def harvest_chart(harvest, width)
  counts = species_order().map { |kind| harvest[kind] }
  top = counts.reduce(fn(best, c) c > best ? c : best, 1)
  slot = width / 4
  bar = slot * 5 / 11
  bars = range(0, 4).map { |i|
    height = 120 * counts[i] / top
    [1, species_colour(species_order()[i]), i * slot + (slot - bar) / 2, 136 - height, bar, height, 6]
  }
  canvas(width, 140, [[0, "border.subtle", 1, 0, 136, width, 136]] + bars)
end

export def legend_cell(state, kind)
  column({"grow": 1, "basis": 0, "align": "center", "gap": 1}, [
    text(str(state["harvest"][kind]), {"size": 5, "weight": "semibold", "fg": species_colour(kind)}),
    muted(species()[kind]["name"])
  ])
end

export def herbarium_screen(state)
  total = species_order().map { |kind| state["harvest"][kind] }.reduce(fn(sum, c) sum + c, 0)
  if total == 0
    return empty_state(
      "Your herbarium is empty",
      "Harvest a ripe plant and it is pressed here.",
      "Back to the garden",
      "go_garden"
    )
  end

  width = chart_width(state)
  card({"gap": 4}, [
    text("#{total} plants pressed so far", {"size": 3, "weight": "semibold"}),
    harvest_chart(state["harvest"], width),
    row({"width": width}, species_order().map { |kind| legend_cell(state, kind) })
  ])
end
