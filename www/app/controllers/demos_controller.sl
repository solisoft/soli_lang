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
  def like(req)
    let p = req["all"] ?? {}
    let liked = (p["liked"] ?? "false") == "true"
    let next_liked = !liked
    render("demos/_like_button", {"liked": next_liked}, {"layout": false})
  end

  # 2. Live debounced search — filter a hardcoded fruit list.
  def search(req)
    let p = req["all"] ?? {}
    let q = (p["q"] ?? "").downcase()
    let fruits = [
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
    let matches = q == "" ? [] : fruits.filter(fn(f) { f.downcase().contains(q) })
    render("demos/_search_results", {
      "matches": matches,
      "query": q
    }, {"layout": false})
  end

  # 3. Lazy tab content. Tab id comes from the URL.
  def tab_body(req)
    let p = req["all"] ?? {}
    let id = p["id"] ?? "overview"
    let now = DateTime.now()
    let bodies = {
      "overview": {"title": "Overview", "body": "Lazy-loaded on click. Switch back — HTMx caches the result."},
      "stats": {"title": "Stats", "body": "47 widgets · 12 lines of JS · 0 build steps."},
      "activity": {"title": "Activity", "body": "Last refresh: #{now.format("%H:%M:%S")}."}
    }
    let data = bodies[id] ?? bodies["overview"]
    render("demos/_tab_body", {"tab": data}, {"layout": false})
  end

  # 4. Inline edit. Accepts a new value, returns the read-only display row.
  def todo_update(req)
    let p = req["all"] ?? {}
    let value = (p["value"] ?? "").trim()
    let display = value == "" ? "(empty)" : value
    render("demos/_todo_row", {"value": display}, {"layout": false})
  end

  # 5. Toast trigger. Returns an empty body plus an HX-Trigger header that
  # spawns a client-side toast via the Alpine toast stack.
  def notify(req)
    let p = req["all"] ?? {}
    let kind = p["kind"] ?? "info"
    let message = match kind {
      "success" => "Saved! Your changes are live.",
      "warning" => "Heads up — that field is empty.",
      "error" => "Something went wrong. Try again.",
      _ => "Just an FYI: this is what `info` toasts look like.",
    }
    let payload = json_stringify({
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
    let jitter = int(Math.random() * 8) + 2
    let now = DateTime.now()
    let value = (now.to_unix() % 100) + jitter
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
    let n = 12
    let now = DateTime.now()
    let bar_width = 240.0 / n
    let bars = []
    let total = 0
    for i in 0..n
      let t = now.to_unix() - (n - 1 - i) * 2
      let wave = Math.sin(t * 0.35) * 25 + 55
      let jitter = Math.random() * 20
      let value = int(wave + jitter)
      if value < 5
        value = 5
      end
      if value > 100
        value = 100
      end
      let h = value * 0.7
      bars.push({
        "label": DateTime.from_unix(t).format("%H:%M:%S"),
        "value": value,
        "x": i * bar_width + 1,
        "y": 76 - h,
        "w": bar_width - 2,
        "h": h
      })
      total = total + value
    end

    # Build a polyline string ("x1,y1 x2,y2 ...") through each bar's top-center.
    let pts = []
    for bar in bars
      let cx = bar["x"] + bar["w"] / 2
      pts.push("#{cx},#{bar["y"]}")
    end

    render("demos/_chart", {
      "bars": bars,
      "polyline": pts.join(" "),
      "avg": int(total / n),
      "latest": bars[n - 1]["value"],
      "first_label": bars[0]["label"],
      "last_label": bars[n - 1]["label"]
    }, {"layout": false})
  end

  # 8. Modal form. HTMx fetches the form HTML; Alpine drops it into an
  # open <dialog>. Submitting echoes back a thank-you panel.
  def modal_form(req)
    let p = req["all"] ?? {}
    let submitted = (p["submitted"] ?? "false") == "true"
    if submitted
      render("demos/_modal_thanks", {"name": p["name"] ?? "friend"}, {"layout": false})
    else
      render("demos/_modal_form", {}, {"layout": false})
    end
  end

  # ── 10. Users Table (dedicated DemoUser model backed by SoliDB) ──

  # Add User form (HTMx-loaded into the modal body in widget #10).
  def user_form()
    render("demos/_user_form", {}, {"layout": false})
  end

  def _users_result(q, sort, dir, page, per_page)
    let valid_sort_keys = ["name", "email", "role", "status", "last_login"]
    if !valid_sort_keys.contains(sort)
      sort = "name"
    end
    if dir != "desc"
      dir = "asc"
    end

    let qb
    if q == nil || q == ""
      qb = DemoUser.order(sort, dir)
    else
      let needle = q.downcase()
      qb = DemoUser.where(
        "CONTAINS(LOWER(doc.name), @needle) || CONTAINS(LOWER(doc.email), @needle)",
        { "needle": needle }
      ).order(sort, dir)
    end

    let result = qb.paginate({"page": page, "per": per_page})
    let records = result["records"]
    let pg = result["pagination"]

    {
      "records": records,
      "total": pg["total"],
      "page": pg["page"],
      "total_pages": pg["total_pages"],
      "per": pg["per"],
      "start": records.length() > 0 ? (pg["page"] - 1) * pg["per"] + 1 : 0,
      "end_val": (pg["page"] - 1) * pg["per"] + records.length()
    }
  end

  def users(req)
    let p = req["all"] ?? {}
    let sort = p["sort"] ?? "name"
    let dir = p["dir"] ?? "asc"
    let q = p["q"] ?? ""
    let r = this._users_result(q, sort, dir, int(p["page"] ?? "1"), 10)

    render("demos/_users_table", {
      "users": r["records"],
      "total": r["total"],
      "page": r["page"],
      "total_pages": r["total_pages"],
      "per_page": r["per"],
      "sort": sort,
      "dir": dir,
      "q": q,
      "start": r["start"],
      "end_val": r["end_val"]
    }, {"layout": false})
  end

  def user_update(req)
    let p = req["all"] ?? {}
    let id = p["id"] ?? ""
    let field = p["field"] ?? "name"
    let value = p["value"] ?? ""

    let user = DemoUser.find(id)
    if user.nil?
      return {"status": 404, "body": ""}
    end

    let attrs = {}
    if field == "status"
      attrs["status"] = user["status"] == "Active" ? "Inactive" : "Active"
    else
      attrs[field] = value
    end

    user.update(attrs)
    if user._errors
      # Validation failed (e.g. uniqueness on email). Re-render the row from
      # the persisted DB state and surface the first error message as a toast.
      let original = DemoUser.find(id)
      let response = render("demos/_user_row", {"user": original}, {"layout": false})
      let first = user._errors[0] ?? {}
      let err_msg = first["message"] ?? "Could not save changes."
      response["headers"]["HX-Trigger"] = json_stringify({
        "soli-toast": {"kind": "error", "message": err_msg}
      })
      return response
    end

    let response = render("demos/_user_row", {"user": user}, {"layout": false})
    let message = match field {
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

  def user_delete(req)
    let p = req["all"] ?? {}
    let id = p["id"] ?? ""
    let user = DemoUser.find(id)
    let name = user.nil? ? "user" : user["name"]
    DemoUser.delete(id)
    {
      "status": 200,
      "headers": {
        "HX-Trigger": json_stringify({
          "soli-toast": {"kind": "success", "message": "Deleted \"#{name}\"."}
        })
      },
      "body": ""
    }
  end

  def user_create(req)
    let p = req["all"] ?? {}
    let user = DemoUser.create({
      "name": p["name"] ?? "New User",
      "email": p["email"] ?? "new@example.com",
      "role": p["role"] ?? "Viewer",
      "status": p["status"] == "on" ? "Active" : "Inactive",
      "last_login": DateTime.now().format("%Y-%m-%d %H:%M")
    })

    if user._errors
      return {"status": 422, "body": str(user._errors)}
    end

    let r = this._users_result(nil, "name", "asc", 1, 10)
    let response = render("demos/_users_table", {
      "users": r["records"],
      "total": r["total"],
      "page": r["page"],
      "total_pages": r["total_pages"],
      "per_page": r["per"],
      "sort": "name",
      "dir": "asc",
      "q": "",
      "start": r["start"],
      "end_val": r["end_val"]
    }, {"layout": false})
    response["headers"]["HX-Trigger"] = json_stringify({
      "soli-toast": {"kind": "success", "message": "User \"#{user["name"]}\" added."},
      "soli-add-user-close": true
    })
    response
  end
end
