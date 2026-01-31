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

Array operations like `push()`, `pop()`, `map()`, `filter()`, and more are available as methods on the Array class. See the Array class documentation for details.

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

All cryptographic functions are available both as static methods on the `Crypto` class and as standalone functions.

### Hash Functions

#### Crypto.sha256(data) / sha256(data)

Computes SHA-256 hash of a string.

**Parameters:**
- `data` (String) - The data to hash

**Returns:** String - 64-character hex string (32 bytes)

**Example:**
```soli
let hash = Crypto.sha256("hello")
// "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
```

#### Crypto.sha512(data) / sha512(data)

Computes SHA-512 hash of a string.

**Parameters:**
- `data` (String) - The data to hash

**Returns:** String - 128-character hex string (64 bytes)

**Example:**
```soli
let hash = Crypto.sha512("hello")
```

#### Crypto.md5(data) / md5(data)

Computes MD5 hash of a string. **Note:** MD5 is cryptographically broken. Use only for checksums, not security.

**Parameters:**
- `data` (String) - The data to hash

**Returns:** String - 32-character hex string (16 bytes)

**Example:**
```soli
let hash = Crypto.md5("hello")
// "5d41402abc4b2a76b9719d911017c592"
```

#### Crypto.hmac(message, key) / hmac(message, key)

Computes HMAC-SHA256 message authentication code.

**Parameters:**
- `message` (String) - The message to authenticate
- `key` (String) - The secret key

**Returns:** String - 64-character hex string (32 bytes)

**Example:**
```soli
let mac = Crypto.hmac("message", "secret_key")
// Use for API signature verification, webhook validation, etc.
```

### Base64 Encoding

Base64 encoding and decoding is available via the **Base64 class**:

- `Base64.encode(data)` - Encodes a string to Base64
- `Base64.decode(data)` - Decodes a Base64 string

See the [Base64 class documentation](/docs/utility/base64) for details.

### Password Hashing

#### Crypto.argon2_hash(password) / argon2_hash(password)

Hashes a password using Argon2id (recommended).

**Parameters:**
- `password` (String) - The password to hash

**Returns:** String - The hash string

**Example:**
```soli
let hash = Crypto.argon2_hash("secretpassword")
// $argon2id$v=19$m=19456,t=2,p=1$...
```

#### Crypto.argon2_verify(password, hash) / argon2_verify(password, hash)

Verifies a password against an Argon2 hash.

**Parameters:**
- `password` (String) - The password to verify
- `hash` (String) - The stored hash

**Returns:** Bool - true if password matches

**Example:**
```soli
if Crypto.argon2_verify(user_input, stored_hash) {
    println("Password correct!")
}
```

#### password_hash(password)

Alias for `Crypto.argon2_hash`.

#### password_verify(password, hash)

Alias for `Crypto.argon2_verify`.

### X25519 Key Exchange

#### Crypto.x25519_keypair() / x25519_keypair()

Generates a new X25519 key pair.

**Returns:** Hash - `{ "public": String, "private": String }` (hex-encoded, 64 chars each)

**Example:**
```soli
let keypair = Crypto.x25519_keypair()
println(keypair["public"])
```

#### Crypto.x25519_public_key(private_key) / x25519_public_key(private_key)

Derives the public key from a private key.

**Parameters:**
- `private_key` (String) - Hex-encoded private key

**Returns:** String - Hex-encoded public key

#### Crypto.x25519_shared_secret(private_key, public_key) / x25519_shared_secret(private_key, public_key)

Computes the shared secret from a private key and another party's public key.

**Parameters:**
- `private_key` (String) - Your hex-encoded private key
- `public_key` (String) - Their hex-encoded public key

**Returns:** String - Hex-encoded shared secret

**Example:**
```soli
// Alice
let alice = Crypto.x25519_keypair()

// Bob
let bob = Crypto.x25519_keypair()

// Both compute the same shared secret
let alice_secret = Crypto.x25519_shared_secret(alice["private"], bob["public"])
let bob_secret = Crypto.x25519_shared_secret(bob["private"], alice["public"])
// alice_secret == bob_secret
```

### Ed25519 Signatures

#### Crypto.ed25519_keypair() / ed25519_keypair()

Generates a new Ed25519 signing key pair.

**Returns:** Hash - `{ "public": String, "private": String }` (hex-encoded, 64 chars each)

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

## Regex Class

The `Regex` class provides static methods for regular expression operations.

### Regex.matches(pattern, string)

Tests if a string matches a regex pattern.

**Parameters:**
- `pattern` (String) - Regular expression pattern
- `string` (String) - String to test

**Returns:** Bool

**Example:**
```soli
Regex.matches("^[a-z]+$", "hello")  // true
Regex.matches("^[0-9]+$", "hello")  // false
```

### Regex.find(pattern, string)

Finds the first match of a pattern in a string.

**Parameters:**
- `pattern` (String) - Regular expression pattern
- `string` (String) - String to search

**Returns:** Hash|null - `{ "match": String, "start": Int, "end": Int }` or null

**Example:**
```soli
let result = Regex.find("[0-9]+", "abc123def")
println(result["match"])  // "123"
println(result["start"])  // 3
```

### Regex.find_all(pattern, string)

Finds all matches of a pattern in a string.

**Parameters:**
- `pattern` (String) - Regular expression pattern
- `string` (String) - String to search

**Returns:** Array - Array of match hashes

**Example:**
```soli
let matches = Regex.find_all("[0-9]+", "a1b2c3")
// [{"match": "1", ...}, {"match": "2", ...}, {"match": "3", ...}]
```

### Regex.replace(pattern, string, replacement)

Replaces the first match of a pattern.

**Parameters:**
- `pattern` (String) - Regular expression pattern
- `string` (String) - String to modify
- `replacement` (String) - Replacement text

**Returns:** String

**Example:**
```soli
Regex.replace("[0-9]+", "a1b2c3", "X")  // "aXb2c3"
```

### Regex.replace_all(pattern, string, replacement)

Replaces all matches of a pattern.

**Parameters:**
- `pattern` (String) - Regular expression pattern
- `string` (String) - String to modify
- `replacement` (String) - Replacement text

**Returns:** String

**Example:**
```soli
Regex.replace_all("[0-9]+", "a1b2c3", "X")  // "aXbXcX"
```

### Regex.split(pattern, string)

Splits a string by a regex pattern.

**Parameters:**
- `pattern` (String) - Regular expression pattern
- `string` (String) - String to split

**Returns:** Array - Array of substrings

**Example:**
```soli
Regex.split("[,;]", "a,b;c,d")  // ["a", "b", "c", "d"]
```

### Regex.capture(pattern, string)

Finds the first match with named capture groups.

**Parameters:**
- `pattern` (String) - Regular expression with named groups `(?P<name>...)`
- `string` (String) - String to search

**Returns:** Hash|null - Match info plus named captures

**Example:**
```soli
let result = Regex.capture(
    "(?P<year>[0-9]{4})-(?P<month>[0-9]{2})",
    "Date: 2024-01-15"
)
println(result["year"])   // "2024"
println(result["month"])  // "01"
```

### Regex.escape(string)

Escapes special regex characters in a string.

**Parameters:**
- `string` (String) - String to escape

**Returns:** String

**Example:**
```soli
Regex.escape("hello.world")  // "hello\\.world"
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

## DateTime Class

The `DateTime` class provides a convenient way to work with dates and times. Create instances using static methods, then use instance methods to extract components or perform arithmetic.

### Static Methods

#### DateTime.now()

Gets the current local date and time.

**Returns:** DateTime - A DateTime instance

**Example:**
```soli
let now = DateTime.now()
println(now.to_iso())  // "2024-01-15T10:30:00"
```

#### DateTime.utc()

Gets the current UTC date and time.

**Returns:** DateTime - A DateTime instance

**Example:**
```soli
let utc = DateTime.utc()
println(utc.to_iso())  // "2024-01-15T15:30:00Z"
```

#### DateTime.parse(string)

Parses a datetime string in ISO 8601 or RFC format.

**Parameters:**
- `string` (String) - Date string to parse

Supported formats:
- RFC 3339: `"2024-01-15T10:30:00Z"`
- RFC 2822: `"Mon, 15 Jan 2024 10:30:00 +0000"`
- ISO datetime: `"2024-01-15T10:30:00"` or `"2024-01-15 10:30:00"`
- ISO date only: `"2024-01-15"`

**Returns:** DateTime - A DateTime instance

**Example:**
```soli
let dt = DateTime.parse("2024-01-15T10:30:00Z")
let date_only = DateTime.parse("2024-01-15")
```

### Instance Methods - Components

#### .year()

Gets the year component (e.g., 2024).

**Returns:** Int

#### .month()

Gets the month component (1-12).

**Returns:** Int - 1 = January, 12 = December

#### .day()

Gets the day of month (1-31).

**Returns:** Int

#### .hour()

Gets the hour component (0-23).

**Returns:** Int

#### .minute()

Gets the minute component (0-59).

**Returns:** Int

#### .second()

Gets the second component (0-59).

**Returns:** Int

#### .weekday()

Gets the day of the week as a string.

**Returns:** String - Lowercase weekday name (e.g., "monday", "tuesday")

**Example:**
```soli
let dt = DateTime.parse("2024-01-15")
println(dt.weekday())  // "monday"
```

### Instance Methods - Formatting

#### .to_unix()

Gets the Unix timestamp (seconds since epoch).

**Returns:** Int

**Example:**
```soli
let dt = DateTime.now()
println(dt.to_unix())  // 1705315800
```

#### .to_iso()

Gets the date/time as an ISO 8601 string.

**Returns:** String

**Example:**
```soli
let dt = DateTime.now()
println(dt.to_iso())  // "2024-01-15T10:30:00"
```

#### .format(pattern)

Formats the date/time using strftime pattern specifiers.

**Parameters:**
- `pattern` (String) - strftime format pattern

Common format specifiers:
- `%Y` - 4-digit year (2024)
- `%m` - 2-digit month (01-12)
- `%d` - 2-digit day (01-31)
- `%H` - 24-hour hour (00-23)
- `%M` - Minute (00-59)
- `%S` - Second (00-59)
- `%B` - Full month name (January)
- `%A` - Full weekday name (Monday)

**Returns:** String

**Example:**
```soli
let dt = DateTime.parse("2024-01-15T10:30:00")
dt.format("%Y-%m-%d %H:%M:%S")  // "2024-01-15 10:30:00"
dt.format("%B %d, %Y")           // "January 15, 2024"
dt.format("%A")                  // "Monday"
```

### Instance Methods - Arithmetic

#### .add_days(n)

Adds days to the date. Use negative values to subtract.

**Parameters:**
- `n` (Int) - Number of days to add

**Returns:** DateTime - A new DateTime instance

**Example:**
```soli
let today = DateTime.now()
let tomorrow = today.add_days(1)
let yesterday = today.add_days(-1)
```

#### .add_hours(n)

Adds hours to the date/time. Use negative values to subtract.

**Parameters:**
- `n` (Int) - Number of hours to add

**Returns:** DateTime - A new DateTime instance

#### .add_weeks(n)

Adds weeks to the date. Use negative values to subtract.

**Parameters:**
- `n` (Int) - Number of weeks to add

**Returns:** DateTime - A new DateTime instance

#### .add_months(n)

Adds months to the date. Use negative values to subtract.

**Parameters:**
- `n` (Int) - Number of months to add

**Returns:** DateTime - A new DateTime instance

#### .add_years(n)

Adds years to the date. Use negative values to subtract.

**Parameters:**
- `n` (Int) - Number of years to add

**Returns:** DateTime - A new DateTime instance

### Complete Example

```soli
// Get current date/time
let now = DateTime.now()
println("Current time: " + now.to_iso())

// Extract components
println("Year: " + now.year())
println("Month: " + now.month())
println("Day: " + now.day())
println("Weekday: " + now.weekday())

// Format output
println(now.format("%B %d, %Y at %H:%M"))

// Date arithmetic
let next_week = now.add_weeks(1)
let last_month = now.add_months(-1)

// Parse a date string
let birthday = DateTime.parse("1990-06-15")
println("Birthday was on a " + birthday.weekday())
```

---

## Duration Class

The `Duration` class represents a span of time. Create durations using static factory methods, then convert them to different units as needed.

### Static Methods

#### Duration.seconds(n)

Creates a duration from a number of seconds.

**Parameters:**
- `n` (Int) - Number of seconds

**Returns:** Duration

**Example:**
```soli
let timeout = Duration.seconds(30)
let one_minute = Duration.seconds(60)
```

#### Duration.minutes(n)

Creates a duration from a number of minutes.

**Parameters:**
- `n` (Int) - Number of minutes

**Returns:** Duration

**Example:**
```soli
let break_time = Duration.minutes(15)
```

#### Duration.hours(n)

Creates a duration from a number of hours.

**Parameters:**
- `n` (Int) - Number of hours

**Returns:** Duration

**Example:**
```soli
let work_day = Duration.hours(8)
let session_timeout = Duration.hours(1)
```

#### Duration.days(n)

Creates a duration from a number of days.

**Parameters:**
- `n` (Int) - Number of days

**Returns:** Duration

**Example:**
```soli
let week = Duration.days(7)
let trial_period = Duration.days(30)
```

### Instance Methods

#### .to_seconds()

Gets the total duration in seconds.

**Returns:** Int

**Example:**
```soli
let duration = Duration.hours(2)
println(duration.to_seconds())  // 7200
```

#### .to_minutes()

Gets the total duration in minutes.

**Returns:** Int

**Example:**
```soli
let duration = Duration.hours(2)
println(duration.to_minutes())  // 120
```

#### .to_hours()

Gets the total duration in hours.

**Returns:** Int

**Example:**
```soli
let duration = Duration.days(1)
println(duration.to_hours())  // 24
```

### Complete Example

```soli
// Create durations
let timeout = Duration.seconds(30)
let break_time = Duration.minutes(15)
let work_day = Duration.hours(8)
let trial = Duration.days(7)

// Convert to different units
println("Timeout: " + timeout.to_seconds() + " seconds")
println("Break: " + break_time.to_minutes() + " minutes")
println("Work day: " + work_day.to_hours() + " hours")
println("Trial: " + trial.to_hours() + " hours")

// Practical example: session expiry
let session_duration = Duration.hours(1)
let expiry_seconds = session_duration.to_seconds()
println("Session expires in " + expiry_seconds + " seconds")
```

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

## Cache Functions

In-memory cache for storing and retrieving data with TTL support.

### cache_set(key, value, ttl_seconds?)

Stores a value in the cache.

**Parameters:**
- `key` (String) - Cache key
- `value` (Any) - Value to cache (will be JSON serialized)
- `ttl_seconds` (Int, optional) - Time to live in seconds (default: 3600)

**Returns:** null

**Example:**
```soli
cache_set("user:123", { "name": "Alice", "email": "alice@example.com" })
cache_set("session", session_data, 1800)  // 30 minute TTL
```

### cache_get(key)

Retrieves a value from the cache.

**Parameters:**
- `key` (String) - Cache key

**Returns:** Any|null - Cached value or null if not found/expired

**Example:**
```soli
let user = cache_get("user:123")
if user != null {
    println("Cached user: " + user["name"])
}
```

### cache_delete(key)

Removes a value from the cache.

**Parameters:**
- `key` (String) - Cache key

**Returns:** Bool - true if key was removed

### cache_has(key)

Checks if a key exists in the cache (and is not expired).

**Parameters:**
- `key` (String) - Cache key

**Returns:** Bool

### cache_clear()

Removes all entries from the cache.

**Returns:** null

### cache_clear_expired()

Removes only expired entries from the cache.

**Returns:** null

### cache_keys()

Returns all valid (non-expired) cache keys.

**Returns:** Array - Array of key strings

### cache_ttl(key)

Gets the remaining TTL for a key.

**Parameters:**
- `key` (String) - Cache key

**Returns:** Int|null - Seconds remaining, or null if not found/expired

### cache_touch(key, ttl)

Extends or sets the TTL for an existing key.

**Parameters:**
- `key` (String) - Cache key
- `ttl` (Int) - New TTL in seconds

**Returns:** Bool - true if key existed and was updated

### cache_size()

Returns the number of entries in the cache.

**Returns:** Int

### cache_config(ttl?, max_size?)

Configures cache defaults.

**Parameters:**
- `ttl` (Int|null, optional) - Default TTL in seconds
- `max_size` (Int|null, optional) - Maximum entries (default: 10000)

**Returns:** null

---

## RateLimiter Class

Sliding window rate limiter for API protection and abuse prevention.

### Constructor

**RateLimiter(key, limit, window_seconds)**

Creates a new rate limiter instance for the given key.

**Parameters:**
- `key` (String) - Rate limit key (e.g., "ip:192.168.1.1" or "user:123")
- `limit` (Int) - Maximum requests allowed in the window
- `window_seconds` (Int) - Time window in seconds

**Example:**
```soli
// Create a rate limiter for API access
let limiter = RateLimiter("api:user123", 100, 60)
```

### Instance Methods

**limiter.allowed()**

Checks if a request is allowed under the rate limit.

**Returns:** Bool - true if allowed, false if rate limited

**Example:**
```soli
let limiter = RateLimiter("ip:" + req["headers"]["X-Forwarded-For"], 100, 60)
if !limiter.allowed() {
    return { "status": 429, "body": "Too Many Requests" }
}
```

**limiter.throttle()**

Returns the number of seconds until the next request is allowed.

**Returns:** Int - Seconds to wait (0 if allowed immediately)

**limiter.status()**

Gets detailed rate limit status.

**Returns:** Hash with keys:
- `allowed` (Bool) - Whether request is allowed
- `remaining` (Int) - Requests remaining in window
- `reset_in` (Int) - Seconds until limit resets
- `limit` (Int) - The limit
- `window` (Int) - The window in seconds

**Example:**
```soli
let limiter = RateLimiter("api:" + api_key, 1000, 3600)
let status = limiter.status()
println("Remaining: " + str(status["remaining"]) + "/" + str(status["limit"]))
```

**limiter.headers()**

Generates rate limit headers for HTTP responses.

**Returns:** Hash with:
- `X-RateLimit-Limit` - Total limit
- `X-RateLimit-Remaining` - Requests remaining
- `X-RateLimit-Reset` - Reset timestamp

**limiter.reset()**

Resets the rate limit for this instance's key.

**Returns:** Bool - true on success

### Static Methods

**RateLimiter.reset_all()**

Resets all rate limit counters.

**Returns:** Bool

**RateLimiter.cleanup()**

Cleans up expired rate limit entries.

**Returns:** Bool

### Helper Functions

**rate_limiter_from_ip(req, limit, window_seconds)**

Creates a RateLimiter instance based on client IP address (extracts from X-Forwarded-For or Remote-Addr).

**Parameters:**
- `req` (Hash) - Request hash
- `limit` (Int) - Maximum requests allowed
- `window_seconds` (Int, optional) - Time window in seconds (default: 60)

**Returns:** RateLimiter instance

---

## Security Headers Functions

Automatic security header injection for HTTP responses.

### enable_security_headers()

Enables automatic security header injection on all responses.

**Returns:** Bool

### disable_security_headers()

Disables automatic security header injection.

**Returns:** Bool

### security_headers_enabled()

Checks if security headers are enabled.

**Returns:** Bool

### set_csp(policy, report_only?)

Sets the Content-Security-Policy header.

**Parameters:**
- `policy` (String) - CSP policy string
- `report_only` (Bool, optional) - Use Content-Security-Policy-Report-Only

**Example:**
```soli
set_csp("default-src 'self'; script-src 'self' 'unsafe-inline'")
```

### set_csp_default_src(...sources)

Builds a CSP header with default-src directive.

**Parameters:**
- `sources` (String...) - CSP source values

**Example:**
```soli
set_csp_default_src("'self'", "'https://trusted-cdn.com'")
// Generates: default-src 'self' 'https://trusted-cdn.com'
```

### set_hsts(max_age, include_subdomains?, preload?)

Sets the Strict-Transport-Security header.

**Parameters:**
- `max_age` (Int) - Max age in seconds
- `include_subdomains` (Bool, optional) - Include subdomains flag (default: true)
- `preload` (Bool, optional) - Add preload directive (default: false)

**Example:**
```soli
set_hsts(31536000, true, false)  // 1 year, include subdomains
```

### prevent_clickjacking()

Sets X-Frame-Options: DENY to prevent clickjacking.

### allow_same_origin_frames()

Sets X-Frame-Options: SAMEORIGIN to allow same-origin framing.

### set_xss_protection(mode)

Sets the X-XSS-Protection header.

**Parameters:**
- `mode` (String) - Protection mode (e.g., "block", "report=...")

### set_content_type_options()

Sets X-Content-Type-Options: nosniff to prevent MIME type sniffing.

### set_referrer_policy(policy)

Sets the Referrer-Policy header.

**Parameters:**
- `policy` (String) - Policy value (e.g., "strict-origin-when-cross-origin")

### set_permissions_policy(policy)

Sets the Permissions-Policy header.

**Parameters:**
- `policy` (String) - Policy string

### set_coep(policy)

Sets the Cross-Origin-Embedder-Policy header.

**Parameters:**
- `policy` (String) - Policy (e.g., "require-corp")

### set_coop(policy)

Sets the Cross-Origin-Opener-Policy header.

**Parameters:**
- `policy` (String) - Policy (e.g., "same-origin")

### set_corp(policy)

Sets the Cross-Origin-Resource-Policy header.

**Parameters:**
- `policy` (String) - Policy (e.g., "same-site")

### secure_headers()

Applies recommended security headers for web apps.

### secure_headers_basic()

Applies basic security headers (X-Frame-Options, X-Content-Type-Options).

### secure_headers_strict()

Applies strict security headers including HSTS and CSP.

### secure_headers_api()

Applies minimal security headers suitable for JSON APIs.

### reset_security_headers()

Resets all security header configuration.

### get_security_headers()

Gets the current security headers configuration.

**Returns:** Hash - Current security headers

---

## File Upload Functions

Parse multipart form data and upload files to SolidB.

### parse_multipart(req)

Parses multipart/form-data from a request.

**Parameters:**
- `req` (Hash) - Request hash with body and headers

**Returns:** Array - Array of file hashes, each containing:
- `filename` (String) - Original filename
- `content_type` (String) - MIME type
- `size` (Int) - File size in bytes
- `data_base64` (String) - File content as base64
- `field_name` (String) - Form field name

**Example:**
```soli
let files = parse_multipart(req)
for file in files {
    println("Uploaded: " + file["filename"] + " (" + str(file["size"]) + " bytes)")
}
```

### upload_to_solidb(req, collection, field_name, solidb_addr)

Uploads a file from multipart form data to SolidB blob storage.

**Parameters:**
- `req` (Hash) - Request hash
- `collection` (String) - SolidB collection name
- `field_name` (String) - Form field name to upload
- `solidb_addr` (String) - SolidB server address

**Returns:** Hash with:
- `blob_id` (String) - Unique blob identifier
- `filename` (String) - Original filename
- `size` (Int) - File size
- `content_type` (String) - MIME type

**Example:**
```soli
let result = upload_to_solidb(req, "uploads", "avatar", "localhost:5678")
if has_key(result, "blob_id") {
    println("Uploaded: " + result["filename"])
    println("Blob ID: " + result["blob_id"])
}
```

### upload_all_to_solidb(req, collection, solidb_addr)

Uploads all files from multipart form data to SolidB.

**Parameters:**
- `req` (Hash) - Request hash
- `collection` (String) - SolidB collection name
- `solidb_addr` (String) - SolidB server address

**Returns:** Array - Array of result hashes (one per file, or error hash if failed)

### get_blob_url(collection, blob_id, base_url, expires_in?)

Generates a URL for downloading a blob.

**Parameters:**
- `collection` (String) - SolidB collection name
- `blob_id` (String) - Blob ID
- `base_url` (String) - Base URL for the SolidB server
- `expires_in` (Int, optional) - Expiration time in seconds (default: 3600)

**Returns:** String - Download URL

---

## SolidB Blob Methods

Methods on Solidb instances for storing and retrieving binary data.

### solidb.store_blob(collection, data_base64, filename, content_type)

Stores a file as a blob in SolidB.

**Parameters:**
- `collection` (String) - Collection name
- `data_base64` (String) - File content as base64
- `filename` (String) - Original filename
- `content_type` (String) - MIME type

**Returns:** String - Unique blob ID

**Example:**
```soli
let db = Solidb("localhost:5678", "myapp")
let blob_id = db.store_blob("avatars", image_data_base64, "photo.jpg", "image/jpeg")
```

### solidb.get_blob(collection, blob_id)

Retrieves a blob from SolidB.

**Parameters:**
- `collection` (String) - Collection name
- `blob_id` (String) - Blob ID

**Returns:** String - File content as base64

### solidb.get_blob_metadata(collection, blob_id)

Gets metadata for a blob without fetching the data.

**Parameters:**
- `collection` (String) - Collection name
- `blob_id` (String) - Blob ID

**Returns:** Hash with:
- `_key` (String) - Blob ID
- `filename` (String) - Original filename
- `content_type` (String) - MIME type
- `size` (Int) - File size in bytes
- `created_at` (String) - Creation timestamp

### solidb.delete_blob(collection, blob_id)

Deletes a blob from SolidB.

**Parameters:**
- `collection` (String) - Collection name
- `blob_id` (String) - Blob ID

**Returns:** String - "OK" on success

---

## See Also

- [Soli Language Reference](/docs/soli-language) - Core language syntax and features
- [Testing Guide](/docs/testing) - Complete testing documentation
- [Validation Guide](/docs/validation) - Input validation in depth
- [Authentication Guide](/docs/authentication) - JWT authentication patterns
