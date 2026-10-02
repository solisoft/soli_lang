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
