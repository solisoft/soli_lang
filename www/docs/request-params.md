# Unified Request Parameters

SoliLang provides a unified `req["all"]` field that merges parameters from all sources into a single, convenient hash. The same value is also exposed automatically as the global `params` variable, so handlers and views can reach it without touching `req`.

## Parameter Sources

When handling HTTP requests, parameters can come from multiple sources:

| Source | Description | Example |
|--------|-------------|---------|
| Route params | Parameters in the URL path | `/users/:id` → `{"id": "123"}` |
| Query params | URL query string | `?page=1&limit=10` → `{"page": "1", "limit": "10"}` |
| Body params | POST/PUT request body (JSON or form) | `{"name": "Alice"}` |

## The `all` Field

Use `req["all"]` to access all parameters unified:

```soli
# Request: POST /users/123/profile?name=alice&age=30
# Body: {"bio": "Developer", "age": "25"}

def update_profile
  # Unified access to all params
  all = req["all"];

  print("User ID:", all["id"]);       # "123" (from route)
  print("Name:", all["name"]);        # "alice" (query overrides route)
  print("Age:", all["age"]);          # "25" (JSON body overrides query)
  print("Bio:", all["bio"]);          # "Developer" (from JSON body)

  {"status": 200, "body": "Profile updated"}
end
```

## The `params` Global

For convenience, the server sets a global `params` variable to the same value as `req["all"]` before every request is dispatched. This lets controllers, plain handlers, and views access unified parameters without accepting or threading `req` through their code:

```soli
# Request: POST /users/123/profile?name=alice
# Body: {"bio": "Developer"}

def update_profile
  print("User ID:", params.id);       # "123" (from route)
  print("Name:", params.name);        # "alice" (from query)
  print("Bio:", params.bio);          # "Developer" (from JSON body)

  {"status": 200, "body": "Profile updated"}
end
```

`params` is refreshed before each request, so it is always in sync with the current handler's `req["all"]`. Dot access (`params.name`) and bracket access (`params["name"]`) both work.

## Nested Parameters

Bracket keys nest, Rack-style — in form bodies, multipart bodies, and query
strings alike:

| Wire format | `params` shape |
|-------------|----------------|
| `title=Hi` | `params["title"]` → `"Hi"` |
| `author[name]=Ada` | `params["author"]["name"]` → `"Ada"` |
| `tags[]=a&tags[]=b` | `params["tags"]` → `["a", "b"]` (submission order) |
| `items[][sku]=x&items[][sku]=y` | `params["items"]` → `[{"sku": "x"}, {"sku": "y"}]` |
| `items[0][sku]=x` | `params["items"]["0"]["sku"]` — numeric segments are hash keys |

Nesting is capped at 32 levels; malformed or over-deep bracket keys stay as
flat literal keys. Arrays come only from `[]` — numeric indices parse as
hash keys (`"0"`, `"1"`, …), which `permit` converts back to arrays.

### permit() — strong parameters

SoliDB is schemaless: an unfiltered `Model.create(params)` persists
**anything** a client posts. `permit(params, shape)` whitelists the shape
you expect and drops the rest:

```soli
permitted = permit(params, {
  "title": true,                            # scalar (containers dropped)
  "tags": [],                               # array of scalars
  "author": {"name": true, "email": true},  # nested hash
  "items": [{"sku": true, "qty": true}]     # array of hashes
})
Post.create(permitted)
```

Unlisted keys are dropped silently; a missing source (`permit(null, …)`)
filters to an empty hash. `[{...}]` also accepts a numeric-keyed hash
(`items[0][sku]` parsing) and returns an array of its filtered values.

`permit` is the primary mass-assignment filter; a model can additionally
declare [`attr_accessible`](/docs/database/models) as defense-in-depth. Both
are whitelists, so the result is their intersection — if a model uses both,
its `attr_accessible` list must include every top-level key controllers
permit (in `--dev`, drops log a `[WARN] attr_accessible …` line).

## The `cookies` Global

For convenience, the server also sets a global `cookies` variable to the same value as `req["cookies"]`. This hash contains all cookies parsed from the `Cookie` header, defaulting to `{}` when no cookies are present:

```soli
def show
  # Read cookies directly (no req prefix needed)
  theme = cookies["theme"] or "light";
  session_id = cookies.session_id;
end
```

Like `params`, the `cookies` global is refreshed before each request and is available in controllers, middleware, and views.

## Priority Order

When the same parameter exists in multiple sources, values are merged with this priority (highest wins):

1. **Body params** (JSON or form) - Highest priority
2. **Query params** - Middle priority
3. **Route params** - Lowest priority

```soli
# Request: PUT /items/42?status=active
# Body: {"status": "urgent", "quantity": "5"}

def update_item
  all = req["all"];

  # "status" appears in both query and body
  # Body wins: all["status"] = "urgent"
  print("Status:", all["status"]);

  # "id" only in route
  print("ID:", all["id"]);

  # "quantity" only in body
  print("Quantity:", all["quantity"]);

  {"status": 200, "body": "OK"}
end
```

## Still Available: Individual Sources

You can still access individual parameter sources separately:

```soli
def handler
  # Route parameters only
  id = req["params"]["id"];

  # Query parameters only
  page = req["query"]["page"];

  # JSON body only
  data = req["json"];

  # Form data only
  form = req["form"];

  # Or unified access
  all = req["all"];

  {"status": 200, "body": "OK"}
end
```

## Complete Example: Search with Pagination

```soli
def search
  all = req["all"];

  # Unified params allow flexible API design
  # Can pass filters via query, body, or both
  query = all["q"] or "";
  page = all["page"] or "1";
  limit = all["limit"] or "20";
  sort = all["sort"] or "relevance";

  # Use unified params for flexible filtering
  filters = {
    "query": query,
    "page": page,
    "limit": limit,
    "sort": sort,
    "category": all["category"],  # Optional, may be null
    "min_price": all["min_price"], # Optional
    "max_price": all["max_price"]  # Optional
  };

  # Execute search with filters
  results = execute_search(filters);

  {
    "status": 200,
    "body": json_stringify({
      "results": results,
      "page": page,
      "limit": limit
    })
  }
end
```

## API Reference

### Request Object Fields

| Field | Type | Description |
|-------|------|-------------|
| `method` | String | HTTP method (GET, POST, PUT, DELETE, etc.) |
| `path` | String | Request path |
| `params` | Hash | Route parameters |
| `query` | Hash | Query string parameters |
| `all` | Hash | Unified parameters (route + query + body) |
| `headers` | Hash | HTTP headers |
| `body` | String | Raw request body |
| `json` | Any/Null | Parsed JSON body |
| `form` | Hash/Null | Parsed form data |
| `files` | Array | Uploaded files |
| `cookies` | Hash | Parsed cookies from the `Cookie` header |

### Parameter Access Patterns

```soli
# Get single param from unified source
id = req["all"]["id"];

# Same thing via the global shorthand
id = params.id;

# Check if param exists
if params.page != null
  page = params.page;
end

# Get with default value
limit = params.limit or "20";

# Iterate over all params (hashes iterate via entries/keys/values)
for pair in entries(params)
  print(pair[0] + ": " + str(pair[1]));
end
```

## Benefits

1. **Flexibility**: Clients can send parameters via URL, query string, or body
2. **Simplicity**: Single access point for all parameters — use `req["all"]` or the global `params`
3. **Backward Compatible**: Individual sources (`req["params"]`, `req["query"]`, etc.) still work
4. **Intuitive Priority**: Body params naturally override URL params
