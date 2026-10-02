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
