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
