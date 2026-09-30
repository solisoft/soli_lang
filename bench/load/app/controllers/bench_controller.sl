# Benchmark controller — one action per workload the `oha` harness measures.
#
#   json_one / json_many : pure JSON serialization (no DB, no view)
#   view                 : server-side template rendering (no DB)
#   db_one / db_list      : database reads
#   seed                 : populate the widgets collection for db/crud runs
#   crud_*               : full create / read / update / delete write lifecycle
#
# All API actions return JSON via `_json`. The view action renders a template.

class BenchController < Controller
    static {
        this.layout = "application"
    }

    # GET /bench/json — serialize a single representative object.
    def json_one
        return this._json(this._sample_widget(1))
    end

    # GET /bench/json_many?n=100 — serialize an array of N generated objects.
    def json_many
        let count = this._clamp(params["n"], 100)
        let items = (0..count).map do |index| this._sample_widget(index) end

        return this._json({ "count": count, "items": items })
    end

    # GET /bench/view?n=50 — render a template over N in-memory rows (no DB).
    def view
        let count = this._clamp(params["n"], 50)
        let items = (0..count).map do |index| this._sample_widget(index) end

        return render("bench/view", { "title": "View benchmark", "items": items })
    end

    # GET /bench/page?step=N — a realistic page: layout + nested partials + DB.
    # Complexity ramps over 4 steps; step N issues N DB queries and renders one
    # more partial-built section, so you can see the cost of a "real" page grow.
    def page
        let step = (params["step"] ?? "4").to_i()
        step = 1 if step < 1
        step = 4 if step > 4
        @step = step

        @products = Widget.limited(12).all()                  # step 1: 1 query
        @recent = []
        @recent = Widget.limited(6).all() if step >= 2        # step 2: +1 query
        @featured = []
        @featured = Widget.limited(3).all() if step >= 3      # step 3: +1 query
        @stats = {}
        if step >= 4                                          # step 4: +1 query
            let sample = Widget.limited(40).all()
            @stats = this._page_stats(sample)
        end

        @title = "Demo page · step " + str(step)

        return render("bench/page")
    end

    # Aggregate a sample of rows into the figures the stats strip shows.
    def _page_stats(rows)
        let count = rows.length()
        let total_price = 0
        for widget in rows
            total_price = total_price + (widget.price ?? 0)
        end
        let average = 0
        average = total_price / count if count > 0

        return { "rows": count, "avg_price": average }
    end

    # GET /bench/db_one — read a single row from the database.
    def db_one
        let widget = Widget.limited(1).all().first

        return this._json(this._widget_json(widget))
    end

    # GET /bench/db_list?n=100 — read N rows from the database (DB-side LIMIT).
    def db_list
        let count = this._clamp(params["n"], 100)
        let widgets = Widget.limited(count).all()
        let rows = widgets.map do |widget| this._widget_json(widget) end

        return this._json({ "count": rows.length(), "items": rows })
    end

    # GET /bench/seed?n=1000 — ensure at least N widgets exist before db/crud runs.
    def seed
        let target = this._clamp(params["n"], 1000)
        let existing = Widget.all().length()
        let created = 0
        let next_index = existing
        while existing + created < target
            Widget.create(this._sample_widget(next_index))
            created = created + 1
            next_index = next_index + 1
        end

        return this._json({ "target": target, "existing": existing, "created": created })
    end

    # POST /bench/results — store one benchmark run (posted by run_bench.sh).
    def save_results
        let payload = req["json"] ?? {}
        let run = BenchmarkRun.create({
            "stamp": payload["stamp"] ?? "unknown",
            "base_url": payload["base_url"],
            "duration": payload["duration"],
            "connections": payload["connections"],
            "peak_rps": payload["peak_rps"],
            "total_requests": payload["total_requests"],
            "workloads": payload["workloads"] ?? []
        })
        return this._json({ "errors": run._errors }, 422) if run._errors

        return this._json({ "id": run._key }, 201)
    end

    # POST /bench/crud — create a row, return its key.
    def crud_create
        let widget = Widget.create(this._permit(params))
        if widget._errors
            return this._json({ "errors": widget._errors }, 422)
        end

        return this._json({ "id": widget._key, "name": widget.name }, 201)
    end

    # GET /bench/crud/:id — read one row by key as JSON.
    def crud_show
        let widget = Widget.find_by("_key", params["id"])
        return this._json({ "error": "not found" }, 404) if widget.nil?

        return this._json(this._widget_json(widget))
    end

    # PUT /bench/crud/:id — update one row.
    def crud_update
        let widget = Widget.find_by("_key", params["id"])
        return this._json({ "error": "not found" }, 404) if widget.nil?

        widget.name = params["name"] unless params["name"].blank?
        widget.price = params["price"].to_i() unless params["price"].blank?
        widget.save()

        return this._json(this._widget_json(widget))
    end

    # DELETE /bench/crud — delete one arbitrary row (repeatable under load).
    # Returns 200 whether or not a row was present, so it can be hammered.
    def crud_delete_any
        let widget = Widget.limited(1).all().first
        return this._json({ "deleted": 0 }) if widget.nil?

        widget.delete()

        return this._json({ "deleted": 1, "id": widget._key })
    end

    # DELETE /bench/crud/:id — delete one row.
    def crud_delete
        let widget = Widget.find_by("_key", params["id"])
        return this._json({ "error": "not found" }, 404) if widget.nil?

        widget.delete()

        return this._json({ "deleted": params["id"] })
    end

    # --- helpers -------------------------------------------------------

    # Wrap any value as a JSON response with the given status (default 200).
    def _json(data, status = 200)
        return {
            "status": status,
            "headers": { "Content-Type": "application/json" },
            "body": data.to_json()
        }
    end

    # Parse a count param, default when blank, clamp to [1, MAX_ITEMS].
    def _clamp(raw, fallback)
        let count = fallback
        count = raw.to_i() unless raw.blank?
        return 1 if count < 1
        return this._max_items() if count > this._max_items()

        return count
    end

    # Upper bound on items per request (keeps a single benchmark request bounded).
    def _max_items
        return 1000
    end

    # Categories cycled through by the synthetic-data generator.
    def _categories
        return ["tools", "toys", "books", "food", "games"]
    end

    # A deterministic synthetic widget for the json/view workloads.
    def _sample_widget(index)
        let categories = this._categories()

        return {
            "id": index,
            "name": "Widget " + str(index),
            "category": categories[index % categories.length()],
            "price": 100 + (index * 7) % 9900,
            "active": index % 2 == 0,
            "tags": ["alpha", "beta", "gamma"]
        }
    end

    # Project a Widget record down to a plain JSON-friendly hash.
    def _widget_json(widget)
        return null if widget.nil?

        return {
            "id": widget._key,
            "name": widget.name,
            "category": widget.category,
            "price": widget.price,
            "active": widget.active
        }
    end

    # Mass-assignment whitelist for create/update.
    def _permit(raw)
        return {
            "name": raw["name"] ?? "Widget",
            "category": raw["category"] ?? "tools",
            "price": (raw["price"] ?? "100").to_i(),
            "active": true
        }
    end
end
