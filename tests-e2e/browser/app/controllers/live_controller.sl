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
    let path = state["path"] || ""
    let open = state["open"]
    if open == null
        open = true
    end

    if event == "increment"
        return {"count": count + 1, "typed": typed, "open": open, "path": path}
    end

    if event == "decrement"
        return {"count": count - 1, "typed": typed, "open": open, "path": path}
    end

    # Echoes the field back through the server so the spec can prove the morph
    # preserves focus and caret rather than replacing the node.
    if event == "set_text"
        return {"count": count, "typed": params["value"] || "", "open": open, "path": path}
    end

    if event == "away"
        return {"count": count, "typed": typed, "open": false, "path": path}
    end

    if event == "patch"
        let next_path = params["path"] || params["href"] || ""
        return {"count": count, "typed": typed, "open": open, "path": next_path}
    end

    {"count": count, "typed": typed, "open": open, "path": path}
end

def badge(event_data)
    { "label": "badge-ok" }
end

def about(event_data)
    { "title": "about-live" }
end

# A file arrives as a hydrated hash once every chunk has landed.
def uploads(event_data)
    let event = event_data["event"]
    let state = event_data["state"]
    let params = event_data["params"]

    if event == "attached"
        let file = params["file"]
        if file != null
            return {"name": file["filename"], "size": file["size"], "error": ""}
        end

        return {"name": "", "size": 0, "error": params["error"] || "no file"}
    end

    {"name": state["name"] || "", "size": state["size"] || 0, "error": state["error"] || ""}
end
