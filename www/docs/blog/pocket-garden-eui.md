# Pocket Garden: a Little Game in a Native Window, from One Script

Most EUI examples are tools: a counter, a notes list, a dashboard. This one is a
game. **Pocket Garden** has four beds, a seed shop and a herbarium. You plant a
radish, water it, and watch it grow in real time; leave it too long without water
and it goes grey and stops. Every eighth second is a new day, and every third day
it rains on everything at once. Ripe plants sell for coins, coins buy better
seeds, and every harvest is pressed into the herbarium as a bar on a chart.

It is 363 lines of Soli in five files, and it runs from a plain script:
`soli garden.sl`, no `soli new`, no server to start. Every screenshot below is
that script, drawn by the real EUI client, and every line of code here is the
code that drew it.

<figure style="margin:1.5rem auto;max-width:1024px;">
  <img src="/images/blog/pocket-garden.svg" width="1024" height="576" alt="A native window titled Pocket Garden with four garden beds: a ripe radish with a red root, a sunflower with yellow petals, a strawberry bush with red berries and three lavender spikes. Beside it, a script file named garden.sl importing four other files, and a clock that ticks once a second while something grows." style="display:block;width:100%;height:auto;border-radius:12px;border:1px solid #30363d;background:#0b0d0f;">
  <figcaption style="text-align:center;color:#8b949e;font-size:0.875rem;margin-top:0.5rem;">Five files, one window, a clock that only ticks while something grows.</figcaption>
</figure>

On the way it covers what a bigger EUI app needs and the
[notes tutorial](/docs/blog/eui-notes-app) left out: splitting the code across
files, keeping the rules in a class, drawing with `canvas`, a clock with `wake`,
and several screens in one window.

## What you need

- **A Soli with `eui_window`**, the first release after 2.11.1.
- **A `soli` that can open a window.** The window is the `eui-desktop` feature,
  which the published binaries leave out. Build it from the repository:
  `cargo install --path . --locked --features eui-desktop`.

## The shape

```
garden.sl      # eui_window, the handler, the header and the tabs
species.sl     # the four plants: price, growing time, selling price
beds.sl        # the rules (a Garden class), the plant drawings, the garden screen
shop.sl        # the seed shop
herbarium.sl   # the harvest chart
```

`garden.sl` is the script you run. It imports the other four:

```soli
# Pocket Garden: `soli garden.sl` opens the window.
import "./species.sl"
import "./beds.sl"
import "./shop.sl"
import "./herbarium.sl"
```

## Step 1: a window from a script

`eui_window(name, handler, view, options)` takes the names of two `def`s of the
script. The handler turns an event into the next state; the view turns state into
a tree of nodes. Here is the handler, in full:

```soli
def pocket_garden(event_data)
  state = event_data["state"] || Garden.fresh
  props = (event_data["params"] ?? {})["props"] ?? {}
  match event_data["event"] {
    "tick" => Garden.tick(state),
    "tab" => state.merge({"tab": props["tab"]}),
    "go_garden" => state.merge({"tab": "Garden"}),
    "pick" => state.merge({"pick": props["kind"]}),
    "plant" => Garden.plant(state, props["bed"]),
    "water" => Garden.water(state, props["bed"]),
    "harvest" => Garden.harvest(state, props["bed"]),
    "buy" => Garden.buy(state, props["kind"]),
    _ => state
  }
end
```

Three details carry most of the weight.

**`event_data["state"] || Garden.fresh`** starts a new game. The first event is
`connect`, and it arrives with an empty state. In Soli an empty hash is falsy, so
`||` swaps it for a fresh garden; `??` would not, because `{}` is not `nil`.

**The handler is a list of verbs.** Each arm hands the state to a function and
returns what that function returns. The rules live elsewhere, so this stays the
one place you read to know what the game can do.

**`_ => state` keeps the game when nothing matches.** The runtime also sends
events you never asked for, such as `connect` and `viewport` when the window is
resized. Falling through to `nil` would throw the garden away on the first resize.

The view and the handler do not run where the script does. They run in the
window's server workers, which load the script's *definitions* (`def`, classes,
enums, constants) and nothing else. A top-level variable is invisible to them, so
everything the game knows is in the state or behind a function.

## Step 2: the rules, as a class

All the rules are static methods of one class, in `beds.sl`. Each takes the state
and returns the next one, and none of them knows about windows:

```soli
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
```

```soli
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
```

```soli
static def grow(bed, rain)
  return bed if bed["kind"].blank? || Garden.ripe?(bed)

  water = rain ? 5 : bed["water"]
  return bed.merge({"water": water}) if water == 0

  bed.merge({"age": bed["age"] + 1, "water": water - 1})
end
```

`tick` is one second of garden time. A planted bed with water grows by one and
drinks one; a bed at zero water stops, and the drawing turns grey. Rain is not
random: every third day, so the game behaves the same way every time you play it,
which is also what made these screenshots repeatable.

## Step 3: splitting the code into files

`import "./beds.sl"` brings in what the file exports. Two things to know:

- **Export what the other files call.** A function another file uses is written
  `export def`, and the class is `export class Garden`. Without `export`, the
  script refuses to start: `Undefined variable 'garden_screen'`.
- **There is no `export const`.** Shared data is a function instead, and that is
  no workaround: a function is a definition, so the window's workers can see it.

```soli
# Everything the game knows about a plant. `export const` does not exist, so
# shared data is a function: the window's workers load definitions, not
# variables, and a function is a definition.
export def species
  {
    "radish":     {"name": "Radish",     "price": 2,  "grows": 6,  "sells": 5},
    "sunflower":  {"name": "Sunflower",  "price": 5,  "grows": 12, "sells": 14},
    "strawberry": {"name": "Strawberry", "price": 8,  "grows": 18, "sells": 24},
    "lavender":   {"name": "Lavender",   "price": 12, "grows": 26, "sells": 40}
  }
end

export def species_order
  ["radish", "sunflower", "strawberry", "lavender"]
end
```

## Step 4: drawing plants with `canvas`

A `canvas` node draws paths. Each path is a list: a kind, a colour, then numbers
in pixels. There are five kinds: `0` polyline, `1` rounded rectangle, `2` area,
`3` filled circle, `4` arc. Each bed is a 180 × 150 canvas with the soil line at
y = 118, and `frac` says how grown the plant is, from 0 (just sown) to 1 (ripe):

```soli
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
```

```soli
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
```

```soli
# Eight petals around a centre, without trigonometry.
export def petal_offsets
  [[14, 0], [10, 10], [0, 14], [-10, 10], [-14, 0], [-10, -10], [0, -14], [10, -10]]
end
```

The colours are theme roles, never hex values: `success.base` for leaves,
`warning.base` for petals, `warning.subtle` for the soil. The client resolves each
against the viewer's palette, so the garden is right in dark mode without a line
of code about dark mode. And the eight petals need no trigonometry: they are eight
fixed offsets from the centre.

<figure style="margin:1.5rem auto;max-width:1024px;">
  <img src="/images/blog/pocket-garden-shop.png" width="1024" alt="The Pocket Garden shop in the dark theme: four cards, each with a ripe plant drawn on a canvas (radish, sunflower, strawberry, lavender), its growing time, its price, and a Buy button or the number of coins still needed." style="display:block;width:100%;height:auto;border-radius:12px;border:1px solid #30363d;background:#0b0d0f;">
  <figcaption style="text-align:center;color:#8b949e;font-size:0.875rem;margin-top:0.5rem;">The shop draws each plant ripe, with the same function the beds use. Dark theme, decided by the viewer.</figcaption>
</figure>

## Step 5: a clock that sleeps

Plants have to grow while you watch. An EUI node asks for time with a `wake` prop,
a period in milliseconds, plus a `wake` handler naming the event to send:

```soli
def pocket_garden_view(state)
  state = Garden.fresh unless state["beds"]
  screen = match state["tab"] {
    "Shop" => shop_screen(state),
    "Herbarium" => herbarium_screen(state),
    _ => garden_screen(state)
  }
  root = column({"pad": 7, "gap": 5, "bg": "surface.base", "height": "100%"}, [
    header(state),
    tabs(["Garden", "Shop", "Herbarium"], state["tab"], "tab"),
    screen
  ])
  # The clock: one `tick` a second, only while something is growing. An
  # empty or fully ripe garden asks for no wakeups at all.
  return root unless Garden.growing?(state)

  root["p"] = (root["p"] ?? {}).merge({"wake": 1000})
  root["on"] = (root["on"] ?? {}).merge({"wake": "tick"})
  root
end
```

The last four lines are the clock. While something grows, the root node asks for a
`tick` every second, and the handler passes it to `Garden.tick`. When nothing
grows, because the beds are empty or every plant is ripe, the view drops the prop,
and the window stops waking up entirely. An idle Pocket Garden costs nothing.

The client puts limits on it: a period under 100 ms is raised to 100 ms, at most
four nodes wake at once, and a window that was not drawn for a while gets one
`tick` when it is shown again, not all the ones it missed.

## Step 6: buttons that know their bed

Four beds share one handler, so each button has to say which bed it belongs to.
A node's `props` come back to the handler as `params["props"]`:

```soli
export def with_props(node, props)
  node["p"] = (node["p"] ?? {}).merge(props)
  node
end
```

```soli
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
```

Every bed's Water button sends the same event, `"water"`, and carries
`{"bed": n}`. The handler reads `props["bed"]` and waters that one. The same
trick picks a seed (`{"kind": kind}`) and buys one in the shop.

The screens themselves are just functions, picked with a `match` on
`state["tab"]` in the view. The `tabs` widget sends `"tab"` with the tab's name
in its props, so switching screens is one more verb in the handler.

<figure style="margin:1.5rem auto;max-width:1024px;">
  <img src="/images/blog/pocket-garden-garden.png" width="1024" alt="The Pocket Garden window on day 2: a ripe radish with a Harvest +5 button, a growing sunflower at sixty percent with one water left, a grey thirsty radish marked Thirsty!, and an empty fourth bed with a Plant Radish button." style="display:block;width:100%;height:auto;border-radius:12px;border:1px solid #30363d;background:#0b0d0f;">
  <figcaption style="text-align:center;color:#8b949e;font-size:0.875rem;margin-top:0.5rem;">Day 2: one radish ripe, the sunflower watered and growing, the other radish dry and grey.</figcaption>
</figure>

## Step 7: the herbarium

Each harvest adds one to a count per species, and the herbarium draws the counts
as bars, to scale, the tallest bar being the most harvested species:

```soli
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
```

Before the first harvest, the screen is an `empty_state` that says what to do and
offers a button back to the garden.

<figure style="margin:1.5rem auto;max-width:1024px;">
  <img src="/images/blog/pocket-garden-herbarium.png" width="1024" alt="The Pocket Garden herbarium after three harvests: a bar chart with a tall red bar for two radishes and a shorter amber bar for one sunflower, and the counts 2, 1, 0 and 0 under the four species." style="display:block;width:100%;height:auto;border-radius:12px;border:1px solid #30363d;background:#0b0d0f;">
  <figcaption style="text-align:center;color:#8b949e;font-size:0.875rem;margin-top:0.5rem;">Three harvests in: two radishes, one sunflower, thirty coins in the purse.</figcaption>
</figure>

## Run it

With a `soli` built with `eui-desktop`, the script opens its own window:

```bash
soli garden.sl
```

The window opens on the script's thread, and `eui_window` returns when you close
it. Without the feature, `soli` says so instead of opening anything.

For tests and screenshots there is a mode without a window:

```bash
$ SOLI_EUI_NO_WINDOW=1 soli garden.sl
ws://127.0.0.1:41871/_eui/session/pocket-garden
soli_desktop=5fc91513…
```

The session only answers on `127.0.0.1`, and only to a client that sends that
cookie. The screenshots in this post were taken that way, by EUI's `snapshot`
tool, which plays a scripted session (click *Plant Radish*, wait three seconds,
click *Water*…) and draws the result with the real renderer.

## Three things that tripped us up

- **`export const` does not exist.** Expose data with a function (Step 3).
- **A text colour is `"fg"`, not `"color"`.** An unknown style key ends the session
  with `unknown style key 'color' (error 400)`. EUI refuses typos loudly, on
  purpose.
- **Whole numbers divide whole.** `3 / 6` is `0`, so the growth fraction multiplies
  by `1.0` first: `bed["age"] / (info["grows"] * 1.0)`.

## Where to take it

- **Seasons.** A season in the state, plants that only grow in some of them, and a
  background colour per season.
- **Pests.** A bad day that halves the water of one bed, and a new verb to chase
  them away.
- **Instant watering.** A `local` handler on the Water button, so the gauge fills
  on the frame you click and the server catches up after; the
  [notes tutorial](/docs/blog/eui-notes-app) shows how.
- **A market.** Selling prices that move a little every day, drawn as a line on
  the herbarium chart (path kind `0`).

## The whole garden

Every file, exactly as it ran for the screenshots above.

### garden.sl

```soli
# Pocket Garden: `soli garden.sl` opens the window.
import "./species.sl"
import "./beds.sl"
import "./shop.sl"
import "./herbarium.sl"

def pocket_garden(event_data)
  state = event_data["state"] || Garden.fresh
  props = (event_data["params"] ?? {})["props"] ?? {}
  match event_data["event"] {
    "tick" => Garden.tick(state),
    "tab" => state.merge({"tab": props["tab"]}),
    "go_garden" => state.merge({"tab": "Garden"}),
    "pick" => state.merge({"pick": props["kind"]}),
    "plant" => Garden.plant(state, props["bed"]),
    "water" => Garden.water(state, props["bed"]),
    "harvest" => Garden.harvest(state, props["bed"]),
    "buy" => Garden.buy(state, props["kind"]),
    _ => state
  }
end

def header(state)
  row({"gap": 4, "align": "center"}, [
    column({"gap": 1, "grow": 1}, [h1("Pocket Garden"), muted(state["news"])]),
    badge("Day #{state["day"]}", "info"),
    badge("#{state["coins"]} coins", "warning")
  ])
end

def pocket_garden_view(state)
  state = Garden.fresh unless state["beds"]
  screen = match state["tab"] {
    "Shop" => shop_screen(state),
    "Herbarium" => herbarium_screen(state),
    _ => garden_screen(state)
  }
  root = column({"pad": 7, "gap": 5, "bg": "surface.base", "height": "100%"}, [
    header(state),
    tabs(["Garden", "Shop", "Herbarium"], state["tab"], "tab"),
    screen
  ])
  # The clock: one `tick` a second, only while something is growing. An
  # empty or fully ripe garden asks for no wakeups at all.
  return root unless Garden.growing?(state)

  root["p"] = (root["p"] ?? {}).merge({"wake": 1000})
  root["on"] = (root["on"] ?? {}).merge({"wake": "tick"})
  root
end

eui_window("pocket-garden", "pocket_garden", "pocket_garden_view", {"title": "Pocket Garden"})
```

### species.sl

```soli
# Everything the game knows about a plant. `export const` does not exist, so
# shared data is a function: the window's workers load definitions, not
# variables, and a function is a definition.
export def species
  {
    "radish":     {"name": "Radish",     "price": 2,  "grows": 6,  "sells": 5},
    "sunflower":  {"name": "Sunflower",  "price": 5,  "grows": 12, "sells": 14},
    "strawberry": {"name": "Strawberry", "price": 8,  "grows": 18, "sells": 24},
    "lavender":   {"name": "Lavender",   "price": 12, "grows": 26, "sells": 40}
  }
end

export def species_order
  ["radish", "sunflower", "strawberry", "lavender"]
end
```

### beds.sl

```soli
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
```

### shop.sl

```soli
import "./species.sl"
import "./beds.sl"

export def shop_card(state, kind)
  info = species()[kind]
  short = info["price"] - state["coins"]
  action = with_props(button("Buy for #{info["price"]}", "buy"), {"kind": kind})
  action = muted("#{short} more coins needed") if short > 0
  card({"grow": 1, "basis": 0, "gap": 3, "align": "center"}, [
    canvas(180, 150, plant_paths(kind, 1.0, false)),
    text(info["name"], {"size": 2, "weight": "semibold"}),
    muted("Ripe in #{info["grows"]}s, sells for #{info["sells"]}"),
    muted("You have #{state["seeds"][kind]}"),
    action
  ])
end

export def shop_screen(state)
  row({"gap": 4}, species_order().map { |kind| shop_card(state, kind) })
end
```

### herbarium.sl

```soli
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
```
