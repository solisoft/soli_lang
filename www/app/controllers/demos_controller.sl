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
end
