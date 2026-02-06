# Unified Request Parameters

SoliLang provides a unified `req["all"]` field that merges parameters from all sources into a single, convenient hash.

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
// Request: POST /users/123/profile?name=alice&age=30
// Body: {"bio": "Developer", "age": "25"}

fn update_profile(req: Any) -> Any {
    // Unified access to all params
    let all = req["all"];

    print("User ID:", all["id"]);       // "123" (from route)
    print("Name:", all["name"]);        // "alice" (query overrides route)
    print("Age:", all["age"]);          // "25" (JSON body overrides query)
    print("Bio:", all["bio"]);          // "Developer" (from JSON body)

    {"status": 200, "body": "Profile updated"}
}
```

## Priority Order

When the same parameter exists in multiple sources, values are merged with this priority (highest wins):

1. **Body params** (JSON or form) - Highest priority
2. **Query params** - Middle priority
3. **Route params** - Lowest priority

```soli
// Request: PUT /items/42?status=active
// Body: {"status": "urgent", "quantity": "5"}

fn update_item(req: Any) -> Any {
    let all = req["all"];

    // "status" appears in both query and body
    // Body wins: all["status"] = "urgent"
    print("Status:", all["status"]);

    // "id" only in route
    print("ID:", all["id"]);

    // "quantity" only in body
    print("Quantity:", all["quantity"]);

    {"status": 200, "body": "OK"}
}
```

## Still Available: Individual Sources

You can still access individual parameter sources separately:

```soli
fn handler(req: Any) -> Any {
    // Route parameters only
    let id = req["params"]["id"];

    // Query parameters only
    let page = req["query"]["page"];

    // JSON body only
    let data = req["json"];

    // Form data only
    let form = req["form"];

    // Or unified access
    let all = req["all"];

    {"status": 200, "body": "OK"}
}
```

## Complete Example: Search with Pagination

```soli
fn search(req: Any) -> Any {
    let all = req["all"];

    // Unified params allow flexible API design
    // Can pass filters via query, body, or both
    let query = all["q"] or "";
    let page = all["page"] or "1";
    let limit = all["limit"] or "20";
    let sort = all["sort"] or "relevance";

    // Use unified params for flexible filtering
    let filters = {
        "query": query,
        "page": page,
        "limit": limit,
        "sort": sort,
        "category": all["category"],  // Optional, may be null
        "min_price": all["min_price"], // Optional
        "max_price": all["max_price"]  // Optional
    };

    // Execute search with filters
    let results = execute_search(filters);

    {
        "status": 200,
        "body": json_stringify({
            "results": results,
            "page": page,
            "limit": limit
        })
    }
}
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

### Parameter Access Patterns

```soli
// Get single param from unified source
let id = req["all"]["id"];

// Check if param exists
if (req["all"]["page"] != null) {
    let page = req["all"]["page"];
}

// Get with default value
let limit = req["all"]["limit"] or "20";

// Iterate over all params
for (key, value) in req["all"] {
    print(key + ": " + value);
}
```

## Benefits

1. **Flexibility**: Clients can send parameters via URL, query string, or body
2. **Simplicity**: Single access point for all parameters
3. **Backward Compatible**: Individual sources (`req["params"]`, `req["query"]`, etc.) still work
4. **Intuitive Priority**: Body params naturally override URL params
