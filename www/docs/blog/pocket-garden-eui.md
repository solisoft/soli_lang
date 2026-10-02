# Pocket Garden: a Little Game in a Native Window, from One Script

Most EUI examples are tools: a counter, a notes list, a dashboard. This one is a
game. **Pocket Garden** has four beds, a seed shop and a herbarium. You plant a
radish, water it, and watch it grow in real time; leave it too long without water
and it goes grey and stops. Every eighth second is a new day, and every third day
it rains on everything at once. Ripe plants sell for coins, coins buy better
seeds, and every harvest is pressed into the herbarium as a bar on a chart.

It is 399 lines of Soli in six files, and it runs from a plain script:
`soli garden.sl`, no `soli new`, no server to start. Every screenshot below is
that script, drawn by the real EUI client, and every line of code here is the
code that drew it.

<figure style="margin:1.5rem auto;max-width:1024px;">
  <img src="/images/blog/pocket-garden.svg" width="1024" height="576" alt="A native window titled Pocket Garden with four garden beds: a ripe radish with a red root, a sunflower with yellow petals, a strawberry bush with red berries and three lavender spikes. Beside it, a script file named garden.sl importing four other files, and a clock that ticks once a second while something grows." style="display:block;width:100%;height:auto;border-radius:12px;border:1px solid #30363d;background:#0b0d0f;">
  <figcaption style="text-align:center;color:#8b949e;font-size:0.875rem;margin-top:0.5rem;">Six files, one window, a clock that only ticks while something grows.</figcaption>
</figure>

On the way it covers what a bigger EUI app needs and the
[notes tutorial](/docs/blog/eui-notes-app) left out: splitting the code across
files, keeping the rules in a class, drawing with `canvas`, a clock with `wake`,
and several screens in one window.

## Play it here

The garden below is the game itself: the same `game.sl`, served by this site, and
drawn by the EUI client compiled to WebAssembly on a `<canvas>`. Nothing is
downloaded until you press the button. Your garden lasts as long as the page is
open; plant a radish, keep it watered, and it is ripe in six seconds.

<figure class="not-prose pg-play" data-eui data-component="pocket-garden" data-allow="">
  <div class="demo__stage">
    <img class="demo__poster" src="/images/blog/pocket-garden-play-dark.png" data-light="/images/blog/pocket-garden-play-light.png" data-dark="/images/blog/pocket-garden-play-dark.png" width="1520" height="1520" alt="Pocket Garden in the page: a radish and a sunflower growing in two of four beds, each watered four times out of five, under the seed chips and the Garden, Shop and Herbarium tabs." loading="lazy" decoding="async">
    <canvas class="demo__canvas" id="eui-pocket-garden" tabindex="0" aria-label="Pocket Garden, running: a live EUI session" hidden></canvas>
    <button class="demo__run" type="button">Play <small>8 MB download</small></button>
    <p class="demo__note" role="status" hidden></p>
  </div>
  <figcaption>The picture is a real render of this session. Press Play and it becomes the game: one socket to this site, and the GPU draws every frame.</figcaption>
</figure>

## What you need

- **A Soli with `eui_window`**, the first release after 2.11.1.
- **A `soli` that can open a window.** The window is the `eui-desktop` feature,
  which the published binaries leave out. Build it from the repository:
  `cargo install --path . --locked --features eui-desktop`.

## The shape

```
garden.sl      # the script you run: opens the window
game.sl        # the handler, the header, the tabs and the view
species.sl     # the four plants: price, growing time, selling price
beds.sl        # the rules (a Garden class), the plant drawings, the garden screen
shop.sl        # the seed shop
herbarium.sl   # the harvest chart
```

`garden.sl` is the script you run, and it is three lines: it imports `game.sl`
and opens the window on the two functions `game.sl` exports.

```soli
# Pocket Garden: `soli garden.sl` opens the window.
import "./game.sl"

eui_window("pocket-garden", "pocket_garden", "pocket_garden_view", {"title": "Pocket Garden"})
```

`game.sl` imports the other four. Keeping the handler and the view out of the
file that opens the window means another program can import them too: the
playable copy on this page is the same `game.sl`, served by the docs site.

## Step 1: a window from a script

`eui_window(name, handler, view, options)` takes the names of two `def`s of the
script, its own or imported. The handler turns an event into the next state; the
view turns state into a tree of nodes. Here is the handler, from `game.sl`, in
full:

```soli
export def pocket_garden(event_data)
  state = event_data["state"] || Garden.fresh
  params = event_data["params"] ?? {}
  props = params["props"] ?? {}
  # `connect` and every resize carry the window's size: the screens lay
  # their cards out by its width.
  state = state.merge({"width": params["viewport"]["width"]}) if params["viewport"]
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

Four details carry most of the weight.

**`event_data["state"] || Garden.fresh`** starts a new game. The first event is
`connect`, and it arrives with an empty state. In Soli an empty hash is falsy, so
`||` swaps it for a fresh garden; `??` would not, because `{}` is not `nil`.

**The handler is a list of verbs.** Each arm hands the state to a function and
returns what that function returns. The rules live elsewhere, so this stays the
one place you read to know what the game can do.

**`_ => state` keeps the game when nothing matches.** The runtime also sends
events you never asked for, such as `connect` and `viewport` when the window is
resized. Falling through to `nil` would throw the garden away on the first resize.

**The window's width rides in the state.** `connect` and every `viewport` event
carry the window's size in `params`, and the handler keeps the width before it
matches. The screens read it: `four_across` in `beds.sl` puts the four beds in a
row, two by two below 1000 pixels, one per row on a phone, and the herbarium's
chart narrows with the window. That is how one game fits a desktop window and the
copy at the top of this page.

The view and the handler do not run where the script does. They run in the
window's server workers, which load the script's *definitions* (`def`, classes,
enums, constants) and nothing else. A top-level variable is invisible to them, so
everything the game knows is in the state, behind a function or in a `const`.

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

Why static methods over hashes, and not a `Bed` object with a `water` method?
Because the state is kept as JSON between events. An object put in it comes back
on the next event as a plain hash of its fields, without its class or methods: a
method call on it reads a missing key and gives `nil`, so the first click works
and the second quietly does not. So the state stays plain data, and the class
works *on* it. Under `--dev` and in an `eui_window` script, Soli refuses a handler
that puts an instance in the state, and names the field and the class.

## Step 3: splitting the code into files

`import "./beds.sl"` brings in what the file exports, and whatever those exports
need to run: the helpers beside them and the files they import. Two things to
know:

- **Export what the other files call.** A function another file uses is written
  `export def`, and the class is `export class Garden`; `export const` and
  `export enum` exist too. A plain `def` comes along with its file, so the
  exports that call it work, but it is that file's own business.
- **Shared data is a function or a constant.** Both are definitions, so the
  window's workers can see them. A top-level variable is not.

```soli
# Everything the game knows about a plant. The window's workers load
# definitions, not variables: a function is one (so is a `const`), a
# top-level variable is not.
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
export def pocket_garden_view(state)
  state = Garden.fresh unless state["beds"]
  screen = match state["tab"] {
    "Shop" => shop_screen(state),
    "Herbarium" => herbarium_screen(state),
    _ => garden_screen(state)
  }
  # The screen scrolls under the header and the tabs: a short window, or the
  # copy in the blog post, still reaches the bottom row.
  root = column({"pad": 7, "gap": 5, "bg": "surface.base", "height": "100%"}, [
    header(state),
    tabs(["Garden", "Shop", "Herbarium"], state["tab"], "tab"),
    scroll({"grow": 1, "basis": 0}, [screen])
  ])
  # The clock: one `tick` a second, only while something is growing. An
  # empty or fully ripe garden asks for no wakeups at all.
  return root unless Garden.growing?(state)

  root["p"] = (root["p"] ?? {}).merge({"wake": 1000})
  root["on"] = (root["on"] ?? {}).merge({"wake": "tick"})
  root
end
```

The screen sits in a `scroll` under the header and the tabs, so a short window
still reaches the bottom row. The last four lines are the clock. While something grows, the root node asks for a
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
    # The catalogue's bar grows to fill its parent, and a card is a column:
    # alone in it, it would grow downwards. A row of its own makes it grow across.
    row({"width": 160}, [progress(frac)]),
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

## Ship it

A game nobody can install is a game for one. `soli build`, given a script
instead of an application folder, writes one executable: the program, its
imports already resolved and type-checked, and the Soli runtime that built it.

```bash
$ soli build garden.sl -o pocket-garden
Building pocket-garden from garden.sl...
  Built pocket-garden (80.2 MB, linux-x86_64, runs on the VM, or the tree-walker if it needs it)
```

Copy that one file to an empty folder on another machine and it runs: no `soli`,
no `.sl` files, no catalogue to install. The widget catalogue is part of the
runtime, and the six files are part of the program.

Two rules decide whether it opens a window:

- **Build with an `eui-desktop` soli.** The executable embeds the soli that built
  it. Built with a published binary, it serves the session and opens nothing.
- **Build on the platform you ship to.** `--target` cross-builds by fetching a
  prebuilt runtime for the other platform, and that runtime has no window either.
  A macOS build is made on a Mac.

### A disk image for macOS

A Mac user expects an app in a `.dmg`, not a bare executable. The repository has
the game in
[`examples/pocket-garden`](https://github.com/solisoft/soli_lang/tree/main/examples/pocket-garden),
and a GitHub Actions workflow,
[`pocket-garden-macos.yml`](https://github.com/solisoft/soli_lang/blob/main/.github/workflows/pocket-garden-macos.yml),
that does the whole thing on an Apple Silicon runner:

1. `cargo build --release --features eui-desktop`, for a soli with the window.
2. `soli build examples/pocket-garden/garden.sl -o build/PocketGarden`.
3. `Pocket Garden.app`: the executable in `Contents/MacOS`, an `Info.plist` naming
   it, and an icon turned into `PocketGarden.icns` by `sips` and `iconutil`.
4. `codesign --force -s -` on the app: an ad-hoc signature, which Apple Silicon
   requires before it runs anything. Use `codesign` itself, not a tool that
   rewrites the binary: the program `soli build` appended rides inside the
   executable's `__LINKEDIT` segment, and a rewrite drops it.
5. `hdiutil create … -format UDZO` over a folder holding the app and a link to
   `/Applications`, so the disk image opens on the familiar drag-to-install
   window.

The workflow also starts the packaged executable with `SOLI_EUI_NO_WINDOW=1` and
checks it serves its session, so a broken bundle fails the build, not a player.

The app is signed but not notarized. Notarization needs a paid Apple developer
account; without it, macOS refuses the first launch of a downloaded copy. On
macOS 15 and later, try to open it once, then go to **System Settings → Privacy &
Security** and click **Open Anyway**. On macOS 14 and earlier, Control-click the
app and choose **Open**. After that it opens like any other.

## Four things that tripped us up

- **A top-level variable never reaches the window.** The handler and the view
  run in the window's workers, which load definitions only: data goes in a
  function or a `const` (Step 1).
- **An object in the state comes back as a hash.** Keep the state plain data and
  put the behaviour in a class around it (Step 2).
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
import "./game.sl"

eui_window("pocket-garden", "pocket_garden", "pocket_garden_view", {"title": "Pocket Garden"})
```

### game.sl

```soli
# Pocket Garden's handler and view: what the window shows and how it answers.
# `garden.sl` opens the window on them; the docs site serves the same pair
# to the copy that plays inside the blog post.
import "./species.sl"
import "./beds.sl"
import "./shop.sl"
import "./herbarium.sl"

export def pocket_garden(event_data)
  state = event_data["state"] || Garden.fresh
  params = event_data["params"] ?? {}
  props = params["props"] ?? {}
  # `connect` and every resize carry the window's size: the screens lay
  # their cards out by its width.
  state = state.merge({"width": params["viewport"]["width"]}) if params["viewport"]
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

export def header(state)
  row({"gap": 4, "align": "center"}, [
    column({"gap": 1, "grow": 1}, [h1("Pocket Garden"), muted(state["news"])]),
    badge("Day #{state["day"]}", "info"),
    badge("#{state["coins"]} coins", "warning")
  ])
end

export def pocket_garden_view(state)
  state = Garden.fresh unless state["beds"]
  screen = match state["tab"] {
    "Shop" => shop_screen(state),
    "Herbarium" => herbarium_screen(state),
    _ => garden_screen(state)
  }
  # The screen scrolls under the header and the tabs: a short window, or the
  # copy in the blog post, still reaches the bottom row.
  root = column({"pad": 7, "gap": 5, "bg": "surface.base", "height": "100%"}, [
    header(state),
    tabs(["Garden", "Shop", "Herbarium"], state["tab"], "tab"),
    scroll({"grow": 1, "basis": 0}, [screen])
  ])
  # The clock: one `tick` a second, only while something is growing. An
  # empty or fully ripe garden asks for no wakeups at all.
  return root unless Garden.growing?(state)

  root["p"] = (root["p"] ?? {}).merge({"wake": 1000})
  root["on"] = (root["on"] ?? {}).merge({"wake": "tick"})
  root
end
```

### species.sl

```soli
# Everything the game knows about a plant. The window's workers load
# definitions, not variables: a function is one (so is a `const`), a
# top-level variable is not.
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
    # The catalogue's bar grows to fill its parent, and a card is a column:
    # alone in it, it would grow downwards. A row of its own makes it grow across.
    row({"width": 160}, [progress(frac)]),
    water_note,
    with_props(action, {"bed": n})
  ])
end

# Four cards side by side, two rows of two in a window narrower than four
# 180-pixel pictures and their margins need, one per row on a phone. Each row
# is a `row`: a card's `grow` shares out width there, where in a column it
# would ask for a share of an unknown height.
export def four_across(state, cards)
  width = state["width"] ?? 1100
  return row({"gap": 4}, cards) if width >= 1000
  return column({"gap": 4}, cards.map { |card| row({}, [card]) }) if width < 600

  column({"gap": 4}, [
    row({"gap": 4}, [cards[0], cards[1]]),
    row({"gap": 4}, [cards[2], cards[3]])
  ])
end

export def garden_screen(state)
  chips = species_order().map { |kind| seed_chip(state, kind) }
  column({"gap": 5}, [
    row({"gap": 2, "align": "center", "wrap": "wrap"}, [text("Seeds", {"size": 1, "weight": "semibold"})] + chips),
    four_across(state, range(0, 4).map { |n| bed_card(state, n) })
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
  four_across(state, species_order().map { |kind| shop_card(state, kind) })
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
```
