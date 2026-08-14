# LiveView controller - handles Live View events
//
# Live View handlers receive an event hash with:
# - event: The event name (e.g., "increment", "decrement")
# - params: Parameters sent with the event
# - state: The current component state
//
# Handlers should return the new state as a hash.

# Counter component handler.
# NOTE: the handler's return REPLACES the component state (only `id` is
# preserved), so every branch must return the complete state hash.
def counter(event_data)
    let event = event_data["event"]
    let state = event_data["state"]
    let params = event_data["params"]
    let count = state["count"] || 0
    let typed = state["typed"] || ""

    if (event == "increment")
        return {
            "count": count + 1,
            "typed": typed
        }
    end

    if (event == "decrement")
        return {
            "count": count - 1,
            "typed": typed
        }
    end

    # Echo the input field through the server: every keystroke round-trips
    # and the patch lands while the field is focused — the DOM morph is what
    # keeps the caret and focus intact.
    if (event == "set_text")
        return {
            "count": count,
            "typed": params["value"] || ""
        }
    end

    # Return unchanged state for unknown events
    state
end

# Metrics component handler - Binary Clock.
# Always recomputes from the current time (cheap), so the very first render
# (mount, no `tick` event yet) has every `ms*`/`s*`/`m*`/`h*` variable
# defined in template scope. Without this, the initial paint errored with
# "Undefined variable 'ms9'" until JS sent the first tick.
def metrics(event_data)
    let event = event_data["event"]
    let state = event_data["state"]

    # Skip explicitly-known no-ops so callers can still pass non-tick events
    # without forcing a re-render. Anything else (including nil / "mount")
    # falls through to the recompute path below.
    if (event != nil && event != "" && event != "tick")
        return state
    end

    # Get current time using DateTime class
    let now = DateTime.utc()
    let h = now.hour()
    let m = now.minute()
    let s = now.second()
    let ms = now.millisecond()

    # Format strings with leading zeros
    let hours_str = "" + h
    let minutes_str = "" + m
    let seconds_str = "" + s

    if (h < 10)
        hours_str = "0" + h
    end
    if (m < 10)
        minutes_str = "0" + m
    end
    if (s < 10)
        seconds_str = "0" + s
    end

    let milliseconds_str = "" + ms
    if (ms < 100)
        if (ms < 10)
            milliseconds_str = "00" + ms
        else
            milliseconds_str = "0" + ms
        end
    end

    # Binary clock bits (pre-computed for template)
    # Hours: 5 bits (0-23)
    let h4 = 0
    let h3 = 0
    let h2 = 0
    let h1 = 0
    let h0 = 0

    let hv = h
    if (hv >= 16)  h4 = 1; hv = hv - 16 end
    if (hv >= 8)   h3 = 1; hv = hv - 8 end
    if (hv >= 4)   h2 = 1; hv = hv - 4 end
    if (hv >= 2)   h1 = 1; hv = hv - 2 end
    if (hv >= 1)   h0 = 1 end

    # Minutes: 6 bits (0-59)
    let m5 = 0
    let m4 = 0
    let m3 = 0
    let m2 = 0
    let m1 = 0
    let m0 = 0

    let mv = m
    if (mv >= 32)  m5 = 1; mv = mv - 32 end
    if (mv >= 16)  m4 = 1; mv = mv - 16 end
    if (mv >= 8)   m3 = 1; mv = mv - 8 end
    if (mv >= 4)   m2 = 1; mv = mv - 4 end
    if (mv >= 2)   m1 = 1; mv = mv - 2 end
    if (mv >= 1)   m0 = 1 end

    # Seconds: 6 bits (0-59)
    let s5 = 0
    let s4 = 0
    let s3 = 0
    let s2 = 0
    let s1 = 0
    let s0 = 0

    let sv = s
    if (sv >= 32)  s5 = 1; sv = sv - 32 end
    if (sv >= 16)  s4 = 1; sv = sv - 16 end
    if (sv >= 8)   s3 = 1; sv = sv - 8 end
    if (sv >= 4)   s2 = 1; sv = sv - 4 end
    if (sv >= 2)   s1 = 1; sv = sv - 2 end
    if (sv >= 1)   s0 = 1 end

    # Milliseconds: 10 bits (0-999)
    let ms9 = 0
    let ms8 = 0
    let ms7 = 0
    let ms6 = 0
    let ms5 = 0
    let ms4 = 0
    let ms3 = 0
    let ms2 = 0
    let ms1 = 0
    let ms0 = 0

    let msv = ms
    if (msv >= 512)  ms9 = 1; msv = msv - 512 end
    if (msv >= 256)  ms8 = 1; msv = msv - 256 end
    if (msv >= 128)  ms7 = 1; msv = msv - 128 end
    if (msv >= 64)   ms6 = 1; msv = msv - 64 end
    if (msv >= 32)   ms5 = 1; msv = msv - 32 end
    if (msv >= 16)   ms4 = 1; msv = msv - 16 end
    if (msv >= 8)    ms3 = 1; msv = msv - 8 end
    if (msv >= 4)    ms2 = 1; msv = msv - 4 end
    if (msv >= 2)    ms1 = 1; msv = msv - 2 end
    if (msv >= 1)    ms0 = 1 end

    return {
        "hours_str": hours_str,
        "minutes_str": minutes_str,
        "seconds_str": seconds_str,
        "milliseconds": ms,
        "milliseconds_str": milliseconds_str,
        "h4": h4, "h3": h3, "h2": h2, "h1": h1, "h0": h0,
        "m5": m5, "m4": m4, "m3": m3, "m2": m2, "m1": m1, "m0": m0,
        "s5": s5, "s4": s4, "s3": s3, "s2": s2, "s1": s1, "s0": s0,
        "ms9": ms9, "ms8": ms8, "ms7": ms7, "ms6": ms6, "ms5": ms5,
        "ms4": ms4, "ms3": ms3, "ms2": ms2, "ms1": ms1, "ms0": ms0
    }
end

# ---------------------------------------------------------------------------
# Field Desk — interactive sample for the LiveView tutorial blog post.
# State is in-memory (this page has to run on the docs site with no extra
# database). The post shows the Model / Job code you would ship instead.
# ---------------------------------------------------------------------------

def desk_seed
    [
        {"id": "n1", "title": "Inspect north hatch", "body": "Seal weeps after last storm. Photo the gasket before you pull it.", "status": "open", "priority": 2, "pinned": true, "file": null},
        {"id": "n2", "title": "Swap radio battery", "body": "Unit 4 is under 20%. Spare pack is in the van, second drawer.", "status": "doing", "priority": 3, "pinned": false, "file": null},
        {"id": "n3", "title": "Log pump hours", "body": "Write the hour-meter before you leave the pad.", "status": "open", "priority": 1, "pinned": false, "file": null},
        {"id": "n4", "title": "Close the east gate", "body": "Latch checked, chain on, photo filed.", "status": "done", "priority": 1, "pinned": false, "file": {"name": "gate.jpg", "size": 184320, "processed": true}}
    ]
end

def desk_filter(notes, tab, q)
    filtered = notes
    if tab != "all" && !tab.blank?
        filtered = filtered.filter(fn(note) note["status"] == tab)
    end
    if !q.blank?
        needle = q.downcase()
        filtered = filtered.filter(fn(note) {
            title = (note["title"] || "").downcase()
            body = (note["body"] || "").downcase()
            title.contains(needle) || body.contains(needle)
        })
    end
    filtered
end

def desk_counts(notes)
    {
        "all": notes.length(),
        "open": notes.filter(fn(note) note["status"] == "open").length(),
        "doing": notes.filter(fn(note) note["status"] == "doing").length(),
        "done": notes.filter(fn(note) note["status"] == "done").length()
    }
end

def desk_find(notes, id)
    for note in notes
        return note if note["id"] == id
    end
    null
end

def desk_pack(state)
    notes = state["notes"] || desk_seed()
    tab = state["tab"] || "all"
    q = state["q"] || ""
    selected_id = state["selected_id"]
    visible = desk_filter(notes, tab, q)
    selected = desk_find(visible, selected_id)
    selected = visible[0] if selected == null && visible.length() > 0
    selected_id = selected["id"] if selected != null
    pending = 0
    for note in notes
        file = note["file"]
        pending = pending + 1 if file != null && file["processed"] != true
    end
    {
        "notes": notes,
        "visible": visible,
        "tab": tab,
        "q": q,
        "selected_id": selected_id,
        "selected": selected,
        "counts": desk_counts(notes),
        "draft_title": state["draft_title"] || "",
        "draft_body": state["draft_body"] || "",
        "composer_open": state["composer_open"] == true,
        "menu_id": state["menu_id"],
        "focus": state["focus"] || 3,
        "flash": state["flash"] || "",
        "pending": pending
    }
end

def desk(event_data)
    event = event_data["event"]
    state = event_data["state"] || {}
    params = event_data["params"] || {}
    packed = desk_pack(state)

    if event == "connect" || event == null || event == ""
        if packed["notes"].length() > 0 && packed["notes"][0]["id"] != null
            return desk_pack(packed)
        end
        return desk_pack({ "notes": desk_seed(), "tab": "all", "focus": 3 })
    end

    if event == "search"
        packed["q"] = params["value"] || params["q"] || ""
        packed["menu_id"] = null
        return desk_pack(packed)
    end

    if event == "tab" || event == "patch"
        next_tab = params["tab"]
        if next_tab.blank?
            query = params["query"] || {}
            next_tab = query["tab"] || "all"
        end
        next_tab = "all" unless ["all", "open", "doing", "done"].includes?(next_tab)
        packed["tab"] = next_tab
        packed["menu_id"] = null
        return {
            "state": desk_pack(packed),
            "patch": "/docs/blog/liveview-desk?tab=#{next_tab}"
        }
    end

    if event == "select"
        packed["selected_id"] = params["id"]
        packed["menu_id"] = null
        return desk_pack(packed)
    end

    if event == "toggle_composer"
        packed["composer_open"] = packed["composer_open"] != true
        packed["menu_id"] = null
        return desk_pack(packed)
    end

    if event == "close_composer"
        packed["composer_open"] = false
        return desk_pack(packed)
    end

    if event == "draft"
        packed["draft_title"] = params["title"] || packed["draft_title"]
        packed["draft_body"] = params["body"] || packed["draft_body"]
        packed["draft_title"] = params["value"] if params["name"] == "title"
        packed["draft_body"] = params["value"] if params["name"] == "body"
        return desk_pack(packed)
    end

    if event == "create"
        title = (params["title"] || packed["draft_title"] || "").trim()
        body = (params["body"] || packed["draft_body"] || "").trim()
        if title.blank?
            packed["flash"] = "A note needs a title."
            packed["composer_open"] = true
            return {
                "state": desk_pack(packed),
                "js": [{ "op": "add_class", "to": "#desk-toast", "class": "desk-toast-on" }]
            }
        end
        notes = packed["notes"] || []
        note = {
            "id": "n#{str(datetime_now())}-#{str(notes.length())}",
            "title": title,
            "body": body,
            "status": "open",
            "priority": packed["focus"] || 2,
            "pinned": false,
            "file": null
        }
        notes = [note] + notes
        packed["notes"] = notes
        packed["selected_id"] = note["id"]
        packed["draft_title"] = ""
        packed["draft_body"] = ""
        packed["composer_open"] = false
        packed["flash"] = "Filed “#{title}”."
        packed["tab"] = "all" if packed["tab"] == "done"
        return {
            "state": desk_pack(packed),
            "js": [{ "op": "add_class", "to": "#desk-toast", "class": "desk-toast-on" }]
        }
    end

    if event == "pin"
        notes = packed["notes"]
        for note in notes
            if note["id"] == params["id"]
                note["pinned"] = note["pinned"] != true
            end
        end
        packed["notes"] = notes
        packed["menu_id"] = null
        return desk_pack(packed)
    end

    if event == "cycle"
        order = ["open", "doing", "done"]
        notes = packed["notes"]
        for note in notes
            if note["id"] == params["id"]
                idx = order.index_of(note["status"]) || 0
                next_idx = (idx + 1) % order.length()
                note["status"] = order[next_idx]
            end
        end
        packed["notes"] = notes
        packed["menu_id"] = null
        return desk_pack(packed)
    end

    if event == "open_menu"
        packed["menu_id"] = params["id"]
        return desk_pack(packed)
    end

    if event == "close_menu"
        packed["menu_id"] = null
        return desk_pack(packed)
    end

    if event == "set_focus"
        next_focus = params["focus"] || packed["focus"] || 3
        next_focus = next_focus.to_i() if next_focus != null
        next_focus = 3 unless [1, 2, 3, 4, 5].includes?(next_focus)
        packed["focus"] = next_focus
        selected = packed["selected"]
        if selected != null
            notes = packed["notes"]
            for note in notes
                if note["id"] == selected["id"]
                    note["priority"] = next_focus
                end
            end
            packed["notes"] = notes
        end
        return desk_pack(packed)
    end

    if event == "attached"
        file = params["file"] || {}
        err = params["error"] || file["error"]
        if err.present?
            packed["flash"] = "Upload failed: #{err}"
            return {
                "state": desk_pack(packed),
                "js": [{ "op": "add_class", "to": "#desk-toast", "class": "desk-toast-on" }]
            }
        end
        selected = packed["selected"]
        if selected == null
            packed["flash"] = "Pick a note before attaching a file."
            return {
                "state": desk_pack(packed),
                "js": [{ "op": "add_class", "to": "#desk-toast", "class": "desk-toast-on" }]
            }
        end
        notes = packed["notes"]
        preview = null
        payload = file["data"]
        content_type = file["content_type"] || ""
        if !payload.blank? && content_type.starts_with?("image/")
            preview = "data:#{content_type};base64,#{payload}"
        end
        for note in notes
            if note["id"] == selected["id"]
                note["file"] = {
                    "name": file["filename"] || file["name"] || "upload",
                    "size": file["size"] || 0,
                    "content_type": content_type,
                    "preview": preview,
                    "processed": false
                }
            end
        end
        packed["notes"] = notes
        packed["flash"] = "Queued #{file["filename"] || "file"} — processing…"
        return {
            "state": desk_pack(packed),
            "tick_interval": 900,
            "js": [{ "op": "add_class", "to": "#desk-toast", "class": "desk-toast-on" }]
        }
    end

    if event == "tick"
        notes = packed["notes"]
        still = false
        for note in notes
            file = note["file"]
            if file != null && file["processed"] != true
                file["processed"] = true
                note["file"] = file
            end
        end
        packed["notes"] = notes
        packed["flash"] = "Attachment ready." if packed["pending"] > 0
        next_state = desk_pack(packed)
        return {
            "state": next_state,
            "tick_interval": 0
        }
    end

    desk_pack(packed)
end

def desk_pulse(event_data)
    event = event_data["event"]
    state = event_data["state"] || {}
    series = state["series"] || [4, 6, 5, 8, 7, 9, 6, 10]

    if event == "connect" || event == "tick" || event == null || event == ""
        next_val = 4 + (datetime_now() % 8)
        series = series + [next_val]
        series = series.slice(-12) if series.length() > 12
        now = DateTime.utc()
        stamp = now.format("%H:%M:%S")
        wrapped = {
            "state": {
                "series": series,
                "series_json": json_stringify(series),
                "stamp": stamp,
                "peak": series.max()
            }
        }
        wrapped["tick_interval"] = 2000 if event == "connect" || event == null || event == ""
        return wrapped
    end

    state
end
