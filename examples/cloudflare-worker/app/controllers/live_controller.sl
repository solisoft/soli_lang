# LiveView handlers: `router_live("tally", "live#tally")` in config/routes.sl.
# Each takes {event, params, state} and returns the new state — on the edge
# exactly as under `soli serve`.

# The shared board on /live.
def tally(event_data)
  state = event_data["state"]
  count = state["count"] || 0
  clicks = state["clicks"] || 0

  changes = match event_data["event"] {
    "increment" => {"count": count + 1, "clicks": clicks + 1},
    "decrement" => {"count": count - 1, "clicks": clicks + 1},
    "reset" => {"count": 0, "clicks": clicks + 1},
    _ => {"count": count, "clicks": clicks}
  }
  state.merge(changes)
end
