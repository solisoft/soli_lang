# Demos controller — backs the /demos/client-interactivity showcase.
# Each fragment endpoint is intentionally stateless (no DB, no session) and
# returns just the HTML HTMx will swap into the page.
class DemosController < Controller

  # Main showcase page (full layout).
  def index()
    render("demos/client_interactivity", {"title": "Client Interactivity Demo"})
  end

  # 1. Optimistic Like button.
  # Echoes back the button with the opposite "liked" state.
  def like
    liked = (params["liked"] ?? "false") == "true"
    next_liked = !liked
    render("demos/_like_button", {"liked": next_liked}, {"layout": false})
  end

  # 2. Live debounced search — filter a hardcoded fruit list.
  def search
    search_query = (params["q"] ?? "").downcase()
    fruits = [
      "Apple",
      "Apricot",
      "Avocado",
      "Banana",
      "Blackberry",
      "Blueberry",
      "Cherry",
      "Coconut",
      "Cranberry",
      "Date",
      "Dragonfruit",
      "Durian",
      "Elderberry",
      "Fig",
      "Gooseberry",
      "Grape",
      "Grapefruit",
      "Guava",
      "Kiwi",
      "Lemon",
      "Lime",
      "Lychee",
      "Mango",
      "Melon",
      "Mulberry",
      "Nectarine",
      "Orange",
      "Papaya",
      "Passionfruit",
      "Peach",
      "Pear",
      "Persimmon",
      "Pineapple",
      "Plum",
      "Pomegranate",
      "Raspberry",
      "Strawberry",
      "Tangerine",
      "Watermelon"
    ]
    matches = search_query == ""
      ? []
      : fruits.filter(fn(fruit) { fruit.downcase().contains(search_query) })
    render("demos/_search_results", {
      "matches": matches,
      "query": search_query
    }, {"layout": false})
  end

  # 3. Lazy tab content. Tab id comes from the URL.
  def tab_body
    id = params["id"] ?? "overview"
    now = DateTime.now()
    bodies = {
      "overview": {"title": "Overview", "body": "Lazy-loaded on click. Switch back — HTMx caches the result."},
      "stats": {"title": "Stats", "body": "47 widgets · 12 lines of JS · 0 build steps."},
      "activity": {"title": "Activity", "body": "Last refresh: #{now.format("%H:%M:%S")}."}
    }
    data = bodies[id] ?? bodies["overview"]
    render("demos/_tab_body", {"tab": data}, {"layout": false})
  end

  # 4. Inline edit. Accepts a new value, returns the read-only display row.
  def todo_update
    value = (params["value"] ?? "").trim()
    display = value == "" ? "(empty)" : value
    render("demos/_todo_row", {"value": display}, {"layout": false})
  end

  # 5. Toast trigger. Returns an empty body plus an HX-Trigger header that
  # spawns a client-side toast via the Alpine toast stack.
  def notify
    kind = params["kind"] ?? "info"
    message = match kind {
      "success" => "Saved! Your changes are live.",
      "warning" => "Heads up — that field is empty.",
      "error" => "Something went wrong. Try again.",
      _ => "Just an FYI: this is what `info` toasts look like.",
    }
    payload = json_stringify({
      "kind": kind,
      "message": message
    })
    {
      "status": 204,
      "headers": {"HX-Trigger": "{\"soli-toast\":#{payload}}"},
      "body": ""
    }
  end

  # 6. Live polling counter. Returns a fresh value every time it's polled.
  def counter()
    jitter = int(Math.random() * 8) + 2
    now = DateTime.now()
    value = (now.to_unix() % 100) + jitter
    render("demos/_counter", {"value": value}, {"layout": false})
  end

  # 7. Confirm-then-dismiss. HTMx swaps the row with an empty response so
  # it vanishes; Alpine animates the collapse just before the swap.
  def delete_item()
    {"status": 200, "body": ""}
  end

  # 9. Live polling bar chart. Each tick we synthesize a fresh 12-point
  # window (sine wave + jitter), render an SVG bar chart with a connected
  # sparkline overlay, and return the markup for HTMx to swap.
  def chart()
    bar_count = 12
    now = DateTime.now()
    bar_width = 240.0 / bar_count
    bars = []
    total = 0
    for i in 0..bar_count
      tick_unix = now.to_unix() - (bar_count - 1 - i) * 2
      wave = Math.sin(tick_unix * 0.35) * 25 + 55
      jitter = Math.random() * 20
      value = int(wave + jitter)
      if value < 5
        value = 5
      end
      if value > 100
        value = 100
      end
      bar_height = value * 0.7
      bars.push({
        "label": DateTime.from_unix(tick_unix).format("%H:%M:%S"),
        "value": value,
        "x": i * bar_width + 1,
        "y": 76 - bar_height,
        "w": bar_width - 2,
        "h": bar_height
      })
      total = total + value
    end

    # Build a polyline string ("x1,y1 x2,y2 ...") through each bar's top-center.
    polyline_points = []
    for bar in bars
      center_x = bar["x"] + bar["w"] / 2
      polyline_points.push("#{center_x},#{bar["y"]}")
    end

    render("demos/_chart", {
      "bars": bars,
      "polyline": polyline_points.join(" "),
      "avg": int(total / bar_count),
      "latest": bars[bar_count - 1]["value"],
      "first_label": bars[0]["label"],
      "last_label": bars[bar_count - 1]["label"]
    }, {"layout": false})
  end

  # 8. Modal form. HTMx fetches the form HTML; Alpine drops it into an
  # open <dialog>. Submitting echoes back a thank-you panel.
  def modal_form
    submitted = (params["submitted"] ?? "false") == "true"
    if submitted
      render("demos/_modal_thanks", {"name": params["name"] ?? "friend"}, {"layout": false})
    else
      render("demos/_modal_form", {}, {"layout": false})
    end
  end

  # ── 10. Users Table (dedicated DemoUser model backed by SoliDB) ──

  # Add User form (HTMx-loaded into the modal body in widget #10).
  def user_form()
    render("demos/_user_form", {}, {"layout": false})
  end

  def _users_result(search_query, sort_column, sort_direction, page, per_page)
    valid_sort_keys = ["name", "email", "role", "status", "last_login"]
    if !valid_sort_keys.contains(sort_column)
      sort_column = "name"
    end
    if sort_direction != "desc"
      sort_direction = "asc"
    end

    let query_builder
    if search_query == nil || search_query == ""
      query_builder = DemoUser.order(sort_column, sort_direction)
    else
      needle = search_query.downcase()
      query_builder = DemoUser.where(
        "CONTAINS(LOWER(doc.name), @needle) || CONTAINS(LOWER(doc.email), @needle)",
        { "needle": needle }
      ).order(sort_column, sort_direction)
    end

    result = query_builder.paginate({"page": page, "per": per_page})
    records = result["records"]
    pagination = result["pagination"]

    {
      "records": records,
      "total": pagination["total"],
      "page": pagination["page"],
      "total_pages": pagination["total_pages"],
      "per": pagination["per"],
      "start": records.length() > 0 ? (pagination["page"] - 1) * pagination["per"] + 1 : 0,
      "end_val": (pagination["page"] - 1) * pagination["per"] + records.length()
    }
  end

  def users
    sort_column    = params["sort"] ?? "name"
    sort_direction = params["dir"] ?? "asc"
    search_query   = params["q"] ?? ""
    result = this._users_result(
      search_query, sort_column, sort_direction,
      int(params["page"] ?? "1"), 10
    )

    render("demos/_users_table", {
      "users": result["records"],
      "total": result["total"],
      "page": result["page"],
      "total_pages": result["total_pages"],
      "per_page": result["per"],
      "sort": sort_column,
      "dir": sort_direction,
      "q": search_query,
      "start": result["start"],
      "end_val": result["end_val"]
    }, {"layout": false})
  end

  def user_update
    id = params["id"] ?? ""
    field = params["field"] ?? "name"
    value = params["value"] ?? ""

    user = DemoUser.find(id)
    attrs = {}
    if field == "status"
      attrs["status"] = user["status"] == "Active" ? "Inactive" : "Active"
    else
      attrs[field] = value
    end

    user.update(attrs)
    if user._errors
      # Validation failed (e.g. uniqueness on email). Re-render the row from
      # the persisted DB state and surface the first error message as a toast.
      original = DemoUser.find(id)
      response = render("demos/_user_row", {"user": original}, {"layout": false})
      first = user._errors[0] ?? {}
      err_msg = first["message"] ?? "Could not save changes."
      response["headers"]["HX-Trigger"] = json_stringify({
        "soli-toast": {"kind": "error", "message": err_msg}
      })
      return response
    end

    response = render("demos/_user_row", {"user": user}, {"layout": false})
    message = match field {
      "name" => "Name updated to \"#{user["name"]}\".",
      "email" => "Email updated to \"#{user["email"]}\".",
      "role" => "Role changed to \"#{user["role"]}\".",
      "status" => "User is now #{user["status"]}.",
      _ => "Saved."
    }
    response["headers"]["HX-Trigger"] = json_stringify({
      "soli-toast": {"kind": "success", "message": message}
    })
    response
  end

  def user_delete
    id = params["id"] ?? ""
    user = DemoUser.find(id)
    DemoUser.delete(id)
    {
      "status": 200,
      "headers": {
        "HX-Trigger": json_stringify({
          "soli-toast": {"kind": "success", "message": "Deleted \"#{user["name"]}\"."}
        })
      },
      "body": ""
    }
  end

  def user_create
    user = DemoUser.create({
      "name": params["name"] ?? "New User",
      "email": params["email"] ?? "new@example.com",
      "role": params["role"] ?? "Viewer",
      "status": params["status"] == "on" ? "Active" : "Inactive",
      "last_login": DateTime.now().format("%Y-%m-%d %H:%M")
    })

    if user._errors
      return {"status": 422, "body": str(user._errors)}
    end

    result = this._users_result(nil, "name", "asc", 1, 10)
    response = render("demos/_users_table", {
      "users": result["records"],
      "total": result["total"],
      "page": result["page"],
      "total_pages": result["total_pages"],
      "per_page": result["per"],
      "sort": "name",
      "dir": "asc",
      "q": "",
      "start": result["start"],
      "end_val": result["end_val"]
    }, {"layout": false})
    response["headers"]["HX-Trigger"] = json_stringify({
      "soli-toast": {"kind": "success", "message": "User \"#{user["name"]}\" added."},
      "soli-add-user-close": true
    })
    response
  end
end
