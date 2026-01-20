---
title: Built-in Functions
description: Reference for Soli's built-in functions
---

# Built-in Functions

Soli provides these built-in functions available in every program.

## I/O Functions

### print

Prints values to standard output.

```rust
print(value1, value2, ...)
```

**Parameters:** Any number of values of any type
**Returns:** `Void`

```rust
print("Hello");           // Hello
print("Value:", 42);      // Value: 42
print(1, 2, 3);           // 1 2 3
```

### input

Reads a line from standard input.

```rust
input(prompt?)
```

**Parameters:** Optional prompt string
**Returns:** `String` - the input line (without newline)

```rust
let name = input("Enter your name: ");
let value = input();  // No prompt
```

### barf

Writes content to a file. Automatically detects text vs binary mode based on the content type.

```rust
barf(path, content)
```

**Parameters:**
- `path`: File path as `String` (relative to current working directory)
- `content`: `String` for text mode, or `Array<Int>` for binary mode (byte values 0-255)

**Returns:** `Void`

```rust
// Text mode - write string to file
barf("config.txt", "Hello, World!");

// Binary mode - write byte array to file
let bytes = [72, 101, 108, 108, 111];  // "Hello" in ASCII
barf("data.bin", bytes);
```

### slurp

Reads content from a file. Returns text by default, or binary data as a byte array.

```rust
slurp(path)
slurp(path, mode)
```

**Parameters:**
- `path`: File path as `String` (relative to current working directory)
- `mode`: Optional mode string. `"binary"` for byte array, any other value or omit for text.

**Returns:** `String` for text mode, `Array<Int>` for binary mode

```rust
// Text mode (default)
let content = slurp("config.txt");
print(content);  // File contents as string

// Binary mode - read file as bytes
let data = slurp("image.png", "binary");
print(len(data));  // Number of bytes
```

### File I/O Example

A complete example showing file operations:

```rust
// Write text to file
barf("output.txt", "Hello from Soli!\n");

// Read it back
let content = slurp("output.txt");
print(content);

// Binary file copy
let original = slurp("photo.png", "binary");
barf("photo_copy.png", original);
print("Copied file with", len(original), "bytes");
```

:::note[Error Handling]
If a file doesn't exist or can't be read/written, the functions will throw an error that can be caught with a try/catch block.
:::

---

## Type Conversion

### str

Converts any value to a string.

```rust
str(value)
```

**Parameters:** Any value
**Returns:** `String`

```rust
str(42)        // "42"
str(3.14)      // "3.14"
str(true)      // "true"
str([1, 2])    // "[1, 2]"
```

### int

Converts a value to an integer.

```rust
int(value)
```

**Parameters:** `Int`, `Float`, `String`, or `Bool`
**Returns:** `Int`

```rust
int(3.7)       // 3
int("42")      // 42
int(true)      // 1
int(false)     // 0
```

### float

Converts a value to a floating-point number.

```rust
float(value)
```

**Parameters:** `Int`, `Float`, or `String`
**Returns:** `Float`

```rust
float(42)       // 42.0
float("3.14")   // 3.14
```

### type

Returns the type name of a value as a string.

```rust
type(value)
```

**Parameters:** Any value
**Returns:** `String`

```rust
type(42)         // "Int"
type(3.14)       // "Float"
type("hello")    // "String"
type(true)       // "Bool"
type([1, 2])     // "Array"
type(null)       // "Null"
```

---

## Array Functions

### len

Returns the length of an array or string.

```rust
len(collection)
```

**Parameters:** `Array` or `String`
**Returns:** `Int`

```rust
len([1, 2, 3])    // 3
len("hello")      // 5
len([])           // 0
```

### push

Adds an element to the end of an array (mutates the array).

```rust
push(array, element)
```

**Parameters:**
- `array`: The array to modify
- `element`: The value to add

**Returns:** `Void`

```rust
let arr = [1, 2, 3];
push(arr, 4);
print(arr);  // [1, 2, 3, 4]
```

### pop

Removes and returns the last element from an array.

```rust
pop(array)
```

**Parameters:** Non-empty array
**Returns:** The removed element

```rust
let arr = [1, 2, 3];
let last = pop(arr);
print(last);  // 3
print(arr);   // [1, 2]
```

### range

Creates an array of integers from start (inclusive) to end (exclusive).

```rust
range(start, end)
```

**Parameters:**
- `start`: Starting integer (inclusive)
- `end`: Ending integer (exclusive)

**Returns:** `Int[]`

```rust
range(0, 5)    // [0, 1, 2, 3, 4]
range(1, 4)    // [1, 2, 3]
range(5, 10)   // [5, 6, 7, 8, 9]
```

---

## Math Functions

### abs

Returns the absolute value of a number.

```rust
abs(number)
```

**Parameters:** `Int` or `Float`
**Returns:** Same type as input

```rust
abs(-5)      // 5
abs(3.14)    // 3.14
abs(-2.5)    // 2.5
```

### min

Returns the smaller of two values.

```rust
min(a, b)
```

**Parameters:** Two numbers (Int or Float)
**Returns:** Smaller value

```rust
min(3, 7)      // 3
min(2.5, 1.8)  // 1.8
min(-1, -5)    // -5
```

### max

Returns the larger of two values.

```rust
max(a, b)
```

**Parameters:** Two numbers (Int or Float)
**Returns:** Larger value

```rust
max(3, 7)      // 7
max(2.5, 1.8)  // 2.5
max(-1, -5)    // -1
```

### sqrt

Returns the square root of a number.

```rust
sqrt(number)
```

**Parameters:** `Int` or `Float`
**Returns:** `Float`

```rust
sqrt(16)     // 4.0
sqrt(2)      // 1.4142...
sqrt(0.25)   // 0.5
```

### pow

Raises a number to a power.

```rust
pow(base, exponent)
```

**Parameters:**
- `base`: Number to raise
- `exponent`: Power to raise to

**Returns:** `Int` (if both inputs are Int and exponent >= 0) or `Float`

```rust
pow(2, 3)      // 8
pow(2, 10)     // 1024
pow(2.0, 0.5)  // 1.4142... (square root)
pow(10, -2)    // 0.01
```

---

## Time Functions

### clock

Returns the current time in seconds since Unix epoch.

```rust
clock()
```

**Parameters:** None
**Returns:** `Float`

```rust
let start = clock();
// ... do some work ...
let elapsed = clock() - start;
print("Took " + str(elapsed) + " seconds");
```

---

## Hash Functions

### keys

Returns an array of all keys in a hash (in insertion order).

```rust
keys(hash)
```

**Parameters:** `Hash`
**Returns:** `Array`

```rust
let person = {"name" => "Alice", "age" => 30};
print(keys(person));  // [name, age]
```

### values

Returns an array of all values in a hash (in insertion order).

```rust
values(hash)
```

**Parameters:** `Hash`
**Returns:** `Array`

```rust
let person = {"name" => "Alice", "age" => 30};
print(values(person));  // [Alice, 30]
```

### has_key

Check if a key exists in a hash.

```rust
has_key(hash, key)
```

**Parameters:**
- `hash`: The hash to check
- `key`: The key to look for

**Returns:** `Bool`

```rust
let scores = {"Alice" => 95, "Bob" => 87};
print(has_key(scores, "Alice"));  // true
print(has_key(scores, "Carol"));  // false
```

### delete

Remove a key from a hash and return its value.

```rust
delete(hash, key)
```

**Parameters:**
- `hash`: The hash to modify
- `key`: The key to remove

**Returns:** The removed value, or `null` if key wasn't found

```rust
let hash = {"a" => 1, "b" => 2};
let val = delete(hash, "a");
print(val);   // 1
print(hash);  // {b => 2}
```

### merge

Combine two hashes into a new hash. The second hash's values win on key conflicts.

```rust
merge(hash1, hash2)
```

**Parameters:** Two hashes
**Returns:** `Hash` - a new hash with all entries

```rust
let h1 = {"a" => 1, "b" => 2};
let h2 = {"b" => 3, "c" => 4};
print(merge(h1, h2));  // {a => 1, b => 3, c => 4}
```

### entries

Get an array of `[key, value]` pairs from a hash.

```rust
entries(hash)
```

**Parameters:** `Hash`
**Returns:** `Array` of `[key, value]` arrays

```rust
let colors = {"red" => "#FF0000", "green" => "#00FF00"};
print(entries(colors));  // [[red, #FF0000], [green, #00FF00]]
```

### clear

Remove all entries from a hash or array (mutates in place).

```rust
clear(collection)
```

**Parameters:** `Hash` or `Array`
**Returns:** `Void`

```rust
let data = {"x" => 1, "y" => 2};
clear(data);
print(data);  // {}
```

---

## HTTP Functions

Soli includes built-in HTTP client functions for making web requests.

:::tip[Async by Default]
All HTTP functions run **asynchronously** in background threads and return `Future` values. Futures auto-resolve when their values are used (printed, indexed, passed to functions, etc.). This allows multiple requests to run in parallel automatically.
:::

### http_get

Makes a GET request and returns a Future that resolves to the response body.

```rust
http_get(url)
```

**Parameters:** URL as `String`
**Returns:** `Future<String>` - auto-resolves to response body

```rust
let response = http_get("https://api.example.com/data");
print(response);  // Auto-resolves and prints the body
```

### http_get_json

Makes a GET request and parses the response as JSON.

```rust
http_get_json(url)
```

**Parameters:** URL as `String`
**Returns:** `Future<Any>` - auto-resolves to parsed JSON (Hash, Array, or primitive)

```rust
let data = http_get_json("https://api.example.com/users");
print(data["name"]);  // Auto-resolves and accesses JSON properties
```

### http_post

Makes a POST request with a string or hash body.

```rust
http_post(url, body)
```

**Parameters:**
- `url`: URL as `String`
- `body`: `String` or `Hash` (hashes are JSON-encoded)

**Returns:** `Future<String>` - auto-resolves to response body

```rust
// String body
let resp = http_post("https://api.example.com/data", "Hello");

// Hash body (auto-serialized to JSON)
let resp = http_post("https://api.example.com/users", {"name": "Alice"});
print(resp);  // Auto-resolves when used
```

### http_post_json

Makes a POST request with a JSON body and parses the response.

```rust
http_post_json(url, data)
```

**Parameters:**
- `url`: URL as `String`
- `data`: Any JSON-serializable value

**Returns:** `Future<Any>` - auto-resolves to parsed JSON response

```rust
let user = {"name": "Alice", "email": "alice@example.com"};
let result = http_post_json("https://api.example.com/users", user);
print(result["id"]);  // Auto-resolves when accessed
```

### http_request

Generic HTTP request with full control over method, headers, and body.

```rust
http_request(method, url, headers?, body?)
```

**Parameters:**
- `method`: HTTP method (`"GET"`, `"POST"`, `"PUT"`, `"DELETE"`, `"PATCH"`, `"HEAD"`)
- `url`: URL as `String`
- `headers`: Optional `Hash` of header name/value pairs
- `body`: Optional request body

**Returns:** `Future<Hash>` - auto-resolves to response details:
- `status`: HTTP status code (Int)
- `status_text`: Status message (String)
- `headers`: Response headers (Hash)
- `body`: Response body (String)

```rust
let headers = {"Authorization": "Bearer token123"};
let result = http_request("GET", "https://api.example.com/me", headers);

print(result["status"]);       // 200 (auto-resolves on index)
print(result["body"]);         // Response body
print(result["headers"]);      // Response headers
```

### json_parse

Parses a JSON string into a Soli value.

```rust
json_parse(json_string)
```

**Parameters:** JSON `String`
**Returns:** Parsed value (Hash, Array, or primitive)

```rust
let json = '{"name": "Alice", "scores": [95, 87, 92]}';
let data = json_parse(json);
print(data["name"]);      // Alice
print(data["scores"][0]); // 95
```

### json_stringify

Converts a Soli value to a JSON string.

```rust
json_stringify(value)
```

**Parameters:** Any JSON-serializable value
**Returns:** `String` - JSON representation

```rust
let data = {"name": "Bob", "active": true, "tags": ["dev", "admin"]};
let json = json_stringify(data);
print(json);  // {"name":"Bob","active":true,"tags":["dev","admin"]}
```

### await

Explicitly waits for a Future to resolve and returns its value.

```rust
await(future)
```

**Parameters:** Any value (non-Futures pass through unchanged)
**Returns:** The resolved value

```rust
let future = http_get_json("https://api.example.com/data");
print(type(future));  // "Future" (before resolution)
let data = await(future);
print(type(data));    // "Hash" (after resolution)
```

:::note
Most operations auto-resolve Futures, so `await()` is only needed when you want explicit control over when resolution happens.
:::

### http_ok

Checks if a response status is in the 2xx (success) range.

```rust
http_ok(response)
```

**Parameters:** Response `Hash` (from `http_request`) or `Int` status code
**Returns:** `Bool`

```rust
let resp = http_request("GET", "https://api.example.com/data");
if http_ok(resp) {
    print("Success!");
}
```

### http_success

Alias for `http_ok()`. Checks if response status is 2xx.

```rust
http_success(response)
```

### http_redirect

Checks if a response status is in the 3xx (redirect) range.

```rust
http_redirect(response)
```

**Parameters:** Response `Hash` or `Int` status code
**Returns:** `Bool`

```rust
print(http_redirect(301));  // true
print(http_redirect(200));  // false
```

### http_client_error

Checks if a response status is in the 4xx (client error) range.

```rust
http_client_error(response)
```

**Parameters:** Response `Hash` or `Int` status code
**Returns:** `Bool`

```rust
let resp = http_request("GET", "https://api.example.com/missing");
if http_client_error(resp) {
    print("Not found or bad request");
}
```

### http_server_error

Checks if a response status is in the 5xx (server error) range.

```rust
http_server_error(response)
```

**Parameters:** Response `Hash` or `Int` status code
**Returns:** `Bool`

```rust
if http_server_error(resp) {
    print("Server is having issues");
}
```

---

## Parallel HTTP Requests

Since all HTTP functions run asynchronously, you can easily make parallel requests:

```rust
// Start all requests simultaneously
let users = http_get_json("https://api.example.com/users");
let posts = http_get_json("https://api.example.com/posts");
let comments = http_get_json("https://api.example.com/comments");

// All three requests run in parallel
// They auto-resolve when accessed
print("Users:", len(users));
print("Posts:", len(posts));
print("Comments:", len(comments));
```

This pattern is much faster than sequential requests because all HTTP calls execute concurrently.

---

## HTTP Server Functions

Soli includes built-in HTTP server functions for creating web servers with route-based request handling.

:::note[Single-threaded & Blocking]
The HTTP server runs on a single thread and processes requests sequentially. When `http_server_listen()` is called, it blocks until the server is terminated (e.g., Ctrl+C).
:::

### http_server_get

Registers a handler for GET requests on a specific path.

```rust
http_server_get(path, handler)
```

**Parameters:**
- `path`: URL pattern as `String` (supports `:param` for route parameters)
- `handler`: Function that takes a request Hash and returns a response Hash

**Returns:** `Void`

```rust
fn home_handler(req: Any) -> Any {
    return {"status": 200, "body": "Welcome!"};
}
http_server_get("/", home_handler);

// With route parameters
fn user_handler(req: Any) -> Any {
    let id = req["params"]["id"];
    return {"status": 200, "body": "User: " + id};
}
http_server_get("/users/:id", user_handler);
```

### http_server_post

Registers a handler for POST requests.

```rust
http_server_post(path, handler)
```

**Parameters:**
- `path`: URL pattern as `String`
- `handler`: Function that takes a request Hash and returns a response Hash

**Returns:** `Void`

```rust
fn create_user(req: Any) -> Any {
    let data = json_parse(req["body"]);
    // ... create user ...
    return {"status": 201, "body": json_stringify({"created": true})};
}
http_server_post("/users", create_user);
```

### http_server_put

Registers a handler for PUT requests.

```rust
http_server_put(path, handler)
```

**Parameters:**
- `path`: URL pattern as `String`
- `handler`: Function that takes a request Hash and returns a response Hash

**Returns:** `Void`

```rust
fn update_user(req: Any) -> Any {
    let id = req["params"]["id"];
    let data = json_parse(req["body"]);
    // ... update user ...
    return {"status": 200, "body": json_stringify({"updated": true})};
}
http_server_put("/users/:id", update_user);
```

### http_server_delete

Registers a handler for DELETE requests.

```rust
http_server_delete(path, handler)
```

**Parameters:**
- `path`: URL pattern as `String`
- `handler`: Function that takes a request Hash and returns a response Hash

**Returns:** `Void`

```rust
fn delete_user(req: Any) -> Any {
    let id = req["params"]["id"];
    // ... delete user ...
    return {"status": 200, "body": json_stringify({"deleted": true})};
}
http_server_delete("/users/:id", delete_user);
```

### http_server_route

Registers a handler for any HTTP method.

```rust
http_server_route(method, path, handler)
```

**Parameters:**
- `method`: HTTP method as `String` (`"GET"`, `"POST"`, `"PUT"`, `"DELETE"`, etc.)
- `path`: URL pattern as `String`
- `handler`: Function that takes a request Hash and returns a response Hash

**Returns:** `Void`

```rust
fn options_handler(req: Any) -> Any {
    return {
        "status": 200,
        "headers": {"Allow": "GET, POST, OPTIONS"},
        "body": ""
    };
}
http_server_route("OPTIONS", "/api", options_handler);
```

### http_server_listen

Starts the HTTP server on the specified port. This function blocks until the server is stopped.

```rust
http_server_listen(port)
```

**Parameters:**
- `port`: Port number as `Int`

**Returns:** Does not return (blocks indefinitely)

```rust
println("Starting server on port 3000...");
http_server_listen(3000);
// Code after this line will not execute until server stops
```

### Request Object

The request hash passed to handlers contains:

| Field | Type | Description |
|-------|------|-------------|
| `method` | String | HTTP method (GET, POST, etc.) |
| `path` | String | Request path (e.g., "/users/123") |
| `params` | Hash | Route parameters (e.g., `{"id": "123"}` for `/users/:id`) |
| `query` | Hash | Query string parameters (e.g., `{"page": "1"}` for `?page=1`) |
| `headers` | Hash | Request headers |
| `body` | String | Request body |

```rust
fn handler(req: Any) -> Any {
    print("Method:", req["method"]);
    print("Path:", req["path"]);
    print("Route params:", req["params"]);
    print("Query params:", req["query"]);
    print("Headers:", req["headers"]);
    print("Body:", req["body"]);

    return {"status": 200, "body": "OK"};
}
```

### Response Object

Handlers must return a response hash with these fields:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `status` | Int | Yes | HTTP status code (200, 404, etc.) |
| `headers` | Hash | No | Response headers |
| `body` | String | Yes | Response body |

```rust
fn json_response(req: Any) -> Any {
    return {
        "status": 200,
        "headers": {"Content-Type": "application/json"},
        "body": json_stringify({"message": "Hello!"})
    };
}
```

### Complete Server Example

```rust
// In-memory data store
let users = {};
let next_id = 1;

fn list_users(req: Any) -> Any {
    return {
        "status": 200,
        "headers": {"Content-Type": "application/json"},
        "body": json_stringify(values(users))
    };
}

fn get_user(req: Any) -> Any {
    let id = req["params"]["id"];
    if (has_key(users, id)) {
        return {
            "status": 200,
            "headers": {"Content-Type": "application/json"},
            "body": json_stringify(users[id])
        };
    } else {
        return {
            "status": 404,
            "body": json_stringify({"error": "User not found"})
        };
    }
}

fn create_user(req: Any) -> Any {
    let data = json_parse(req["body"]);
    let id = str(next_id);
    next_id = next_id + 1;

    users[id] = {"id": id, "name": data["name"]};

    return {
        "status": 201,
        "headers": {"Content-Type": "application/json"},
        "body": json_stringify(users[id])
    };
}

// Register routes
http_server_get("/users", list_users);
http_server_get("/users/:id", get_user);
http_server_post("/users", create_user);

// Start server
println("Server running on http://localhost:3000");
http_server_listen(3000);
```

Test with curl:
```bash
curl http://localhost:3000/users
curl -X POST -H "Content-Type: application/json" -d '{"name":"Alice"}' http://localhost:3000/users
curl http://localhost:3000/users/1
```

---

## Cryptographic Functions

Soli provides built-in functions for secure password hashing using Argon2, the winner of the Password Hashing Competition and the recommended algorithm for password storage.

### argon2_hash

Hashes a password using Argon2id (the recommended variant).

```rust
argon2_hash(password)
```

**Parameters:**
- `password`: Password as `String`

**Returns:** `String` - The hash in PHC string format (includes algorithm, parameters, salt, and hash)

```rust
let password = "my_secret_password";
let hash = argon2_hash(password);
print(hash);  // $argon2id$v=19$m=19456,t=2,p=1$...
```

:::tip[PHC Format]
The returned hash is in PHC (Password Hashing Competition) string format, which is self-describing and includes all parameters needed for verification. You can safely store this string in a database.
:::

### argon2_verify

Verifies a password against an Argon2 hash.

```rust
argon2_verify(password, hash)
```

**Parameters:**
- `password`: Password to verify as `String`
- `hash`: The hash string (from `argon2_hash`) as `String`

**Returns:** `Bool` - `true` if password matches, `false` otherwise

```rust
let password = "my_secret_password";
let hash = argon2_hash(password);

// Verify correct password
print(argon2_verify(password, hash));  // true

// Verify wrong password
print(argon2_verify("wrong_password", hash));  // false
```

### Password Hashing Example

A complete example for user authentication:

```rust
// Registration: hash and store the password
fn register_user(username: String, password: String) -> Any {
    let hash = argon2_hash(password);
    // Store username and hash in your database
    return {"username": username, "password_hash": hash};
}

// Login: verify the password
fn verify_login(password: String, stored_hash: String) -> Bool {
    return argon2_verify(password, stored_hash);
}

// Usage
let user = register_user("alice", "secure_password_123");
print("Stored hash:", user["password_hash"]);

// Later, verify login
if (verify_login("secure_password_123", user["password_hash"])) {
    print("Login successful!");
} else {
    print("Invalid password");
}
```

:::caution[Security Note]
- Never store passwords in plain text
- Always use `argon2_hash()` before storing passwords
- Use `argon2_verify()` to check passwords - never compare hashes directly
- Each call to `argon2_hash()` generates a unique salt, so the same password produces different hashes
:::

---

## SoliDB Functions

Soli includes built-in functions for connecting to [SoliDB](https://github.com/solisoft/solidb), a high-performance document database using MessagePack binary protocol.

### Solidb Class

The `Solidb` class provides a convenient interface to SoliDB with connection pooling and automatic reconnection.

```rust
let db = new Solidb(host, database)
```

**Parameters:**
- `host`: SoliDB server address (e.g., `"localhost:6745"`)
- `database`: Database name to use

**Returns:** A `Solidb` instance

```rust
// Connect to local SoliDB
let db = new Solidb("localhost:6745", "mydb");

// With authentication
db.auth("admin", "password123");

// Query the database
let users = db.query("FOR u IN users RETURN u");
```

### Instance Methods

All instance methods run asynchronously and return `Future` values that auto-resolve when used.

#### auth

Authenticate with the database. Credentials are stored and reused for subsequent operations.

```rust
db.auth(username, password)
```

**Parameters:**
- `username`: Username as `String`
- `password`: Password as `String`

**Returns:** `Future<String>` - resolves to `"Authenticated"`

```rust
db.auth("admin", "my_secret_password");
```

#### query

Execute a raw SDBQL query with optional bind variables.

```rust
db.query(sdbql, bind_vars?)
```

**Parameters:**
- `sdbql`: SDBQL query string as `String`
- `bind_vars`: Optional `Hash` of bind variables

**Returns:** `Future<String>` - JSON string of results

```rust
// Simple query
let users = db.query("FOR u IN users RETURN u");

// Query with bind variables
let results = db.query(
    "FOR u IN users FILTER u.age > @min_age AND u.city == @city RETURN u",
    {"min_age": 18, "city": "Paris"}
);
```

#### get

Retrieve a document by key.

```rust
db.get(collection, key)
```

**Parameters:**
- `collection`: Collection name as `String`
- `key`: Document key as `String`

**Returns:** `Future<String>` - JSON string of the document

```rust
let user = db.get("users", "user123");
print(user["name"]);
```

#### insert

Insert a new document.

```rust
db.insert(collection, key, document)
```

**Parameters:**
- `collection`: Collection name as `String`
- `key`: Document key as `String` (or `null` for auto-generated key)
- `document`: Document data as `Hash`

**Returns:** `Future<String>` - JSON string of inserted document

```rust
db.insert("users", "user456", {
    "name": "Alice",
    "email": "alice@example.com",
    "age": 30
});

// Auto-generate key
db.insert("users", null, {"name": "Bob", "score": 95});
```

#### update

Update an existing document (replaces the document).

```rust
db.update(collection, key, document)
```

**Parameters:**
- `collection`: Collection name as `String`
- `key`: Document key as `String`
- `document`: New document data as `Hash`

**Returns:** `Future<String>` - JSON string of updated document

```rust
db.update("users", "user123", {"name": "Alice Updated", "age": 31});
```

#### upsert

Update a document, merging with existing data if it exists.

```rust
db.upsert(collection, key, document)
```

**Parameters:**
- `collection`: Collection name as `String`
- `key`: Document key as `String`
- `document`: Document data as `Hash`

**Returns:** `Future<String>` - JSON string of upserted document

```rust
// If user123 exists, merges the data; otherwise creates new
db.upsert("users", "user123", {"age": 25, "city": "London"});
```

#### delete

Delete a document by key.

```rust
db.delete(collection, key)
```

**Parameters:**
- `collection`: Collection name as `String`
- `key`: Document key as `String`

**Returns:** `Future<String>` - resolves to `"OK"`

```rust
db.delete("users", "user123");
```

#### list

List all documents in a collection.

```rust
db.list(collection)
```

**Parameters:**
- `collection`: Collection name as `String`

**Returns:** `Future<String>` - JSON string of all documents

```rust
let all_users = db.list("users");
print("Found", len(all_users), "users");
```

#### explain

Explain the query execution plan for debugging.

```rust
db.explain(sdbql, bind_vars?)
```

**Parameters:**
- `sdbql`: SDBQL query string as `String`
- `bind_vars`: Optional `Hash` of bind variables

**Returns:** `Future<String>` - JSON string of execution plan

```rust
let plan = db.explain("FOR u IN users FILTER u.age > @age RETURN u", {"age": 18});
print(plan);
```

#### ping

Check the database connection.

```rust
db.ping()
```

**Parameters:** None
**Returns:** `Future<String>` - server timestamp

```rust
let ts = db.ping();
print("Server timestamp:", ts);
```

#### connected

Check if the database has been authenticated.

```rust
db.connected()
```

**Parameters:** None
**Returns:** `Bool`

```rust
if db.connected() {
    print("Authenticated and ready");
} else {
    print("Not authenticated");
}
```

### Global Functions

For advanced use cases, global functions are also available:

| Function | Parameters | Returns | Description |
|----------|-----------|---------|-------------|
| `solidb_connect(addr)` | String | Future&lt;String&gt; | Test connection |
| `solidb_ping(addr)` | String | Future&lt;String&gt; | Ping server |
| `solidb_auth(addr, db, user, pass)` | String, String, String, String | Future&lt;String&gt; | Authenticate |
| `solidb_query(addr, db, sdbql, vars?)` | String, String, String, Hash? | Future&lt;String&gt; | Execute query |

### Complete Example

```rust
// Create database connection
let db = new Solidb("localhost:6745", "myapp");

// Authenticate (optional)
db.auth("admin", "secret123");

// Create a user
db.insert("users", "user001", {
    "name": "Alice",
    "email": "alice@example.com",
    "tags": ["admin", "developer"]
});

// Query users
let admins = db.query("FOR u IN users FILTER 'admin' IN u.tags RETURN u");

// Get a specific user
let user = db.get("users", "user001");
print(user["name"]);  // Alice

// Update
db.update("users", "user001", {"name": "Alice Smith"});

// Check if authenticated
print("Connected:", db.connected());
```

:::note[Async by Default]
All SoliDB instance methods run asynchronously in background threads and return `Future` values. Futures auto-resolve when their values are used (printed, indexed, passed to functions, etc.).
:::

---

## Dotenv Functions

Load environment variables from `.env` files. This is useful for configuration management.

### dotenv

Load environment variables from a `.env` file. Returns the number of variables loaded.

```rust
dotenv(path?)
```

**Parameters:**
- `path`: Optional path to the `.env` file. Defaults to `".env"`

**Returns:** `Int` - Number of variables loaded, or error if file not found

```rust
// Load from default .env file
let count = dotenv();
print("Loaded", count, "variables");

// Load from custom path
dotenv("config/.env");
```

### dotenv!

Same as `dotenv()`, but does nothing if the file doesn't exist.

```rust
dotenv!(path?)
```

**Parameters:**
- `path`: Optional path to the `.env` file. Defaults to `".env"`

**Returns:** `Int` - Number of variables loaded (0 if file not found)

```rust
// Won't error if .env doesn't exist
let count = dotenv!();
```

### .env File Format

The `.env` file supports the following format:

```bash
# Comments start with #
DATABASE_URL=postgres://user:pass@localhost:5432/mydb
API_KEY=your-api-key-here
DEBUG=true
PORT=8080

# Empty values are allowed
OPTIONAL_VAR=

# Values can be quoted
NAME="John Doe"
MULTI_WORD_VAR="hello world"

# Shell-style expansion (using $)
EXPANDED_VAR=${OTHER_VAR}/path
```

### Example Usage

```rust
// Load environment variables
dotenv();

// Access loaded variables
let db_url = getenv("DATABASE_URL");
let api_key = getenv("API_KEY");

print("Database:", db_url);
print("API Key:", api_key);

// Set environment variables
setenv("DEBUG", "true");

// Check if a variable exists
if hasenv("SECRET_KEY") {
    print("Secret key is set");
}

// Remove environment variables
unsetenv("TEMP_VAR");
```

:::note[File Location]
The `.env` file path is relative to the current working directory where the Soli script is executed.
:::

---

## Environment Variable Functions

Access and modify environment variables.

### getenv

Get the value of an environment variable.

```rust
getenv(name)
```

**Parameters:**
- `name`: Variable name as `String`

**Returns:** `String` if variable exists, `Null` otherwise

```rust
let home = getenv("HOME");
let path = getenv("PATH");

if home == null {
    print("HOME not set");
} else {
    print("Home directory:", home);
}
```

### setenv

Set an environment variable.

```rust
setenv(name, value)
```

**Parameters:**
- `name`: Variable name as `String`
- `value`: Value as `String`

**Returns:** `Void`

```rust
setenv("DEBUG", "true");
setenv("API_URL", "https://api.example.com");
```

### unsetenv

Remove an environment variable.

```rust
unsetenv(name)
```

**Parameters:**
- `name`: Variable name as `String`

**Returns:** `Void`

```rust
unsetenv("TEMP_VAR");
```

### hasenv

Check if an environment variable exists.

```rust
hasenv(name)
```

**Parameters:**
- `name`: Variable name as `String`

**Returns:** `Bool`

```rust
if hasenv("SECRET_KEY") {
    print("Secret key is configured");
} else {
    print("Please set SECRET_KEY");
}
```

---

## Summary Table
---

---

## Summary Table

| Function | Parameters | Returns | Description |
|----------|-----------|---------|-------------|
| `print(...)` | Any values | Void | Print to stdout |
| `input(prompt?)` | Optional String | String | Read line from stdin |
| `str(x)` | Any | String | Convert to string |
| `int(x)` | Int/Float/String/Bool | Int | Convert to integer |
| `float(x)` | Int/Float/String | Float | Convert to float |
| `type(x)` | Any | String | Get type name |
| `len(x)` | Array/String/Hash | Int | Get length |
| `push(arr, val)` | Array, Any | Void | Add to array |
| `pop(arr)` | Array | Any | Remove from array |
| `range(a, b)` | Int, Int | Int[] | Create range array |
| `abs(x)` | Int/Float | Int/Float | Absolute value |
| `min(a, b)` | Numbers | Number | Minimum value |
| `max(a, b)` | Numbers | Number | Maximum value |
| `sqrt(x)` | Int/Float | Float | Square root |
| `pow(a, b)` | Numbers | Number | Exponentiation |
| `clock()` | None | Float | Current time |
| `keys(h)` | Hash | Array | Get all keys |
| `values(h)` | Hash | Array | Get all values |
| `has_key(h, k)` | Hash, Any | Bool | Check if key exists |
| `delete(h, k)` | Hash, Any | Any | Remove and return value |
| `merge(h1, h2)` | Hash, Hash | Hash | Combine hashes |
| `entries(h)` | Hash | Array | Get [key, value] pairs |
| `clear(c)` | Hash/Array | Void | Remove all entries |
| `http_get(url)` | String | Future&lt;String&gt; | GET request (async) |
| `http_get_json(url)` | String | Future&lt;Any&gt; | GET and parse JSON (async) |
| `http_post(url, body)` | String, String/Hash | Future&lt;String&gt; | POST request (async) |
| `http_post_json(url, data)` | String, Any | Future&lt;Any&gt; | POST JSON request (async) |
| `http_request(...)` | method, url, headers?, body? | Future&lt;Hash&gt; | Generic HTTP request (async) |
| `json_parse(s)` | String | Any | Parse JSON string |
| `json_stringify(v)` | Any | String | Convert to JSON |
| `await(v)` | Any | Any | Wait for Future to resolve |
| `http_ok(r)` | Hash/Int | Bool | Check if status is 2xx |
| `http_success(r)` | Hash/Int | Bool | Alias for http_ok |
| `http_redirect(r)` | Hash/Int | Bool | Check if status is 3xx |
| `http_client_error(r)` | Hash/Int | Bool | Check if status is 4xx |
| `http_server_error(r)` | Hash/Int | Bool | Check if status is 5xx |
| `http_server_get(path, handler)` | String, Function | Void | Register GET route handler |
| `http_server_post(path, handler)` | String, Function | Void | Register POST route handler |
| `http_server_put(path, handler)` | String, Function | Void | Register PUT route handler |
| `http_server_delete(path, handler)` | String, Function | Void | Register DELETE route handler |
| `http_server_route(method, path, handler)` | String, String, Function | Void | Register handler for any method |
| `http_server_listen(port)` | Int | - | Start HTTP server (blocking) |
| `argon2_hash(password)` | String | String | Hash password with Argon2id |
| `argon2_verify(password, hash)` | String, String | Bool | Verify password against hash |
| `barf(path, content)` | String, String/Array<Int> | Void | Write file (text or binary) |
| `slurp(path, mode?)` | String, String? | String/Array<Int> | Read file (text or binary) |
| `new Solidb(host, db)` | String, String | Instance | Create SoliDB client |
| `db.auth(user, pass)` | String, String | Future&lt;String&gt; | Authenticate (instance method) |
| `db.query(sdbql, vars?)` | String, Hash? | Future&lt;String&gt; | Execute SDBQL query |
| `db.get(coll, key)` | String, String | Future&lt;String&gt; | Get document |
| `db.insert(coll, key, doc)` | String, String/Null, Hash | Future&lt;String&gt; | Insert document |
| `db.update(coll, key, doc)` | String, String, Hash | Future&lt;String&gt; | Update document |
| `db.upsert(coll, key, doc)` | String, String, Hash | Future&lt;String&gt; | Upsert document |
| `db.delete(coll, key)` | String, String | Future&lt;String&gt; | Delete document |
| `db.list(coll)` | String | Future&lt;String&gt; | List documents |
| `db.explain(sdbql, vars?)` | String, Hash? | Future&lt;String&gt; | Explain query plan |
| `db.ping()` | - | Future&lt;String&gt; | Ping server |
| `db.connected()` | - | Bool | Check auth status |
| `dotenv(path?)` | String? | Int | Load .env file |
| `dotenv!(path?)` | String? | Int | Load .env file (silent) |
| `getenv(name)` | String | String/Null | Get environment variable |
| `setenv(name, value)` | String, String | Void | Set environment variable |
| `unsetenv(name)` | String | Void | Remove environment variable |
| `hasenv(name)` | String | Bool | Check if variable exists |
