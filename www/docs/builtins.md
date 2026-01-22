# Built-in Functions Reference

Soli provides a comprehensive set of built-in functions for common programming tasks. This reference documents all available functions organized by category.

---

## Core Functions

### I/O Functions

#### print(value)

Prints a value to standard output without a newline.

**Parameters:**
- `value` (Any) - The value to print

**Returns:** null

**Example:**
```soli
print("Hello")
print(" World")  // Output: Hello World
```

#### println(value)

Prints a value to standard output with a newline.

**Parameters:**
- `value` (Any) - The value to print

**Returns:** null

**Example:**
```soli
println("Hello World")
println(42)
```

#### input(prompt?)

Reads a line of input from the user.

**Parameters:**
- `prompt` (String, optional) - Prompt to display before reading input

**Returns:** String - The user's input

**Example:**
```soli
let name = input("Enter your name: ")
println("Hello, " + name)
```

---

### Type Functions

#### type(value)

Returns the type name of a value as a string.

**Parameters:**
- `value` (Any) - The value to check

**Returns:** String - One of: "null", "bool", "int", "float", "string", "array", "hash", "function", "class", "instance"

**Example:**
```soli
type(42)        // "int"
type("hello")   // "string"
type([1, 2, 3]) // "array"
type(null)      // "null"
```

#### str(value)

Converts a value to a string.

**Parameters:**
- `value` (Any) - The value to convert

**Returns:** String

**Example:**
```soli
str(42)       // "42"
str(3.14)     // "3.14"
str(true)     // "true"
str([1, 2])   // "[1, 2]"
```

#### int(value)

Converts a value to an integer.

**Parameters:**
- `value` (Any) - The value to convert (string or float)

**Returns:** Int - The integer value, or error if conversion fails

**Example:**
```soli
int("42")     // 42
int(3.7)      // 3
int("3.14")   // 3
```

#### float(value)

Converts a value to a float.

**Parameters:**
- `value` (Any) - The value to convert (string or int)

**Returns:** Float

**Example:**
```soli
float("3.14")  // 3.14
float(42)      // 42.0
```

#### len(value)

Returns the length of a string, array, or hash.

**Parameters:**
- `value` (String|Array|Hash) - The collection to measure

**Returns:** Int - The number of elements/characters

**Example:**
```soli
len("hello")      // 5
len([1, 2, 3])    // 3
len({"a": 1})     // 1
```

---

### Array Functions

#### push(array, value)

Adds a value to the end of an array (mutates the array).

**Parameters:**
- `array` (Array) - The array to modify
- `value` (Any) - The value to add

**Returns:** Array - The modified array

**Example:**
```soli
let arr = [1, 2]
push(arr, 3)
println(arr)  // [1, 2, 3]
```

#### pop(array)

Removes and returns the last element from an array.

**Parameters:**
- `array` (Array) - The array to modify

**Returns:** Any - The removed element, or null if empty

**Example:**
```soli
let arr = [1, 2, 3]
let last = pop(arr)
println(last)  // 3
println(arr)   // [1, 2]
```

#### range(start, end, step?)

Creates an array of numbers from start to end (exclusive).

**Parameters:**
- `start` (Int) - Starting value (inclusive)
- `end` (Int) - Ending value (exclusive)
- `step` (Int, optional) - Step increment (default: 1)

**Returns:** Array - Array of integers

**Example:**
```soli
range(0, 5)      // [0, 1, 2, 3, 4]
range(1, 10, 2)  // [1, 3, 5, 7, 9]
range(5, 0, -1)  // [5, 4, 3, 2, 1]
```

---

### Hash Functions

#### keys(hash)

Returns an array of all keys in a hash.

**Parameters:**
- `hash` (Hash) - The hash to get keys from

**Returns:** Array - Array of keys

**Example:**
```soli
let h = {"name": "Alice", "age": 30}
keys(h)  // ["name", "age"]
```

#### values(hash)

Returns an array of all values in a hash.

**Parameters:**
- `hash` (Hash) - The hash to get values from

**Returns:** Array - Array of values

**Example:**
```soli
let h = {"name": "Alice", "age": 30}
values(h)  // ["Alice", 30]
```

#### has_key(hash, key)

Checks if a hash contains a specific key.

**Parameters:**
- `hash` (Hash) - The hash to search
- `key` (Any) - The key to look for

**Returns:** Bool

**Example:**
```soli
let h = {"name": "Alice"}
has_key(h, "name")  // true
has_key(h, "age")   // false
```

#### delete(hash, key)

Removes a key-value pair from a hash.

**Parameters:**
- `hash` (Hash) - The hash to modify
- `key` (Any) - The key to remove

**Returns:** Any - The removed value, or null if not found

**Example:**
```soli
let h = {"name": "Alice", "age": 30}
delete(h, "age")
println(h)  // {"name": "Alice"}
```

#### merge(hash1, hash2)

Merges two hashes into a new hash.

**Parameters:**
- `hash1` (Hash) - The first hash
- `hash2` (Hash) - The second hash (values override hash1)

**Returns:** Hash - A new merged hash

**Example:**
```soli
let a = {"x": 1, "y": 2}
let b = {"y": 3, "z": 4}
merge(a, b)  // {"x": 1, "y": 3, "z": 4}
```

#### entries(hash)

Returns an array of [key, value] pairs.

**Parameters:**
- `hash` (Hash) - The hash to convert

**Returns:** Array - Array of [key, value] arrays

**Example:**
```soli
let h = {"a": 1, "b": 2}
entries(h)  // [["a", 1], ["b", 2]]
```

#### from_entries(array)

Creates a hash from an array of [key, value] pairs.

**Parameters:**
- `array` (Array) - Array of [key, value] arrays

**Returns:** Hash

**Example:**
```soli
from_entries([["a", 1], ["b", 2]])  // {"a": 1, "b": 2}
```

#### clear(hash)

Removes all entries from a hash.

**Parameters:**
- `hash` (Hash) - The hash to clear

**Returns:** null

**Example:**
```soli
let h = {"a": 1, "b": 2}
clear(h)
println(h)  // {}
```

---

### String Functions

#### split(string, separator)

Splits a string into an array by a separator.

**Parameters:**
- `string` (String) - The string to split
- `separator` (String) - The delimiter

**Returns:** Array - Array of substrings

**Example:**
```soli
split("a,b,c", ",")       // ["a", "b", "c"]
split("hello world", " ") // ["hello", "world"]
```

#### join(array, separator)

Joins an array into a string with a separator.

**Parameters:**
- `array` (Array) - The array to join
- `separator` (String) - The delimiter

**Returns:** String

**Example:**
```soli
join(["a", "b", "c"], ",")  // "a,b,c"
join([1, 2, 3], "-")        // "1-2-3"
```

#### contains(string, substring)

Checks if a string contains a substring.

**Parameters:**
- `string` (String) - The string to search in
- `substring` (String) - The string to find

**Returns:** Bool

**Example:**
```soli
contains("hello world", "world")  // true
contains("hello", "xyz")          // false
```

#### index_of(string, substring)

Finds the position of a substring.

**Parameters:**
- `string` (String) - The string to search in
- `substring` (String) - The string to find

**Returns:** Int - Index of first occurrence, or -1 if not found

**Example:**
```soli
index_of("hello", "ll")   // 2
index_of("hello", "xyz")  // -1
```

#### substring(string, start, end?)

Extracts a portion of a string.

**Parameters:**
- `string` (String) - The source string
- `start` (Int) - Starting index (inclusive)
- `end` (Int, optional) - Ending index (exclusive)

**Returns:** String

**Example:**
```soli
substring("hello", 1, 4)  // "ell"
substring("hello", 2)     // "llo"
```

#### upcase(string)

Converts a string to uppercase.

**Parameters:**
- `string` (String) - The string to convert

**Returns:** String

**Example:**
```soli
upcase("hello")  // "HELLO"
```

#### downcase(string)

Converts a string to lowercase.

**Parameters:**
- `string` (String) - The string to convert

**Returns:** String

**Example:**
```soli
downcase("HELLO")  // "hello"
```

#### trim(string)

Removes whitespace from both ends of a string.

**Parameters:**
- `string` (String) - The string to trim

**Returns:** String

**Example:**
```soli
trim("  hello  ")  // "hello"
```

#### html_escape(string)

Escapes HTML special characters.

**Parameters:**
- `string` (String) - The string to escape

**Returns:** String

**Example:**
```soli
html_escape("<script>alert('xss')</script>")
// "&lt;script&gt;alert('xss')&lt;/script&gt;"
```

#### html_unescape(string)

Unescapes HTML entities.

**Parameters:**
- `string` (String) - The string to unescape

**Returns:** String

**Example:**
```soli
html_unescape("&lt;p&gt;")  // "<p>"
```

#### sanitize_html(string)

Removes potentially dangerous HTML tags and attributes.

**Parameters:**
- `string` (String) - The HTML to sanitize

**Returns:** String - Safe HTML

**Example:**
```soli
sanitize_html("<p onclick='evil()'>Hello</p>")
// "<p>Hello</p>"
```

---

### File I/O Functions

#### slurp(path)

Reads the entire contents of a file.

**Parameters:**
- `path` (String) - Path to the file

**Returns:** String - File contents, or error on failure

**Example:**
```soli
let content = slurp("config.json")
println(content)
```

#### barf(path, content)

Writes content to a file (overwrites existing).

**Parameters:**
- `path` (String) - Path to the file
- `content` (String) - Content to write

**Returns:** null

**Example:**
```soli
barf("output.txt", "Hello, World!")
```

---

### Math Functions

#### abs(number)

Returns the absolute value of a number.

**Parameters:**
- `number` (Int|Float) - The number

**Returns:** Int|Float

**Example:**
```soli
abs(-5)    // 5
abs(-3.14) // 3.14
```

#### min(a, b)

Returns the smaller of two numbers.

**Parameters:**
- `a` (Int|Float) - First number
- `b` (Int|Float) - Second number

**Returns:** Int|Float

**Example:**
```soli
min(3, 7)  // 3
min(5.5, 2.2)  // 2.2
```

#### max(a, b)

Returns the larger of two numbers.

**Parameters:**
- `a` (Int|Float) - First number
- `b` (Int|Float) - Second number

**Returns:** Int|Float

**Example:**
```soli
max(3, 7)  // 7
```

#### sqrt(number)

Returns the square root of a number.

**Parameters:**
- `number` (Int|Float) - The number (must be non-negative)

**Returns:** Float

**Example:**
```soli
sqrt(16)  // 4.0
sqrt(2)   // 1.4142135623730951
```

#### pow(base, exponent)

Returns base raised to the power of exponent.

**Parameters:**
- `base` (Int|Float) - The base
- `exponent` (Int|Float) - The exponent

**Returns:** Float

**Example:**
```soli
pow(2, 3)   // 8.0
pow(10, -2) // 0.01
```

#### clock()

Returns the current Unix timestamp as a float with sub-second precision.

**Returns:** Float - Unix timestamp

**Example:**
```soli
let start = clock()
// ... do work ...
let elapsed = clock() - start
println("Took " + str(elapsed) + " seconds")
```

---

## HTTP Functions

### http_get(url, options?)

Performs an HTTP GET request.

**Parameters:**
- `url` (String) - The URL to fetch
- `options` (Hash, optional) - Request options
  - `headers` (Hash) - Custom headers

**Returns:** Hash - `{ "status": Int, "body": String, "headers": Hash }`

**Example:**
```soli
let response = http_get("https://api.example.com/data")
if response["status"] == 200 {
    println(response["body"])
}
```

### http_post(url, body, options?)

Performs an HTTP POST request.

**Parameters:**
- `url` (String) - The URL to post to
- `body` (String) - The request body
- `options` (Hash, optional) - Request options

**Returns:** Hash - `{ "status": Int, "body": String, "headers": Hash }`

**Example:**
```soli
let response = http_post(
    "https://api.example.com/users",
    "name=Alice&email=alice@example.com",
    { "headers": { "Content-Type": "application/x-www-form-urlencoded" } }
)
```

### http_post_json(url, data, options?)

Performs an HTTP POST request with JSON body.

**Parameters:**
- `url` (String) - The URL to post to
- `data` (Hash|Array) - Data to serialize as JSON
- `options` (Hash, optional) - Additional options

**Returns:** Hash - Response with parsed JSON body if applicable

**Example:**
```soli
let response = http_post_json(
    "https://api.example.com/users",
    { "name": "Alice", "email": "alice@example.com" }
)
```

### http_get_json(url, options?)

Performs an HTTP GET request and parses JSON response.

**Parameters:**
- `url` (String) - The URL to fetch
- `options` (Hash, optional) - Request options

**Returns:** Hash - Response with parsed JSON body

**Example:**
```soli
let data = http_get_json("https://api.example.com/users/1")
println(data["body"]["name"])
```

### http_request(method, url, options?)

Performs a custom HTTP request.

**Parameters:**
- `method` (String) - HTTP method (GET, POST, PUT, PATCH, DELETE, etc.)
- `url` (String) - The URL
- `options` (Hash, optional) - Request options

**Returns:** Hash - Response object

**Example:**
```soli
let response = http_request("DELETE", "https://api.example.com/users/1")
```

### HTTP Status Helpers

#### http_ok(response)

Checks if response status is 200.

**Example:**
```soli
if http_ok(response) {
    println("Success!")
}
```

#### http_success(response)

Checks if response status is 2xx.

#### http_redirect(response)

Checks if response status is 3xx.

#### http_client_error(response)

Checks if response status is 4xx.

#### http_server_error(response)

Checks if response status is 5xx.

### http_get_all(urls)

Performs multiple GET requests in parallel.

**Parameters:**
- `urls` (Array) - Array of URLs to fetch

**Returns:** Array - Array of response objects

**Example:**
```soli
let responses = http_get_all([
    "https://api.example.com/users",
    "https://api.example.com/posts"
])
```

### http_parallel(requests)

Performs multiple custom requests in parallel.

**Parameters:**
- `requests` (Array) - Array of request hashes with `method`, `url`, and optional `options`

**Returns:** Array - Array of response objects

**Example:**
```soli
let responses = http_parallel([
    { "method": "GET", "url": "https://api.example.com/users" },
    { "method": "POST", "url": "https://api.example.com/logs", "body": "{}" }
])
```

---

## JSON Functions

### json_parse(string)

Parses a JSON string into a Soli value.

**Parameters:**
- `string` (String) - JSON string to parse

**Returns:** Any - Parsed value (Hash, Array, String, Int, Float, Bool, or null)

**Example:**
```soli
let data = json_parse('{"name": "Alice", "age": 30}')
println(data["name"])  // Alice
```

### json_stringify(value)

Converts a Soli value to a JSON string.

**Parameters:**
- `value` (Any) - Value to serialize

**Returns:** String - JSON representation

**Example:**
```soli
let json = json_stringify({ "name": "Alice", "scores": [95, 87, 92] })
println(json)  // {"name":"Alice","scores":[95,87,92]}
```

---

## Cryptography Functions

### Password Hashing

#### argon2_hash(password)

Hashes a password using Argon2id (recommended).

**Parameters:**
- `password` (String) - The password to hash

**Returns:** String - The hash string

**Example:**
```soli
let hash = argon2_hash("secretpassword")
// $argon2id$v=19$m=19456,t=2,p=1$...
```

#### argon2_verify(password, hash)

Verifies a password against an Argon2 hash.

**Parameters:**
- `password` (String) - The password to verify
- `hash` (String) - The stored hash

**Returns:** Bool - true if password matches

**Example:**
```soli
if argon2_verify(user_input, stored_hash) {
    println("Password correct!")
}
```

#### password_hash(password)

Alias for `argon2_hash`.

#### password_verify(password, hash)

Alias for `argon2_verify`.

### X25519 Key Exchange

#### x25519_keypair()

Generates a new X25519 key pair.

**Returns:** Hash - `{ "public": String, "private": String }` (base64-encoded)

**Example:**
```soli
let keypair = x25519_keypair()
println(keypair["public"])
```

#### x25519_public_key(private_key)

Derives the public key from a private key.

**Parameters:**
- `private_key` (String) - Base64-encoded private key

**Returns:** String - Base64-encoded public key

#### x25519_shared_secret(private_key, public_key)

Computes the shared secret from a private key and another party's public key.

**Parameters:**
- `private_key` (String) - Your base64-encoded private key
- `public_key` (String) - Their base64-encoded public key

**Returns:** String - Base64-encoded shared secret

**Example:**
```soli
// Alice
let alice = x25519_keypair()

// Bob
let bob = x25519_keypair()

// Both compute the same shared secret
let alice_secret = x25519_shared_secret(alice["private"], bob["public"])
let bob_secret = x25519_shared_secret(bob["private"], alice["public"])
// alice_secret == bob_secret
```

### Ed25519 Signatures

#### ed25519_keypair()

Generates a new Ed25519 signing key pair.

**Returns:** Hash - `{ "public": String, "private": String }` (base64-encoded)

---

## JWT Functions

### jwt_sign(payload, secret, options?)

Creates a signed JWT token.

**Parameters:**
- `payload` (Hash) - Claims to include in the token
- `secret` (String) - Secret key for signing
- `options` (Hash, optional) - Token options
  - `expires_in` (Int) - Expiration time in seconds
  - `algorithm` (String) - "HS256", "HS384", or "HS512"

**Returns:** String - The JWT token

**Example:**
```soli
let token = jwt_sign(
    { "sub": "user123", "role": "admin" },
    "my-secret-key",
    { "expires_in": 3600 }
)
```

### jwt_verify(token, secret)

Verifies and decodes a JWT token.

**Parameters:**
- `token` (String) - The JWT token
- `secret` (String) - Secret key used for signing

**Returns:** Hash - Decoded payload, or `{ "error": true, "message": String }` on failure

**Example:**
```soli
let result = jwt_verify(token, "my-secret-key")
if has_key(result, "error") {
    println("Invalid token: " + result["message"])
} else {
    println("User: " + result["sub"])
}
```

### jwt_decode(token)

Decodes a JWT token without verification (unsafe for authentication).

**Parameters:**
- `token` (String) - The JWT token

**Returns:** Hash - Decoded payload

**Example:**
```soli
let payload = jwt_decode(token)
println(payload["sub"])  // Inspect claims without verification
```

---

## Regex Functions

### regex_match(pattern, string)

Tests if a string matches a regex pattern.

**Parameters:**
- `pattern` (String) - Regular expression pattern
- `string` (String) - String to test

**Returns:** Bool

**Example:**
```soli
regex_match("^[a-z]+$", "hello")  // true
regex_match("^[0-9]+$", "hello")  // false
```

### regex_find(pattern, string)

Finds the first match of a pattern in a string.

**Parameters:**
- `pattern` (String) - Regular expression pattern
- `string` (String) - String to search

**Returns:** Hash|null - `{ "match": String, "start": Int, "end": Int }` or null

**Example:**
```soli
let result = regex_find("[0-9]+", "abc123def")
println(result["match"])  // "123"
println(result["start"])  // 3
```

### regex_find_all(pattern, string)

Finds all matches of a pattern in a string.

**Parameters:**
- `pattern` (String) - Regular expression pattern
- `string` (String) - String to search

**Returns:** Array - Array of match hashes

**Example:**
```soli
let matches = regex_find_all("[0-9]+", "a1b2c3")
// [{"match": "1", ...}, {"match": "2", ...}, {"match": "3", ...}]
```

### regex_replace(pattern, string, replacement)

Replaces the first match of a pattern.

**Parameters:**
- `pattern` (String) - Regular expression pattern
- `string` (String) - String to modify
- `replacement` (String) - Replacement text

**Returns:** String

**Example:**
```soli
regex_replace("[0-9]+", "a1b2c3", "X")  // "aXb2c3"
```

### regex_replace_all(pattern, string, replacement)

Replaces all matches of a pattern.

**Parameters:**
- `pattern` (String) - Regular expression pattern
- `string` (String) - String to modify
- `replacement` (String) - Replacement text

**Returns:** String

**Example:**
```soli
regex_replace_all("[0-9]+", "a1b2c3", "X")  // "aXbXcX"
```

### regex_split(pattern, string)

Splits a string by a regex pattern.

**Parameters:**
- `pattern` (String) - Regular expression pattern
- `string` (String) - String to split

**Returns:** Array - Array of substrings

**Example:**
```soli
regex_split("[,;]", "a,b;c,d")  // ["a", "b", "c", "d"]
```

### regex_capture(pattern, string)

Finds the first match with named capture groups.

**Parameters:**
- `pattern` (String) - Regular expression with named groups `(?P<name>...)`
- `string` (String) - String to search

**Returns:** Hash|null - Match info plus named captures

**Example:**
```soli
let result = regex_capture(
    "(?P<year>[0-9]{4})-(?P<month>[0-9]{2})",
    "Date: 2024-01-15"
)
println(result["year"])   // "2024"
println(result["month"])  // "01"
```

### regex_escape(string)

Escapes special regex characters in a string.

**Parameters:**
- `string` (String) - String to escape

**Returns:** String

**Example:**
```soli
regex_escape("hello.world")  // "hello\\.world"
```

---

## Environment Functions

### getenv(name)

Gets an environment variable.

**Parameters:**
- `name` (String) - Variable name

**Returns:** String|null - Variable value or null if not set

**Example:**
```soli
let path = getenv("PATH")
let debug = getenv("DEBUG")
```

### setenv(name, value)

Sets an environment variable.

**Parameters:**
- `name` (String) - Variable name
- `value` (String) - Variable value

**Returns:** null

**Example:**
```soli
setenv("MY_VAR", "my_value")
```

### unsetenv(name)

Removes an environment variable.

**Parameters:**
- `name` (String) - Variable name

**Returns:** null

### hasenv(name)

Checks if an environment variable exists.

**Parameters:**
- `name` (String) - Variable name

**Returns:** Bool

**Example:**
```soli
if hasenv("DATABASE_URL") {
    let url = getenv("DATABASE_URL")
}
```

### dotenv(path?)

Loads environment variables from a .env file.

**Parameters:**
- `path` (String, optional) - Path to .env file (default: ".env")

**Returns:** Int - Number of variables loaded

**Example:**
```soli
dotenv()                    // Loads .env and .env.{APP_ENV}
dotenv(".env.production")   // Loads specific file
```

### dotenv!(path?)

Same as `dotenv()` - loads environment variables from a .env file.

---

## DateTime Functions

Soli uses Unix timestamps (integers) for datetime operations.

### datetime_now_local()

Gets the current local time as a Unix timestamp.

**Returns:** Int - Unix timestamp

**Example:**
```soli
let now = __datetime_now_local()
```

### datetime_now_utc()

Gets the current UTC time as a Unix timestamp.

**Returns:** Int - Unix timestamp

### datetime_from_unix(timestamp)

Creates a datetime from a Unix timestamp.

**Parameters:**
- `timestamp` (Int) - Unix timestamp

**Returns:** Int - The same timestamp (for consistency)

### datetime_to_unix(timestamp)

Converts a datetime to a Unix timestamp.

**Parameters:**
- `timestamp` (Int) - Unix timestamp

**Returns:** Int - Unix timestamp

### datetime_parse(string)

Parses a datetime string to a Unix timestamp.

**Parameters:**
- `string` (String) - Date string in ISO 8601 or RFC formats

**Returns:** Int|null - Unix timestamp or null if parsing fails

**Example:**
```soli
let ts = __datetime_parse("2024-01-15T10:30:00Z")
let ts2 = __datetime_parse("2024-01-15")
```

### datetime_format(timestamp, format)

Formats a timestamp as a string.

**Parameters:**
- `timestamp` (Int) - Unix timestamp
- `format` (String) - Format string (strftime format)

**Returns:** String

**Example:**
```soli
__datetime_format(ts, "%Y-%m-%d %H:%M:%S")  // "2024-01-15 10:30:00"
__datetime_format(ts, "%B %d, %Y")           // "January 15, 2024"
```

### datetime_components(timestamp)

Gets datetime components as a hash.

**Parameters:**
- `timestamp` (Int) - Unix timestamp

**Returns:** Hash - `{ "year", "month", "day", "hour", "minute", "second", "nanosecond", "weekday", "ordinal", "is_dst" }`

**Example:**
```soli
let parts = __datetime_components(ts)
println(parts["year"])     // 2024
println(parts["weekday"])  // "monday"
```

### datetime_add(timestamp, seconds)

Adds seconds to a timestamp.

**Parameters:**
- `timestamp` (Int) - Unix timestamp
- `seconds` (Int) - Seconds to add

**Returns:** Int - New timestamp

### datetime_sub(timestamp, seconds)

Subtracts seconds from a timestamp.

### datetime_diff(timestamp1, timestamp2)

Calculates the difference between two timestamps in seconds.

**Parameters:**
- `timestamp1` (Int) - First timestamp
- `timestamp2` (Int) - Second timestamp

**Returns:** Int - Difference (timestamp2 - timestamp1)

### datetime_is_before(timestamp1, timestamp2)

Checks if timestamp1 is before timestamp2.

**Returns:** Bool

### datetime_is_after(timestamp1, timestamp2)

Checks if timestamp1 is after timestamp2.

**Returns:** Bool

### datetime_to_iso(timestamp)

Converts a timestamp to ISO 8601 format.

**Parameters:**
- `timestamp` (Int) - Unix timestamp

**Returns:** String - ISO 8601 formatted string

**Example:**
```soli
__datetime_to_iso(ts)  // "2024-01-15T10:30:00+00:00"
```

### datetime_weekday(timestamp)

Gets the weekday name for a timestamp.

**Parameters:**
- `timestamp` (Int) - Unix timestamp

**Returns:** String - Weekday name (e.g., "monday")

---

## Validation Functions

Soli provides a schema-based validation system using the `V` class.

### V.string()

Creates a string validator.

**Returns:** Validator

### V.int()

Creates an integer validator.

### V.float()

Creates a float validator.

### V.bool()

Creates a boolean validator.

### V.array(element_schema?)

Creates an array validator with optional element schema.

**Parameters:**
- `element_schema` (Validator, optional) - Schema for array elements

### V.hash(schema?)

Creates a hash validator with optional nested schema.

**Parameters:**
- `schema` (Hash, optional) - Nested field schemas

### Validator Chain Methods

Validators support chaining:

```soli
V.string().required().min(3).max(100).email()
```

Available methods:
- `.required()` - Field must be present and non-null
- `.optional()` - Field can be omitted
- `.nullable()` - Field can be null
- `.default(value)` - Default value if missing
- `.min(n)` - Minimum length/value
- `.max(n)` - Maximum length/value
- `.pattern(regex)` - Must match regex pattern
- `.email()` - Must be valid email format
- `.url()` - Must be valid URL format

### validate(data, schema)

Validates data against a schema.

**Parameters:**
- `data` (Hash) - Data to validate
- `schema` (Hash) - Validation schema

**Returns:** Hash - `{ "valid": Bool, "data": Hash, "errors": Array }`

**Example:**
```soli
let schema = {
    "name": V.string().required().min(2),
    "email": V.string().required().email(),
    "age": V.int().optional().min(0).max(150)
}

let result = validate({
    "name": "Alice",
    "email": "alice@example.com"
}, schema)

if result["valid"] {
    println("Data is valid!")
    println(result["data"])
} else {
    for error in result["errors"] {
        println(error["field"] + ": " + error["message"])
    }
}
```

---

## Session Functions

Session functions manage user session data in web applications.

### session_get(key)

Gets a value from the current session.

**Parameters:**
- `key` (String) - Session key

**Returns:** Any|null - Value or null if not found

**Example:**
```soli
let user_id = session_get("user_id")
```

### session_set(key, value)

Sets a value in the current session.

**Parameters:**
- `key` (String) - Session key
- `value` (Any) - Value to store

**Returns:** null

**Example:**
```soli
session_set("user_id", 123)
session_set("cart", [])
```

### session_delete(key)

Removes a value from the session.

**Parameters:**
- `key` (String) - Session key

**Returns:** Any|null - The removed value

### session_destroy()

Destroys the entire session.

**Returns:** null

**Example:**
```soli
// Logout user
session_destroy()
```

### session_regenerate()

Regenerates the session ID (for security after login).

**Returns:** String - New session ID

**Example:**
```soli
// After successful login
session_set("user_id", user["id"])
session_regenerate()
```

### session_has(key)

Checks if a key exists in the session.

**Parameters:**
- `key` (String) - Session key

**Returns:** Bool

### session_id()

Gets the current session ID.

**Returns:** String|null

---

## Testing Functions

### Test DSL Functions

#### test(description, callback)

Defines a test case.

**Parameters:**
- `description` (String) - Test description
- `callback` (Function) - Test function

**Example:**
```soli
test("addition works correctly", fn() {
    assert_eq(1 + 1, 2)
})
```

#### describe(description, callback)

Groups related tests.

**Parameters:**
- `description` (String) - Group description
- `callback` (Function) - Function containing tests

**Example:**
```soli
describe("Calculator", fn() {
    test("adds numbers", fn() {
        assert_eq(add(1, 2), 3)
    })

    test("subtracts numbers", fn() {
        assert_eq(subtract(5, 3), 2)
    })
})
```

#### context(description, callback)

Alias for `describe()`.

#### it(description, callback)

Alias for `test()`.

#### specify(description, callback)

Alias for `test()`.

#### before_each(callback)

Runs before each test in the current describe block.

#### after_each(callback)

Runs after each test in the current describe block.

#### before_all(callback)

Runs once before all tests in the current describe block.

#### after_all(callback)

Runs once after all tests in the current describe block.

#### pending()

Marks a test as pending (not yet implemented).

#### skip()

Skips the current test.

### Assertion Functions

#### assert(condition)

Asserts that a condition is true.

**Example:**
```soli
assert(1 < 2)
assert(user != null)
```

#### assert_not(condition)

Asserts that a condition is false.

#### assert_eq(actual, expected)

Asserts that two values are equal.

**Example:**
```soli
assert_eq(add(1, 2), 3)
assert_eq(user["name"], "Alice")
```

#### assert_ne(actual, expected)

Asserts that two values are not equal.

#### assert_null(value)

Asserts that a value is null.

#### assert_not_null(value)

Asserts that a value is not null.

#### assert_gt(a, b)

Asserts that a > b.

#### assert_lt(a, b)

Asserts that a < b.

#### assert_match(string, pattern)

Asserts that a string matches a regex pattern.

**Example:**
```soli
assert_match(email, "^[^@]+@[^@]+$")
```

#### assert_contains(collection, value)

Asserts that an array or string contains a value.

**Example:**
```soli
assert_contains([1, 2, 3], 2)
assert_contains("hello", "ell")
```

#### assert_hash_has_key(hash, key)

Asserts that a hash contains a specific key.

#### assert_json(string)

Asserts that a string is valid JSON.

---

## Factory Functions

Factories help create test data.

### Factory.define(name, data)

Defines a factory template.

**Parameters:**
- `name` (String) - Factory name
- `data` (Hash) - Default data

**Example:**
```soli
Factory.define("user", {
    "name": "Test User",
    "email": "test@example.com",
    "role": "user"
})
```

### Factory.create(name)

Creates an instance from a factory.

**Parameters:**
- `name` (String) - Factory name

**Returns:** Hash - Created data

**Example:**
```soli
let user = Factory.create("user")
```

### Factory.create_with(name, overrides)

Creates an instance with overridden attributes.

**Parameters:**
- `name` (String) - Factory name
- `overrides` (Hash) - Attributes to override

**Returns:** Hash

**Example:**
```soli
let admin = Factory.create_with("user", { "role": "admin" })
```

### Factory.create_list(name, count)

Creates multiple instances.

**Parameters:**
- `name` (String) - Factory name
- `count` (Int) - Number to create

**Returns:** Array

**Example:**
```soli
let users = Factory.create_list("user", 5)
```

### Factory.sequence(name)

Gets the next value in a sequence.

**Parameters:**
- `name` (String) - Sequence name

**Returns:** Int - Next sequence value (starts at 0)

**Example:**
```soli
Factory.sequence("user_id")  // 0
Factory.sequence("user_id")  // 1
Factory.sequence("user_id")  // 2
```

### Factory.clear()

Clears all factory definitions and sequences.

---

## I18n Functions

The `I18n` class provides internationalization support.

### I18n.locale()

Gets the current locale.

**Returns:** String

**Example:**
```soli
println(I18n.locale())  // "en"
```

### I18n.set_locale(locale)

Sets the current locale.

**Parameters:**
- `locale` (String) - Locale code (e.g., "en", "fr", "de")

**Returns:** String - The new locale

**Example:**
```soli
I18n.set_locale("fr")
```

### I18n.translate(key, locale?, translations?)

Translates a key.

**Parameters:**
- `key` (String) - Translation key
- `locale` (String, optional) - Override locale
- `translations` (Hash, optional) - Translation dictionary

**Returns:** String - Translated text or key as fallback

**Example:**
```soli
let translations = {
    "en.greeting": "Hello",
    "fr.greeting": "Bonjour"
}

I18n.set_locale("fr")
I18n.translate("greeting", null, translations)  // "Bonjour"
```

### I18n.plural(key, count, locale?, translations?)

Gets pluralized translation.

**Parameters:**
- `key` (String) - Base translation key
- `count` (Int) - Count for pluralization
- `locale` (String, optional) - Override locale
- `translations` (Hash, optional) - Translation dictionary

**Returns:** String

**Example:**
```soli
let translations = {
    "en.items_zero": "No items",
    "en.items_one": "1 item",
    "en.items_other": "Many items"
}

I18n.plural("items", 0, null, translations)  // "No items"
I18n.plural("items", 1, null, translations)  // "1 item"
I18n.plural("items", 5, null, translations)  // "Many items"
```

### I18n.format_number(number, locale?)

Formats a number according to locale conventions.

**Parameters:**
- `number` (Int|Float) - Number to format
- `locale` (String, optional) - Override locale

**Returns:** String

**Example:**
```soli
I18n.set_locale("en")
I18n.format_number(1234.56)  // "1234.56"

I18n.set_locale("fr")
I18n.format_number(1234.56)  // "1234,56"
```

### I18n.format_currency(amount, currency, locale?)

Formats a currency amount.

**Parameters:**
- `amount` (Int|Float) - Amount
- `currency` (String) - Currency code (USD, EUR, GBP, JPY)
- `locale` (String, optional) - Override locale

**Returns:** String

**Example:**
```soli
I18n.format_currency(1234.56, "USD", "en")  // "$1,234.56"
I18n.format_currency(1234.56, "EUR", "fr")  // "1.234,56"
```

### I18n.format_date(timestamp, locale?)

Formats a date according to locale conventions.

**Parameters:**
- `timestamp` (Int) - Unix timestamp
- `locale` (String, optional) - Override locale

**Returns:** String

**Example:**
```soli
I18n.format_date(ts, "en")  // "01/15/2024"
I18n.format_date(ts, "fr")  // "15/01/2024"
I18n.format_date(ts, "de")  // "15.01.2024"
```

---

## Control Flow

### break

Exits a loop early.

**Example:**
```soli
for i in range(0, 10) {
    if i == 5 {
        break
    }
    println(i)
}
```

### await

Awaits an asynchronous operation (used internally for async HTTP).

---

## See Also

- [Soli Language Reference](/docs/soli-language) - Core language syntax and features
- [Testing Guide](/docs/testing) - Complete testing documentation
- [Validation Guide](/docs/validation) - Input validation in depth
- [Authentication Guide](/docs/authentication) - JWT authentication patterns
