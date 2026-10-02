import "./species.sl"

export def species_colour(kind)
  match kind {
    "radish" => "danger.base",
    "sunflower" => "warning.base",
    "strawberry" => "success.base",
    _ => "accent.base"
  }
end

# One bar per species, in four equal slots, to scale: the tallest bar is the
# most harvested species. The legend under it uses the same four slots.
export def harvest_chart(harvest)
  counts = species_order().map { |kind| harvest[kind] }
  top = counts.reduce(fn(best, c) c > best ? c : best, 1)
  bars = range(0, 4).map { |i|
    height = 120 * counts[i] / top
    [1, species_colour(species_order()[i]), i * 220 + 60, 136 - height, 100, height, 6]
  }
  canvas(880, 140, [[0, "border.subtle", 1, 0, 136, 880, 136]] + bars)
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

  card({"gap": 4}, [
    text("#{total} plants pressed so far", {"size": 3, "weight": "semibold"}),
    harvest_chart(state["harvest"]),
    row({"width": 880}, species_order().map { |kind| legend_cell(state, kind) })
  ])
end
