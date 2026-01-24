// HTTP Client Example
// Demonstrates the built-in HTTP functions

// Simple GET request
print("=== Simple GET request ===");
let response = http_get("https://httpbin.org/get");
print("Response:", response);

// GET request with JSON parsing
print("\n=== GET JSON ===");
let data = http_get_json("https://httpbin.org/json");
print("Parsed JSON:", data);

// POST request with string body
print("\n=== POST with string body ===");
let post_response = http_post("https://httpbin.org/post", "Hello, World!");
print("Response:", post_response);

// POST request with JSON body (hash automatically serialized)
// Note: Both colon (:) and fat arrow (=>) syntax are supported for hashes
print("\n=== POST JSON ===");
let payload = {"name": "Alice", "age": 30, "active": true};
let json_response = http_post_json("https://httpbin.org/post", payload);
print("Response:", json_response);

// Generic HTTP request with custom headers
print("\n=== Generic HTTP request ===");
let headers = {"Authorization": "Bearer token123", "X-Custom-Header": "custom-value"};
let result = http_request("GET", "https://httpbin.org/headers", headers);
print("Status:", result["status"]);
print("Headers:", result["headers"]);
print("Body:", result["body"]);

// JSON utilities
print("\n=== JSON utilities ===");
let obj = {"items" => [1, 2, 3], "nested" => {"key" => "value"}};
let json_str = json_stringify(obj);
print("Stringified:", json_str);

let parsed = json_parse(json_str);
print("Parsed back:", parsed);
print("Nested key:", parsed["nested"]["key"]);

// Response status checks
print("\n=== Response status checks ===");
let response = http_request("GET", "https://httpbin.org/status/200");
print("Status:", response["status"]);
print("Is OK (2xx)?", http_ok(response));
print("Is Success?", http_success(response));
print("Is Redirect (3xx)?", http_redirect(response));
print("Is Client Error (4xx)?", http_client_error(response));
print("Is Server Error (5xx)?", http_server_error(response));

// Test with a 404 response
print("\n=== 404 Response ===");
let not_found = http_request("GET", "https://httpbin.org/status/404");
print("Status:", not_found["status"]);
print("Is OK?", http_ok(not_found));
print("Is Client Error?", http_client_error(not_found));

// Async/Parallel HTTP requests
// All HTTP functions automatically run in background threads
// and auto-resolve when the value is used
print("\n=== Parallel HTTP Requests ===");
let start = clock();

// These requests start immediately and run in parallel
let r1 = http_get("https://httpbin.org/delay/1");
let r2 = http_get("https://httpbin.org/delay/1");
let r3 = http_get("https://httpbin.org/delay/1");

// Check the type before using - it's a Future
print("Type of r1 (before use):", type(r1));

// Values auto-resolve when used (e.g., passed to len(), print(), or indexed)
print("Response lengths:", len(r1), len(r2), len(r3));

let elapsed = clock() - start;
print("Total time:", elapsed, "seconds");
print("(3 parallel 1-second requests complete in ~1-2 seconds, not 3)");

// You can also use await() to explicitly wait for a Future
print("\n=== Explicit await() ===");
let future = http_get_json("https://httpbin.org/json");
print("Created future:", type(future));
let result = await(future);
print("Slideshow title:", result["slideshow"]["title"]);
