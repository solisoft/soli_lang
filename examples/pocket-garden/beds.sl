import "./species.sl"

# The rules of the garden. Pure functions of the state: each takes the state
# and gives back the next one, so the handler stays a list of verbs.
export class Garden
  static def fresh
    {
      "tab": "Garden",
      "coins": 6,
      "tick": 0,
      "day": 1,
      "pick": "radish",
      "seeds": {"radish": 2, "sunflower": 1, "strawberry": 0, "lavender": 0},
      "harvest": {"radish": 0, "sunflower": 0, "strawberry": 0, "lavender": 0},
      "beds": range(0, 4).map { |i| Garden.empty_bed },
      "news": "Pick a seed, then plant it in an empty bed."
    }
  end

  static def empty_bed
    {"kind": "", "age": 0, "water": 0}
  end

  static def ripe?(bed)
    return false if bed["kind"].blank?

    bed["age"] >= species()[bed["kind"]]["grows"]
  end

  static def growing?(state)
    state["beds"].any? { |bed| bed["kind"].present? && !Garden.ripe?(bed) }
  end

  # One second of garden time. Every eighth tick is a new day, and every
  # third day it rains, which waters every bed at once.
  static def tick(state)
    tick = state["tick"] + 1
    day = state["day"]
    news = state["news"]
    rain = false
    if tick % 8 == 0
      day = day + 1
      rain = day % 3 == 0
      news = rain ? "Day #{day}: rain! Every bed is watered." : "Day #{day} begins."
    end
    beds = state["beds"].map { |bed| Garden.grow(bed, rain) }
    state.merge({"tick": tick, "day": day, "beds": beds, "news": news})
  end

  static def grow(bed, rain)
    return bed if bed["kind"].blank? || Garden.ripe?(bed)

    water = rain ? 5 : bed["water"]
    return bed.merge({"water": water}) if water == 0

    bed.merge({"age": bed["age"] + 1, "water": water - 1})
  end

  static def plant(state, n)
    kind = state["pick"]
    name = species()[kind]["name"]
    if state["seeds"][kind] == 0
      return state.merge({"news": "No #{name} seeds left: visit the shop."})
    end

    beds = Garden.replace(state["beds"], n, {"kind": kind, "age": 0, "water": 3})
    seeds = Garden.bump(state["seeds"], kind, -1)
    state.merge({"beds": beds, "seeds": seeds, "news": "A #{name} goes into bed #{n + 1}."})
  end

  static def water(state, n)
    bed = state["beds"][n].merge({"water": 5})
    state.merge({"beds": Garden.replace(state["beds"], n, bed), "news": "Bed #{n + 1} is watered."})
  end

  static def harvest(state, n)
    bed = state["beds"][n]
    return state unless Garden.ripe?(bed)

    info = species()[bed["kind"]]
    state.merge({
      "coins": state["coins"] + info["sells"],
      "harvest": Garden.bump(state["harvest"], bed["kind"], 1),
      "beds": Garden.replace(state["beds"], n, Garden.empty_bed),
      "news": "#{info["name"]} harvested: +#{info["sells"]} coins."
    })
  end

  static def buy(state, kind)
    info = species()[kind]
    if state["coins"] < info["price"]
      return state.merge({"news": "#{info["name"]} seeds cost #{info["price"]} coins."})
    end

    state.merge({
      "coins": state["coins"] - info["price"],
      "seeds": Garden.bump(state["seeds"], kind, 1),
      "news": "One #{info["name"]} seed bought."
    })
  end

  static def replace(beds, n, bed)
    range(0, beds.length).map { |i| i == n ? bed : beds[i] }
  end

  static def bump(counts, key, delta)
    copy = counts.merge({})
    copy[key] = (copy[key] ?? 0) + delta
    copy
  end
end

# ---- drawing --------------------------------------------------------------

# Eight petals around a centre, without trigonometry.
export def petal_offsets
  [[14, 0], [10, 10], [0, 14], [-10, 10], [-14, 0], [-10, -10], [0, -14], [10, -10]]
end

# One bed as canvas paths, in a 180 × 150 box whose soil line is y = 118:
# the soil, then the plant at its stage. `frac` runs from 0 (just sown) to
# 1 (ripe); a thirsty plant is drawn grey. Colours are theme roles, so the
# garden is right in dark mode too.
export def plant_paths(kind, frac, thirsty)
  soil = [[1, "warning.subtle", 0, 118, 180, 32, 8]]
  return soil if kind.blank?

  leaf = thirsty ? "text.muted" : "success.base"
  return soil + [[3, leaf, 90, 113, 5]] if frac < 0.15

  ripe = frac >= 1.0
  plant = match kind {
    "radish" => radish_paths(frac, leaf, ripe),
    "sunflower" => sunflower_paths(frac, leaf, ripe),
    "strawberry" => strawberry_paths(frac, leaf, ripe),
    _ => lavender_paths(frac, leaf, ripe)
  }
  soil + plant
end

export def radish_paths(frac, leaf, ripe)
  top = 118 - 44 * frac
  paths = [[0, leaf, 3, 90, 118, 90, top], [3, leaf, 78, top, 11], [3, leaf, 102, top + 5, 10]]
  return paths unless ripe

  paths + [[3, "danger.base", 90, 124, 14], [0, "danger.base", 2, 90, 136, 90, 146]]
end

export def sunflower_paths(frac, leaf, ripe)
  top = 118 - 92 * frac
  paths = [
    [0, leaf, 4, 90, 118, 90, top],
    [3, leaf, 76, 118 - 36 * frac, 9],
    [3, leaf, 104, 118 - 56 * frac, 9]
  ]
  return paths + [[3, "warning.base", 90, top, 6]] unless ripe

  petals = petal_offsets().map { |p| [3, "warning.base", 90 + p[0], top + p[1], 8] }
  paths + petals + [[3, "text.default", 90, top, 9]]
end

export def strawberry_paths(frac, leaf, ripe)
  r = 10 + 12 * frac
  paths = [[3, leaf, 70, 110, r], [3, leaf, 110, 110, r], [3, leaf, 90, 98, r]]
  return paths unless ripe

  berries = [[72, 118], [94, 121], [112, 116]].map { |b| [3, "danger.base", b[0], b[1], 6] }
  paths + berries
end

export def lavender_paths(frac, leaf, ripe)
  heights = [[68, 80], [90, 100], [112, 86]]
  stems = heights.map { |s| [0, leaf, 2, s[0], 118, s[0], 118 - s[1] * frac] }
  return stems unless ripe

  spikes = heights.map { |s| [0, "accent.base", 7, s[0], 118 - s[1] + 26, s[0], 118 - s[1]] }
  stems + spikes
end

# ---- the garden screen ----------------------------------------------------

export def with_props(node, props)
  node["p"] = (node["p"] ?? {}).merge(props)
  node
end

export def seed_chip(state, kind)
  label = "#{species()[kind]["name"]} ×#{state["seeds"][kind]}"
  chip = kind == state["pick"] ? button(label, "pick") : secondary_button(label, "pick")
  with_props(chip, {"kind": kind})
end

export def bed_card(state, n)
  bed = state["beds"][n]
  kind = bed["kind"]
  picture = canvas(180, 150, plant_paths(kind, 0.0, false))
  if kind.blank?
    picked = species()[state["pick"]]["name"]
    return card({"grow": 1, "basis": 0, "gap": 3, "align": "center"}, [
      picture,
      muted("Bed #{n + 1} is empty"),
      with_props(secondary_button("Plant #{picked}", "plant"), {"bed": n})
    ])
  end

  info = species()[kind]
  frac = Garden.ripe?(bed) ? 1.0 : bed["age"] / (info["grows"] * 1.0)
  thirsty = bed["water"] == 0 && !Garden.ripe?(bed)
  picture = canvas(180, 150, plant_paths(kind, frac, thirsty))
  action = secondary_button("Water", "water")
  action = button("Harvest +#{info["sells"]}", "harvest") if Garden.ripe?(bed)
  water_note = muted("Water #{bed["water"]}/5")
  water_note = text("Thirsty!", {"size": 1, "weight": "semibold", "fg": "danger.base"}) if thirsty
  card({"grow": 1, "basis": 0, "gap": 3, "align": "center"}, [
    picture,
    text(info["name"], {"size": 2, "weight": "semibold"}),
    progress(frac),
    water_note,
    with_props(action, {"bed": n})
  ])
end

export def garden_screen(state)
  chips = species_order().map { |kind| seed_chip(state, kind) }
  column({"gap": 5}, [
    row({"gap": 2, "align": "center"}, [text("Seeds", {"size": 1, "weight": "semibold"})] + chips),
    row({"gap": 4}, range(0, 4).map { |n| bed_card(state, n) })
  ])
end
