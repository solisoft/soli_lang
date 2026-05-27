# Building a CRUD Datatable with HTMx, Alpine, and Soli

> **See it live:** [`/demos/client-interactivity`](/demos/client-interactivity) — scroll to widget #10. The full source lives in `app/controllers/demos_controller.sl` and `app/views/demos/_users_table.html.slv`.

Datatables are the workhorse of any admin UI: a list of records you can search, sort, paginate, edit inline, toggle status on, change a role through, and delete with a confirm. In a typical SPA, that is dozens of components, a state store, and a network layer.

In Soli, it is one model, one controller, two partials, and a sprinkle of HTMx attributes. Total: about 250 lines, zero hand-written JavaScript.

<figure style="margin:1.5rem auto;max-width:1024px;">
  <img src="/images/blog/htmx-datatable.jpg" width="1024" height="576" alt="Modern admin CRUD datatable built with Soli + HTMx: searchable, sortable, paginated, with inline editing, role dropdowns, status toggles, and real-time row updates with no JavaScript framework." style="display:block;width:100%;height:auto;border-radius:12px;border:1px solid #30363d;background:#0b0d0f;">
  <figcaption style="text-align:center;color:#8b949e;font-size:0.875rem;margin-top:0.5rem;">A full-featured, production-ready admin datatable — powered by server-rendered HTML and HTMx.</figcaption>
</figure>

## What we are building

- **Search** — debounced, hits the server, swaps just the table
- **Sort** — clickable headers, asc/desc toggle, keeps the current query and page
- **Pagination** — server-rendered links, also via `outerHTML` swap
- **Inline edit** — click a name or email, edit, submit, replace just that row
- **Role select** — `<select>` posts on change, replaces the row
- **Status toggle** — click the badge, server flips Active/Inactive
- **Delete** — `hx-confirm` prompt, removes the row with a transition
- **Add user** — modal-loaded form, posts and refreshes the table
- **Toast** — every save triggers an `HX-Trigger` for a success toast

## The model

```soli
class DemoUser < Model
  validates("name", { "presence": true })
  validates("email", { "presence": true, "uniqueness": true })
end
```

That is it. `name`, `email`, `role`, `status`, `last_login` are stored as plain document fields. `Model` gives us `find`, `where`, `order`, `paginate`, `create`, `update`, and `delete` out of the box.

## The controller: one helper, five actions

The trick that keeps the controller flat is a single private helper that knows how to run the query — search + sort + paginate — and shape the result. Every action that renders the table reuses it.

```soli
class DemosController < Controller
  def _users_result(search_query, sort_column, sort_direction, page, per_page)
    valid_sort_keys = ["name", "email", "role", "status", "last_login"]
    sort_column = "name" unless valid_sort_keys.contains(sort_column)
    sort_direction = "asc" unless sort_direction == "desc"

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
    pagination = result["pagination"]
    records = result["records"]

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
end
```

Two things to notice. First, sort key validation is a whitelist — never trust raw `?sort=` values, they end up in the AQL `SORT` clause. Second, `.paginate({"page": page, "per": per_page})` is a builtin on the query chain. It returns `{"records": [...], "pagination": {"page", "per", "total", "total_pages"}}` in a single round trip — one count query, one slice query — so we never load the whole collection just to slice it.

### Listing

```soli
def users
  sort_column    = params["sort"] ?? "name"
  sort_direction = params["dir"] ?? "asc"
  search_query   = params["q"] ?? ""
  result = this._users_result(
    search_query, sort_column, sort_direction,
    int(params["page"] ?? "1"), 10
  )
  render("demos/_users_table", result.merge({
    "sort": sort_column,
    "dir": sort_direction,
    "q": search_query
  }), {"layout": false})
end
```

The endpoint returns a partial — `{"layout": false}` disables the application layout. HTMx swaps just the table fragment into the page.

### Inline update

```soli
def user_update
  user = DemoUser.find(params["id"] ?? "")
  field = params["field"] ?? "name"
  attrs = {}
  if field == "status"
    attrs["status"] = user["status"] == "Active" ? "Inactive" : "Active"
  else
    attrs[field] = params["value"] ?? ""
  end

  user.update(attrs)
  if user._errors
    # Validation failed (e.g. uniqueness on email). Re-render the row from
    # the persisted DB state and surface the first error as a toast.
    original = DemoUser.find(params["id"] ?? "")
    response = render("demos/_user_row", {"user": original}, {"layout": false})
    first = user._errors[0] ?? {}
    response["headers"]["HX-Trigger"] = json_stringify({
      "soli-toast": {"kind": "error", "message": first["message"] ?? "Could not save changes."}
    })
    return response
  end

  response = render("demos/_user_row", {"user": user}, {"layout": false})
  message = match field {
    "name"   => "Name updated to \"#{user["name"]}\".",
    "email"  => "Email updated to \"#{user["email"]}\".",
    "role"   => "Role changed to \"#{user["role"]}\".",
    "status" => "User is now #{user["status"]}.",
    _        => "Saved."
  }
  response["headers"]["HX-Trigger"] = json_stringify({
    "soli-toast": {"kind": "success", "message": message}
  })
  response
end
```

One endpoint handles every inline mutation. The field being changed comes from `hx-vals='{"field": "name"}'` set on the form. The response is just the new `<tr>`, plus an `HX-Trigger` header that fires a client-side event picked up by the global toast stack.

### Validation failures are also toasts

`user.update(attrs)` does not raise on validation failure — it sets `user._errors` to an array of `{field, message}` and leaves the record unsaved. The branch above turns that into an `error`-kind toast and re-renders the row from the persisted DB state, so the table snaps back to the truth and the user sees *why*. Try editing a row's email to one that already exists on widget #10 — the toast will read `"email has already been taken"` and the cell will revert.

This is the nicest piece of the pattern: validations declared on the model (`validates("email", { "uniqueness": true })`) propagate all the way out to a toast, with the controller doing nothing field-specific.

## The partials

### `_users_table.html.slv` — the shell

The table renders its own `id` on the wrapper div, so the search input, sort links, and pagination links can all target it with `hx-target="#users-table-container"` and `hx-swap="outerHTML"`. Each swap returns a fresh shell.

```soli
<div id="users-table-container">
  <table class="...">
    <thead>
      <tr>
        <% for col in [["name", "Name"], ["email", "Email"], ["role", "Role"]] %>
          <th>
            <a hx-get="/demos/api/users?sort=<%= col[0] %>&dir=<%= sort == col[0] && dir == "asc" ? "desc" : "asc" %>&q=<%= q %>&page=1"
               hx-target="#users-table-container"
               hx-swap="outerHTML"><%= col[1] %></a>
          </th>
        <% end %>
      </tr>
    </thead>
    <tbody>
      <% for user in users %>
        <%- partial("demos/user_row", {"user": user}) %>
      <% end %>
    </tbody>
  </table>
</div>
```

The "sort indicator" (the little arrow) is just `sort == col && dir == "asc"`. State lives in the URL, not in the DOM.

### `_user_row.html.slv` — Alpine + HTMx per row

Each row is its own Alpine island. We keep two pieces of state per editable field: `editingName` (mode flag) and `nameValue` (the live input). On first render, `x-init` reads the canonical value from `data-name` on the `<tr>`. When the form posts and the row is swapped, Alpine re-runs `x-init` on the fresh DOM — so we never need to sync state across the swap manually.

```html
<tr id="user-row-<%= user["_key"] %>"
    x-data="{ editingName: false, nameValue: '', nameOriginal: '' }"
    x-init="nameValue = $el.dataset.name; nameOriginal = $el.dataset.name"
    data-name="<%= user["name"] %>">
  <td>
    <span x-show="!editingName"
          @click="editingName = true; $nextTick(() => $refs.nameInput.focus())"
          x-text="nameValue"></span>
    <span x-show="!editingName" class="text-xs text-gray-600">click</span>

    <form x-show="editingName" x-cloak
          hx-patch="/demos/api/user/<%= user["_key"] %>"
          hx-vals='{"field": "name"}'
          hx-target="#user-row-<%= user["_key"] %>"
          hx-swap="outerHTML"
          @submit="editingName = false"
          @keydown.escape="editingName = false; nameValue = nameOriginal">
      <input x-ref="nameInput" x-model="nameValue" name="value" />
      <button type="submit">save</button>
    </form>
  </td>
</tr>
```

A few things worth pointing out:

- **`hx-vals` carries metadata, the form carries the value.** The input is `name="value"` and the discriminator is `field`. The controller sees both in the `params` global.
- **`@submit="editingName = false"` does not interfere with HTMx.** Alpine's `@submit` is a plain listener that does not prevent default; HTMx still intercepts the submit and sends the AJAX request with the form data.
- **Escape rolls back the input.** Saving the original in `nameOriginal` on init means Esc can restore the previous value without a server round-trip.

The "Active/Inactive" badge is the same pattern with zero inputs — the click is the value:

```html
<span hx-patch="/demos/api/user/<%= user["_key"] %>"
      hx-vals='{"field": "status"}'
      hx-target="#user-row-<%= user["_key"] %>"
      hx-swap="outerHTML">
  <%= user["status"] %>
</span>
```

The server inspects `field == "status"` and flips the value.

## Add user: modal + HTMx-loaded form

The form HTML is not in the page on first load. The "+ Add User" button opens an Alpine-managed modal and asks HTMx to fetch the form on demand:

```html
<button @click="addOpen = true;
               $nextTick(() => htmx.ajax('GET', '/demos/api/user-form',
                                         { target: $refs.addBody, swap: 'innerHTML' }))">
  + Add User
</button>
```

The form posts to `/demos/api/users`, targets the table container with `outerHTML`, and the controller emits a compound `HX-Trigger`:

```soli
response["headers"]["HX-Trigger"] = json_stringify({
  "soli-toast": {"kind": "success", "message": "User \"#{user["name"]}\" added."},
  "soli-add-user-close": true
})
```

Two events in one header. The toast stack listens for `soli-toast`, the modal wrapper listens for `soli-add-user-close` to flip its `addOpen` flag back to `false`. The frontend never has to care about the response body — the side effects are signaled out-of-band.

## The toast stack

A single Alpine component at the top of the page receives toast events from anywhere on the server:

```html
<div x-data="toastStack()" @soli-toast.window="add($event.detail)"
     class="fixed top-6 right-6 ...">
  <template x-for="t in toasts" :key="t.id">
    <div :class="t.kind === 'success' ? 'bg-emerald-500/15' : ...">
      <p x-text="t.message"></p>
    </div>
  </template>
</div>
```

Any controller that returns `HX-Trigger: {"soli-toast": {"kind": "...", "message": "..."}}` gets a free toast — no per-form wiring, no client-side state.

## Why this pattern works

The whole datatable boils down to four moves:

1. **State lives in the URL** (search query, sort, page). Hitting refresh, sharing the link, or hitting back all work for free.
2. **Each endpoint returns the smallest fragment that needs updating** — a row for inline edits, the whole table for search/sort/pagination/add.
3. **Alpine handles purely-visual state** (which form is open, which field is being edited). It never owns data.
4. **`HX-Trigger` decouples side effects from response bodies.** Toasts, modal closing, focus management — all one-liners on the server.

You do not need a frontend framework to ship a fast, polished admin UI. You need a model, a controller, two partials, and HTMx + Alpine as the seam between them.

## See also

- [Client Interactivity](/docs/core-concepts/client-interactivity) — the full reference for HTMx + Alpine patterns in Soli
- [Query Builder](/docs/database/query-builder) — `.where`, `.order`, `.paginate`, and the chainable model API
- [Validations](/docs/database/validations) — `presence`, `uniqueness`, and the rest of the rule set used on `DemoUser`
