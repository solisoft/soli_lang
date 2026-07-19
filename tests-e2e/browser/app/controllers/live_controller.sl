# LiveView handler for the browser fixture.
#
# The handler's return REPLACES the component state (only `id` survives), so
# every branch returns the complete state hash.
def counter(event_data)
    let event = event_data["event"]
    let state = event_data["state"]
    let params = event_data["params"]
    let count = state["count"] || 0
    let typed = state["typed"] || ""

    if event == "increment"
        return {"count": count + 1, "typed": typed}
    end

    if event == "decrement"
        return {"count": count - 1, "typed": typed}
    end

    # Echoes the field back through the server so the spec can prove the morph
    # preserves focus and caret rather than replacing the node.
    if event == "set_text"
        return {"count": count, "typed": params["value"] || ""}
    end

    state
end
