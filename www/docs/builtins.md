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
print(" World")  # Output: Hello World
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
name = input("Enter your name: ")
println("Hello, " + name)
```

---

### Debugging Functions

#### debug()

Triggers a debugger breakpoint: pauses execution and opens the interactive debug
page, where you can inspect variable state and evaluate arbitrary Soli code.
Only active in development mode — in production `debug()` calls are ignored.

**Parameters:** none

**Returns:** a breakpoint value

**Example:**
```soli
def process_user(user_id: Int) -> Hash
  user = User.find(user_id)
  debug()   # pause here and inspect `user`
  enrich_profile(user)
end
```

> **Renamed:** this builtin was called `break()` before. `break` is now a loop
> keyword, so the debugger builtin moved to `debug()`. See
> [Debugging](/docs/development-tools/debugging).

---

### Type Functions

#### type(value)

Returns the type name of a value as a string.

**Parameters:**
- `value` (Any) - The value to check

**Returns:** String - One of: "null", "bool", "int", "float", "string", "array", "hash", "function", "class", "instance"

**Example:**
```soli
type(42)        # "int"
type("hello")   # "string"
type([1, 2, 3]) # "array"
type(null)      # "null"
```

#### value.is_a?(class_name)

Returns whether a value is an instance of the specified class (or a subclass). Works on all objects including model instances.

**For instances (including model instances):** Checks class hierarchy
```soli
user = User.find("123")
user.is_a?("User")      # true
user.is_a?("Model")     # true (inheritance)
user.is_a?("String")    # false
```

**For primitives:** Supports "int", "numeric", "object" type names
```soli
123.is_a?("Int")        # true
123.is_a?("numeric")    # true
123.is_a?("string")     # false
"hello".is_a?("String") # true
[1, 2].is_a?("Array")   # true
```

#### defined(name)

Checks if a variable is defined in the current scope chain. Returns `true` if the variable exists, `false` otherwise.

**Parameters:**
- `name` (String) - The variable name to check

**Returns:** Bool

**Example:**
```soli
x = 42;
defined("x");        # true
defined("y");        # false

def check(val) {
  if defined("val") { "exists" } else { "not set" }
}
```

#### const_get(name)

Resolves a string name to its value in the current scope chain. Returns the value (class, function, variable, etc.) if defined, or `null` otherwise.

Useful for dynamic dispatch — e.g., converting a string like `"DemoUser"` into the actual `DemoUser` class.

**Parameters:**
- `name` (String) - The name to look up

**Returns:** Any or `null`

**Example:**
```soli
class DemoUser { }

let cls = const_get("DemoUser");
cls.new();                   # Creates a DemoUser instance

let not_found = const_get("NonExistent");
assert_null(not_found);
```

#### str(value)

Converts a value to a string.

**Parameters:**
- `value` (Any) - The value to convert

**Returns:** String

**Example:**
```soli
str(42)       # "42"
str(3.14)     # "3.14"
str(true)     # "true"
str([1, 2])   # "[1, 2]"
```

#### int(value)

Converts a value to an integer.

**Parameters:**
- `value` (Any) - The value to convert (string or float)

**Returns:** Int - The integer value, or error if conversion fails

**Example:**
```soli
int("42")     # 42
int(3.7)      # 3
int("3.14")   # 3
```

#### float(value)

Converts a value to a float.

**Parameters:**
- `value` (Any) - The value to convert (string or int)

**Returns:** Float

**Example:**
```soli
float("3.14")  # 3.14
float(42)      # 42.0
```

#### len(value)

Returns the length of a string, array, or hash. Also available as a method: `.len`, `.length`, `.size` are all aliases.

**Parameters:**
- `value` (String|Array|Hash) - The collection to measure

**Returns:** Int - The number of elements/characters

**Example:**
```soli
len("hello")      # 5
len([1, 2, 3])    # 3
len({"a": 1})     # 1

# Method syntax (all equivalent)
[1, 2, 3].len     # 3
[1, 2, 3].length  # 3
[1, 2, 3].size    # 3
```

---

### Array Functions

Array operations like `push()`, `pop()`, `map()`, `filter()`, and more are available as methods on the Array class. See the Array class documentation for details.

#### concat(other, ...)

Appends the elements of one or more arrays to the receiver **in place**, then returns the receiver. Mirrors Ruby's `Array#concat` — unlike `+`, which produces a new array, `.concat` mutates the original. The passed-in arrays are not modified. Raises if any argument is not an Array.

**Parameters:**
- `other` (Array) - One or more arrays whose elements are appended to the receiver

**Returns:** Array - The mutated receiver

**Example:**
```soli
a = [1, 2]
a.concat([3, 4])
print(a)  # [1, 2, 3, 4]

# Multiple arrays at once
nums = [1]
nums.concat([2, 3], [4, 5])
print(nums)  # [1, 2, 3, 4, 5]

# Empty arg is a no-op
[1, 2].concat([])  # [1, 2]
```

#### intersection(other)

Returns a new array of elements present in **both** the receiver and `other`, in receiver order, with duplicates removed. Originals are unchanged. Raises if the argument is not an Array.

**Parameters:**
- `other` (Array) - The array to intersect with

**Returns:** Array - A new deduplicated array of shared elements

**Example:**
```soli
[1, 2, 3].intersection([2, 3, 4])      # [2, 3]
[1, 1, 2, 2, 3].intersection([1, 2])    # [1, 2]
[1, 2, 3].intersection([4, 5, 6])        # []
[].intersection([1, 2])                  # []
```

#### union(other)

Returns a new array containing every element from the receiver followed by every new element from `other`, with duplicates removed. Originals are unchanged. Raises if the argument is not an Array.

**Parameters:**
- `other` (Array) - The array to union with

**Returns:** Array - A new deduplicated array containing both inputs

**Example:**
```soli
[1, 2, 3].union([2, 3, 4])    # [1, 2, 3, 4]
[1, 1, 2].union([2, 3])        # [1, 2, 3]
[1, 1, 2].union([])            # [1, 2]
```

#### difference(other)

Returns a new array of receiver elements that are **not** in `other`, with duplicates removed. Unlike `-`, the result is always deduplicated. Originals are unchanged. Raises if the argument is not an Array.

**Parameters:**
- `other` (Array) - The array of elements to exclude

**Returns:** Array - A new deduplicated array of receiver-only elements

**Example:**
```soli
[1, 2, 3].difference([2, 3])      # [1]
[1, 1, 2, 2, 3].difference([3])    # [1, 2]
[1, 2, 3].difference([1, 2, 3])    # []
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
range(0, 5)      # [0, 1, 2, 3, 4]
range(1, 10, 2)  # [1, 3, 5, 7, 9]
range(5, 0, -1)  # [5, 4, 3, 2, 1]
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
h = {"name": "Alice", "age": 30}
keys(h)  # ["name", "age"]
```

#### values(hash)

Returns an array of all values in a hash.

**Parameters:**
- `hash` (Hash) - The hash to get values from

**Returns:** Array - Array of values

**Example:**
```soli
h = {"name": "Alice", "age": 30}
values(h)  # ["Alice", 30]
```

#### has_key(hash, key)

Checks if a hash contains a specific key.

**Parameters:**
- `hash` (Hash) - The hash to search
- `key` (Any) - The key to look for

**Returns:** Bool

**Example:**
```soli
h = {"name": "Alice"}
has_key(h, "name")  # true
has_key(h, "age")   # false
```

#### delete(hash, key)

Removes a key-value pair from a hash.

**Parameters:**
- `hash` (Hash) - The hash to modify
- `key` (Any) - The key to remove

**Returns:** Any - The removed value, or null if not found

**Example:**
```soli
h = {"name": "Alice", "age": 30}
delete(h, "age")
println(h)  # {"name": "Alice"}
```

#### hash.merge(other)

Merges two hashes into a new hash.

**Parameters:**
- `other` (Hash) - The hash to merge in (values override the receiver)

**Returns:** Hash - A new merged hash

**Example:**
```soli
a = {"x": 1, "y": 2}
b = {"y": 3, "z": 4}
a.merge(b)  # {"x": 1, "y": 3, "z": 4}
```

#### entries(hash)

Returns an array of [key, value] pairs.

**Parameters:**
- `hash` (Hash) - The hash to convert

**Returns:** Array - Array of [key, value] arrays

**Example:**
```soli
h = {"a": 1, "b": 2}
entries(h)  # [["a", 1], ["b", 2]]
```

#### from_entries(array)

Creates a hash from an array of [key, value] pairs.

**Parameters:**
- `array` (Array) - Array of [key, value] arrays

**Returns:** Hash

**Example:**
```soli
from_entries([["a", 1], ["b", 2]])  # {"a": 1, "b": 2}
```

#### clear(hash)

Removes all entries from a hash.

**Parameters:**
- `hash` (Hash) - The hash to clear

**Returns:** null

**Example:**
```soli
h = {"a": 1, "b": 2}
clear(h)
println(h)  # {}
```

---

### String Functions

#### string.split([separator])

Splits a string into an array. Called as a method on the string.

**Parameters:**
- `separator` (String, optional) - The delimiter. Defaults to `" "` (a single space) when omitted.

**Returns:** Array - Array of substrings

**Example:**
```soli
"a,b,c".split(",")        # ["a", "b", "c"]
"hello world".split(" ")  # ["hello", "world"]

# Separator is optional — defaults to " "
"hello world".split       # ["hello", "world"]   (no parens, auto-invoked)
"hello world".split()     # ["hello", "world"]
"one  two".split          # ["one", "", "two"]   (consecutive spaces yield empty elements)
```

#### join(array, separator)

Joins an array into a string with a separator.

**Parameters:**
- `array` (Array) - The array to join
- `separator` (String) - The delimiter

**Returns:** String

**Example:**
```soli
join(["a", "b", "c"], ",")  # "a,b,c"
join([1, 2, 3], "-")        # "1-2-3"
```

#### contains(string, substring)

Checks if a string contains a substring.

**Parameters:**
- `string` (String) - The string to search in
- `substring` (String) - The string to find

**Returns:** Bool

**Example:**
```soli
contains("hello world", "world")  # true
contains("hello", "xyz")          # false
```

#### index_of(string, substring)

Finds the position of a substring.

**Parameters:**
- `string` (String) - The string to search in
- `substring` (String) - The string to find

**Returns:** Int - Index of first occurrence, or -1 if not found

**Example:**
```soli
index_of("hello", "ll")   # 2
index_of("hello", "xyz")  # -1
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
substring("hello", 1, 4)  # "ell"
substring("hello", 2)     # "llo"
```

#### upcase(string)

Converts a string to uppercase.

**Parameters:**
- `string` (String) - The string to convert

**Returns:** String

**Example:**
```soli
upcase("hello")  # "HELLO"
```

#### downcase(string)

Converts a string to lowercase.

**Parameters:**
- `string` (String) - The string to convert

**Returns:** String

**Example:**
```soli
downcase("HELLO")  # "hello"
```

#### trim(string)

Removes whitespace from both ends of a string.

**Parameters:**
- `string` (String) - The string to trim

**Returns:** String

**Example:**
```soli
trim("  hello  ")  # "hello"
```

#### html_escape(string)

Escapes HTML special characters.

**Parameters:**
- `string` (String) - The string to escape

**Returns:** String

**Example:**
```soli
html_escape("<script>alert('xss')</script>")
# "&lt;script&gt;alert('xss')&lt;/script&gt;"
```

#### string.html_entities()

Encodes every non-ASCII character as an HTML **numeric** entity (`é` → `&#233;`),
leaving ASCII — tags, attributes, and existing `&#…;` entities — untouched. The result
is pure ASCII, so it renders identically under any charset, and the method is idempotent
(running it twice changes nothing).

Use it when embedding accented text in an **HTML email body**: many providers/clients
re-emit the body as Latin-1 regardless of the request `Content-Type` charset or an
in-document `<meta charset="utf-8">`, which double-encodes raw UTF-8 (`é` → `Ã©`).
Numeric entities sidestep that entirely.

**Returns:** String

**Example:**
```soli
"Vous avez été inscrit·e".html_entities()
# "Vous avez &#233;t&#233; inscrit&#183;e"

"<p>plain ascii</p>".html_entities()  # unchanged
```

#### html_unescape(string)

Unescapes HTML entities.

**Parameters:**
- `string` (String) - The string to unescape

**Returns:** String

**Example:**
```soli
html_unescape("&lt;p&gt;")  # "<p>"
```

#### sanitize_html(string)

Removes potentially dangerous HTML tags and attributes.

**Parameters:**
- `string` (String) - The HTML to sanitize

**Returns:** String - Safe HTML

**Example:**
```soli
sanitize_html("<p onclick='evil()'>Hello</p>")
# "<p>Hello</p>"
```

#### url_encode(value)

Percent-encodes a value for safe use as a URL component (query value, path
segment, fragment, etc.). Strict RFC 3986 component encoding — every
reserved character (`/`, `?`, `&`, `=`, `#`, space, …) is escaped.

**Parameters:**
- `value` (String|Int|Float|Bool|null) - Scalars are stringified first; `null`
  becomes `""`.

**Returns:** String

**Example:**
```soli
url_encode("hello world")        # "hello%20world"
url_encode("a/b?c=d")            # "a%2Fb%3Fc%3Dd"
url_encode("café")               # "caf%C3%A9"

# Building a query string by hand:
q = "search " + str(page)
url = "/results?q=" + url_encode(q)
```

Reach for `url_encode` whenever you splice user-controlled or framework
data into a URL component — the strict policy means you don't have to
think about which separator chars need escaping in your specific spot.

#### url_decode(string)

Decodes a URL component using form-style rules: `+` becomes a space,
`%xx` becomes the corresponding byte. Invalid `%xx` sequences (e.g.
`%ZZ`) pass through literally; the function only errors when the
percent-decoded bytes are not valid UTF-8.

**Parameters:**
- `string` (String) - The encoded value. `null` decodes to `""`.

**Returns:** String

**Example:**
```soli
url_decode("hello%20world")          # "hello world"
url_decode("hello+world")            # "hello world"
url_decode("a%2Fb%3Fc%3Dd")          # "a/b?c=d"
url_decode("caf%C3%A9")              # "café"

# Roundtrip:
url_decode(url_encode("a/b?c=d"))    # "a/b?c=d"

# Fallback for input you don't trust:
safe = url_decode(raw) rescue raw
```

`req["query"]` and `req["form"]` are already decoded — use `url_decode`
when you receive a URL through some other channel (a header, a webhook
body, a stored URL string).

---

### File I/O Functions

> **Security — filesystem jail and symlink defense.** When the application is started with `soli serve <dir>`, every path passed to `slurp` / `barf` / `File.*` / `mkdir_p` / `file_write_*` is resolved relative to `<dir>` and rejected if it escapes that root (via absolute paths, `..` segments, or symlinks pointing outside). On Unix, every `File.*` open additionally passes `O_NOFOLLOW`, and metadata lookups go through `symlink_metadata`, so a path that *is* a symlink fails to open through `File.*` even when the target is itself in-jail — this defends against local TOCTOU swaps after canonicalisation. CLI invocations (`soli run`, the REPL, the test runner) do **not** install a jail, so command-line scripts keep full filesystem access.
>
> Code that genuinely needs to step outside the jail or follow symlinks deliberately (log shippers, backup scripts, cron-style maintenance jobs, tailing a symlinked log file) should use the parallel `Trusted` class — `Trusted.read("/var/log/...")`, `Trusted.write(...)`, etc. Its API mirrors `File` exactly but skips both the jail check and the `O_NOFOLLOW` flag, making the unsafe access explicit at the call site so reviewers and grep can find it.

#### slurp(path) / slurp(path, mode)

Reads the entire contents of a file. The optional second argument selects how
the bytes are interpreted:

- omitted — read as a UTF-8 string (the default).
- `"binary"` — read as a byte array (`Array<Int 0-255>`).
- a **charset label** (`"latin1"`, `"iso-8859-1"`, `"windows-1252"`, `"utf-8"`, …) —
  read the raw bytes and decode them from that encoding into a UTF-8 string.
  Use this to import a legacy (non-UTF-8) file without garbling accented
  characters. An unrecognized mode raises.

**Parameters:**
- `path` (String) - Path to the file
- `mode` (String, optional) - `"binary"` or a charset label

**Returns:** String (text/charset modes) or `Array<Int>` (binary mode); error on failure

**Example:**
```soli
content = slurp("config.json")           # UTF-8 string
bytes   = slurp("logo.png", "binary")    # Array<Int>
legacy  = slurp("clients.csv", "latin1") # Latin-1 file -> UTF-8 string
```

#### barf(path, content)

Writes content to a file (overwrites existing). Accepts either a string (its
UTF-8 bytes are written) or a byte array (`Array<Int 0-255>`) — pair it with
`Encoding.encode(...)` to write a file back out in a legacy charset.

**Parameters:**
- `path` (String) - Path to the file
- `content` (String or `Array<Int>`) - Content to write

**Returns:** null

**Example:**
```soli
barf("output.txt", "Hello, World!")
barf("clients.csv", Encoding.encode(text, "latin1"))  # export as Latin-1
```

#### File.read(path) / File.read(path, encoding)

Reads a file through the `File` class (jailed). Without an encoding it returns
a UTF-8 string; with a charset label (`"latin1"`, etc.) it decodes the raw
bytes from that encoding. `Trusted.read` mirrors this for unjailed access.

```soli
text   = File.read("notes.txt")            # UTF-8
legacy = File.read("clients.csv", "latin1") # Latin-1 -> UTF-8
```

#### Trusted.* — unjailed file access

`Trusted` mirrors the entire `File` API (`Trusted.read`, `Trusted.write`,
`Trusted.append`, `Trusted.exists`, `Trusted.is_dir`, `Trusted.glob`, …) but
**skips the filesystem jail and the `O_NOFOLLOW` symlink guard**. Reach for it
only from server-side code that legitimately must step outside `<dir>` or follow
a symlink — log shippers, backup scripts, cron-style maintenance jobs — and
never with a path derived from request input. Because the class name is explicit
at the call site, reviewers and `grep` can find every unjailed access, and the
`smell/dangerous-server-builtin` lint flags `Trusted.*` calls made from
`app/controllers/`, `app/middleware/`, or `app/views/`.

```soli
log = Trusted.read("/var/log/app/today.log")    # absolute path, outside the jail
Trusted.append("/var/log/app/audit.log", line)  # follows a symlinked logfile
```

---

### Math Functions

Math operations are available as methods on numbers:

```soli
(-5).abs       # 5
16.sqrt        # 4.0
2.pow(10)      # 1024
[3, 7].min     # 3
[3, 7].max     # 7
```

See the [Number Methods](#number-methods) and [Math Class](#math-class) sections for more details.

#### clock()

Returns the current Unix timestamp as a float with sub-second precision.

**Returns:** Float - Unix timestamp

**Example:**
```soli
start = clock()
# ... do work ...
elapsed = clock() - start
println("Took " + str(elapsed) + " seconds")
```

---

## HTTP Functions

All HTTP request functions are exposed on the `HTTP` class. The standalone
`http_get` / `http_post` / `http_request` etc. helpers were removed in favor of
this class-based API.

> **Security — SSRF blocklist & redirects.** Every URL passed to `HTTP.*` is
> validated up-front: schemes other than `http`/`https` are rejected, as are
> hosts that resolve to loopback/private/link-local IP ranges (so a request to
> `http://169.254.169.254/...` for cloud metadata, or `http://10.0.0.1/`, fails
> immediately). Auto-redirects are **not** followed by the synchronous
> `HTTP.get` / `HTTP.post` / `HTTP.request` paths — a 3xx response is returned
> as-is so a redirect-controlled `Location` cannot bypass the blocklist.
> Asynchronous and Model-driven HTTP (the reqwest-backed paths) follow redirects
> with a custom policy that re-runs the SSRF check on every hop. Apps that need
> to follow a 3xx from `HTTP.get` should inspect `response["status"]` and
> `response["headers"]["location"]` and re-issue the request manually.

### HTTP.get(url, options?)

Performs an HTTP GET request.

**Parameters:**
- `url` (String) - The URL to fetch
- `options` (Hash, optional) - Request options
  - `headers` (Hash) - Custom headers
  - `timeout` (Int|Float) - Per-call timeout in **seconds**, overriding the
    default 30s client timeout for this request only. Fractional seconds are
    allowed (`0.5`). Must be positive.

**Returns:** Hash - `{ "status": Int, "body": String, "headers": Hash }`

**Example:**
```soli
response = HTTP.get("https://api.example.com/data")
if response["status"] == 200
  println(response["body"])
end

# Give a slow upstream at most 5 seconds before giving up.
fast = HTTP.get("https://api.example.com/slow", { "timeout": 5 })
```

### HTTP.post(url, body, options?)

Performs an HTTP POST request.

**Parameters:**
- `url` (String) - The URL to post to
- `body` (String|Hash) - The request body
- `options` (Hash, optional) - Request options
  - `timeout` (Int|Float) - Per-call timeout in seconds (see `HTTP.get`)

**Returns:** Hash - `{ "status": Int, "body": String, "headers": Hash }`

**Example:**
```soli
response = HTTP.post(
  "https://api.example.com/users",
  "name=Alice&email=alice@example.com",
  { "headers": { "Content-Type": "application/x-www-form-urlencoded" } }
)

# A per-call timeout works on every HTTP.* method.
response = HTTP.post("https://api.example.com/users", { "name": "Alice" }, { "timeout": 10 })
```

### HTTP.post_json(url, data, options?)

Performs an HTTP POST request with JSON body.

**Parameters:**
- `url` (String) - The URL to post to
- `data` (Hash|Array) - Data to serialize as JSON
- `options` (Hash, optional) - Additional options

**Returns:** Hash - Response with parsed JSON body if applicable

**Example:**
```soli
response = HTTP.post_json(
  "https://api.example.com/users",
  { "name": "Alice", "email": "alice@example.com" }
)
```

### HTTP.get_json(url, options?)

Performs an HTTP GET request and parses JSON response.

**Parameters:**
- `url` (String) - The URL to fetch
- `options` (Hash, optional) - Request options

**Returns:** Hash - Response with parsed JSON body

**Example:**
```soli
data = HTTP.get_json("https://api.example.com/users/1")
println(data["body"]["name"])
```

### HTTP.get_jsonp(url, options?)

Performs an HTTP GET against a JSONP endpoint and unwraps the `callback(...)`
padding, returning the parsed value. Use it to consume legacy cross-origin APIs
that only expose JSONP. The `?callback=...` name goes in the URL you pass; the
JavaScript wrapper (and an optional leading `/**/` guard) is stripped before the
inner JSON is parsed.

**Parameters:**
- `url` (String) - The JSONP URL (typically containing a `callback` query param)
- `options` (Hash, optional) - Request options (e.g. `{ "timeout": 5 }`)

**Returns:** Any - The parsed value (Hash, Array, …). Raises if the body is not a
valid `callback(...)` wrapper or the inner JSON is malformed.

**Example:**
```soli
feed = HTTP.get_jsonp("https://api.example.com/feed?callback=cb")
println(feed["items"][0])
```

### HTTP.put(url, body, options?) / HTTP.patch(url, body, options?) / HTTP.delete(url, options?) / HTTP.head(url, options?)

PUT / PATCH / DELETE / HEAD counterparts to `HTTP.get` and `HTTP.post`. JSON
variants (`HTTP.put_json`, `HTTP.patch_json`) serialize the body automatically.

### HTTP.request(method, url, headers?, body?)

Performs a custom HTTP request.

**Parameters:**
- `method` (String) - HTTP method (GET, POST, PUT, PATCH, DELETE, etc.)
- `url` (String) - The URL
- `headers` (Hash, optional) - Request headers. A `timeout` key (Int|Float
  seconds) in this hash is consumed as the per-call timeout rather than being
  sent as a header.
- `body` (String|Hash, optional) - The request body

**Returns:** Hash - Response object

**Example:**
```soli
response = HTTP.request("DELETE", "https://api.example.com/users/1")

# Custom headers plus a 3-second per-call timeout.
response = HTTP.request("GET", "https://api.example.com/slow", {
  "Authorization": "Bearer " + token,
  "timeout": 3
})
```

### HTTP Status Helpers

#### http_ok(response)

Checks if response status is 200.

**Example:**
```soli
if http_ok(response)
  println("Success!")
end
```

#### http_success(response)

Checks if response status is 2xx.

#### http_redirect(response)

Checks if response status is 3xx.

#### http_client_error(response)

Checks if response status is 4xx.

#### http_server_error(response)

Checks if response status is 5xx.

### HTTP.get_all(urls, options?)

Performs multiple GET requests in parallel.

**Parameters:**
- `urls` (Array) - Array of URLs to fetch
- `options` (Hash, optional) - Request options applied to every request in the
  batch
  - `timeout` (Int|Float) - Per-call timeout in seconds (see `HTTP.get`)

**Returns:** Array - Array of response bodies as strings (or `{"error": ...}` hashes for failed requests)

**Example:**
```soli
responses = HTTP.get_all([
  "https://api.example.com/users",
  "https://api.example.com/posts"
])

# Cap every request in the batch at 5 seconds.
responses = HTTP.get_all([
  "https://api.example.com/users",
  "https://api.example.com/posts"
], { "timeout": 5 })
```

### HTTP.get_all_json(urls, options?)

Performs multiple GET requests in parallel and parses each response body as JSON. Equivalent to mapping `HTTP.get_json` over an array of URLs, but executed concurrently.

**Parameters:**
- `urls` (Array) - Array of URLs to fetch
- `options` (Hash, optional) - Request options applied to every request in the
  batch (`timeout`, in seconds)

**Returns:** Array - Array of parsed JSON values (or `{"error": ...}` hashes for failed requests / non-2xx responses / unparseable bodies)

**Example:**
```soli
responses = HTTP.get_all_json([
  "https://api.example.com/users.json",
  "https://api.example.com/posts.json"
])

users = responses[0]
posts = responses[1]
if posts.has_key("error") {
  print("posts failed: " + posts["error"])
}
```

### HTTP.parallel(requests)

Performs multiple custom requests in parallel.

**Parameters:**
- `requests` (Array) - Array of request hashes with `method`, `url`, optional
  `headers`, optional `body`, and an optional per-request `timeout` (Int|Float
  seconds)

**Returns:** Array - Array of response objects

**Example:**
```soli
responses = HTTP.parallel([
  { "method": "GET", "url": "https://api.example.com/users", "timeout": 5 },
  { "method": "POST", "url": "https://api.example.com/logs", "body": "{}" }
])
```

## S3 Functions

The S3 class provides static methods for interacting with Amazon S3 and S3-compatible storage (MinIO, DigitalOcean Spaces, etc.).

### Configuration

Credentials are loaded from environment variables automatically at app startup from `.env` files.

**Environment Variables:**
- `AWS_ACCESS_KEY_ID` or `S3_ACCESS_KEY` - Access key
- `AWS_SECRET_ACCESS_KEY` or `S3_SECRET_KEY` - Secret key  
- `AWS_REGION` or `S3_REGION` - Region (default: `us-east-1`)
- `S3_ENDPOINT` - Custom endpoint for S3-compatible services (optional)

**Example .env:**
```bash
S3_ACCESS_KEY=your_access_key
S3_SECRET_KEY=your_secret_key
S3_REGION=us-east-1
# For MinIO or other S3-compatible storage:
# S3_ENDPOINT=http://localhost:9000
```

### S3.list_buckets()

Lists all buckets in the S3 account.

**Returns:** Array of bucket names

**Example:**
```soli
buckets = S3.list_buckets()
print(buckets)  # ["bucket1", "bucket2"]
```

### S3.create_bucket(name)

Creates a new bucket.

**Parameters:**
- `name` (String) - Bucket name

**Returns:** Boolean - `true` on success

**Example:**
```soli
S3.create_bucket("my-app-files")
```

### S3.delete_bucket(name)

Deletes a bucket.

**Parameters:**
- `name` (String) - Bucket name

**Returns:** Boolean - `true` on success

**Example:**
```soli
S3.delete_bucket("my-app-files")
```

### S3.put_object(bucket, key, body, options?)

Uploads an object to S3.

**Parameters:**
- `bucket` (String) - Bucket name
- `key` (String) - Object key (path in bucket)
- `body` (String) - Object content
- `options` (Hash, optional) - Additional options
  - `content_type` (String) - Content MIME type (default: `application/octet-stream`)

**Returns:** Boolean - `true` on success

**Example:**
```soli
# Simple upload
S3.put_object("my-bucket", "hello.txt", "Hello World!")

# With content type
S3.put_object("my-bucket", "data.json", '{"key": "value"}', {
  "content_type": "application/json"
})
```

### S3.get_object(bucket, key)

Downloads an object from S3.

**Parameters:**
- `bucket` (String) - Bucket name
- `key` (String) - Object key

**Returns:** String - Object content

**Example:**
```soli
content = S3.get_object("my-bucket", "hello.txt")
print(content)  # "Hello World!"
```

### S3.delete_object(bucket, key)

Deletes an object from S3.

**Parameters:**
- `bucket` (String) - Bucket name
- `key` (String) - Object key

**Returns:** Boolean - `true` on success

**Example:**
```soli
S3.delete_object("my-bucket", "hello.txt")
```

### S3.list_objects(bucket, prefix?)

Lists objects in a bucket.

**Parameters:**
- `bucket` (String) - Bucket name
- `prefix` (String, optional) - Filter by prefix

**Returns:** Array of object keys

**Example:**
```soli
# List all objects
files = S3.list_objects("my-bucket")

# List objects with prefix
files = S3.list_objects("my-bucket", "documents/")
```

### S3.copy_object(source, dest)

Copies an object within S3.

**Parameters:**
- `source` (String) - Source in format `bucket/key`
- `dest` (String) - Destination in format `bucket/key`

Both bucket names must match `^[a-z0-9.-]{3,63}$` — lowercase letters, digits, `.` and `-`, 3 to 63 characters. Names containing other characters are rejected before any S3 call to prevent header injection (e.g. an attacker-supplied `bucket?versionId=evil` cannot redirect the copy to a different version).

**Returns:** Boolean - `true` on success

**Example:**
```soli
S3.copy_object("source-bucket/file.txt", "dest-bucket/file.txt")
```

## POP3 (Email Reading)

The `Pop3` class reads email from a mailbox over POP3. It connects over implicit
TLS by default (port `995`) and parses each message into a structured hash.

### Pop3.new(host, user, password, opts?)

Connect and authenticate, returning a client instance. The optional `opts` hash
accepts `port` (default `995`) and `tls` (default `true`).

```soli
mail = Pop3.new("pop.gmail.com", "me@gmail.com", "app-password")

# Plaintext on a custom port (e.g. a local test server)
mail = Pop3.new("127.0.0.1", "user", "pass", { "port": 110, "tls": false })
```

> **Gmail / 2FA accounts:** use an [App Password](https://support.google.com/accounts/answer/185833),
> not your normal password, and enable POP in the account settings.

### Instance methods

| Method | Returns |
|--------|---------|
| `mail.stat()` | `{ "count": Int, "size": Int }` — message count and total octets |
| `mail.list()` | `[ { "id": Int, "size": Int }, ... ]` — per-message sizes |
| `mail.fetch(id)` | A parsed message hash (see below) |
| `mail.fetch_all()` | An array of parsed message hashes |
| `mail.delete(id)` | `true` — marks the message for deletion (applied on `quit`) |
| `mail.quit()` | `true` — commits deletions and closes the connection |

`fetch_all()` is capped at 200 messages by default; raise it with the
`SOLI_POP3_MAX_MESSAGES` environment variable.

### Message hash shape

```soli
{
  "id":           1,
  "size":         2048,
  "subject":      "Hello from Alice",
  "from":         { "name": "Alice", "address": "alice@example.com" },
  "to":           [ { "name": "Bob", "address": "bob@example.com" } ],
  "date":         "2026-06-01T10:00:00Z",
  "text_body":    "Hi Bob, ...",
  "html_body":    "<p>Hi Bob, ...</p>",
  "attachments":  [ { "name": "report.pdf", "content_type": "application/pdf", "size": 51200 } ],
  "raw":          "From: Alice ..."   # full RFC822 source
}
```

`from` is a single `{name, address}` hash (or `null`); `to` is an array of them.
Missing headers/bodies are `null`. Attachment `content_type` is `type/subtype`.

### Example

```soli
mail = Pop3.new("pop.gmail.com", "me@gmail.com", "app-password")

print("You have #{mail.stat()["count"]} messages")

for msg in mail.fetch_all()
  print("#{msg["date"]} — #{msg["from"]["address"]}: #{msg["subject"]}")
end

mail.quit()
```

## IMAP (Email Reading)

The `Imap` class reads email over IMAP4rev1. Unlike POP3, IMAP is stateful and
server-side: you `select()` a mailbox, then `search()` and `fetch()` within it,
leaving the messages on the server. It connects over implicit TLS by default
(port `993`) and parses each message into the same structured hash as `Pop3`.

### Imap.new(host, user, password, opts?)

Connect and authenticate, returning a client instance. The optional `opts` hash
accepts `port` (default `993`) and `tls` (default `true`).

```soli
mail = Imap.new("imap.gmail.com", "me@gmail.com", "app-password")

# Plaintext on a custom port (e.g. a local test server)
mail = Imap.new("127.0.0.1", "user", "pass", { "port": 143, "tls": false })
```

> **Gmail / 2FA accounts:** use an [App Password](https://support.google.com/accounts/answer/185833),
> not your normal password, and enable IMAP in the account settings.

### Instance methods

| Method | Returns |
|--------|---------|
| `mail.select(mailbox = "INBOX")` | Mailbox status: `{ "mailbox", "exists", "recent", "unseen", "uidvalidity", "uidnext", "flags" }`. Selects it for subsequent calls. |
| `mail.mailboxes()` | `[ { "name", "delimiter", "flags" }, ... ]` — all mailboxes/folders |
| `mail.search(criteria = "ALL")` | Array of **sequence numbers** matching an IMAP search key |
| `mail.uid_search(criteria = "ALL")` | Array of **UIDs** matching an IMAP search key |
| `mail.fetch(seq)` | A parsed message hash for the given sequence number |
| `mail.fetch_uid(uid)` | A parsed message hash for the given UID |
| `mail.fetch_all()` | An array of parsed message hashes from the selected mailbox |
| `mail.mark_seen(seq)` / `mail.mark_unseen(seq)` | Toggle the `\Seen` flag; returns `true` |
| `mail.delete(seq)` | Marks the message `\Deleted` (removed on `expunge`); returns `true` |
| `mail.expunge()` | Permanently removes `\Deleted` messages; returns `true` |
| `mail.copy(seq, mailbox)` | Copies the message into another mailbox; returns `true` |
| `mail.move(seq, mailbox)` | Moves the message (RFC 6851 `MOVE`); returns `true` |
| `mail.logout()` | Closes the connection; returns `true` |

`fetch_all()` requires a prior `select()` and is capped at 200 messages by
default; raise it with the `SOLI_IMAP_MAX_MESSAGES` environment variable.
`fetch`/`fetch_uid` use `BODY.PEEK[]`, so reading a message does **not** mark it
`\Seen` — call `mark_seen()` explicitly if you want that.

### Search criteria

`criteria` is passed straight through as an IMAP search key, so any standard
expression works:

```soli
mail.uid_search("UNSEEN")                       # unread
mail.search("FROM alice@example.com")           # by sender
mail.search("SINCE 1-Jun-2026 SUBJECT invoice") # combine keys
mail.search("ALL")                              # everything (the default)
```

### Message hash shape

Fetched messages carry the same fields as `Pop3` plus IMAP identity fields
(`seq`, `uid`, `flags`):

```soli
{
  "seq":          1,
  "uid":          4821,
  "flags":        ["\\Seen", "\\Answered"],
  "size":         2048,
  "subject":      "Hello from Alice",
  "from":         { "name": "Alice", "address": "alice@example.com" },
  "to":           [ { "name": "Bob", "address": "bob@example.com" } ],
  "date":         "2026-06-01T10:00:00Z",
  "text_body":    "Hi Bob, ...",
  "html_body":    "<p>Hi Bob, ...</p>",
  "attachments":  [ { "name": "report.pdf", "content_type": "application/pdf", "size": 51200 } ],
  "raw":          "From: Alice ..."   # full RFC822 source
}
```

### Example

```soli
mail = Imap.new("imap.gmail.com", "me@gmail.com", "app-password")

info = mail.select("INBOX")
print("#{info["exists"]} messages, #{info["unseen"]} unread")

# Fetch and archive every unread message
for uid in mail.uid_search("UNSEEN")
  msg = mail.fetch_uid(uid)
  print("#{msg["date"]} — #{msg["from"]["address"]}: #{msg["subject"]}")
  mail.mark_seen(msg["seq"])
end

mail.logout()
```

## JSON Functions

### json_parse(string)

Parses a JSON string into a Soli value.

**Parameters:**
- `string` (String) - JSON string to parse

**Returns:** Any - Parsed value (Hash, Array, String, Int, Float, Bool, or null)

**Example:**
```soli
data = json_parse('{"name": "Alice", "age": 30}')
println(data["name"])  # Alice
```

### json_stringify(value)

Converts a Soli value to a JSON string.

**Parameters:**
- `value` (Any) - Value to serialize

**Returns:** String - JSON representation

**Example:**
```soli
json = json_stringify({ "name": "Alice", "scores": [95, 87, 92] })
println(json)  # {"name":"Alice","scores":[95,87,92]}
```

---

## AI Functions

Embedding generation and LLM text completion against OpenAI-compatible
endpoints. Endpoints and API keys are read from the environment, so credentials
stay out of app code and there is one place to review where text is sent (a
single point for GDPR / data-residency review). Any OpenAI-compatible server
works — OpenAI, or a self-hosted vLLM / Ollama / llama.cpp (which often need no
API key).

### embed(text)

Generate an embedding vector for a string — the write-side counterpart to
`Model.similar`, which embeds the *query* but not the documents you store.

**Parameters:**
- `text` (String) - text to embed

**Returns:** Array<Float> - the embedding vector

**Environment:** `SOLI_EMBEDDING_API_KEY` (required), `SOLI_EMBEDDING_URL`
(default `https://api.openai.com/v1/embeddings`), `SOLI_EMBEDDING_MODEL`
(default `text-embedding-3-small`). Raises if the key is unset or the call fails.

**Example:**
```soli
class Article < Model
  vector_index "embedding", dimension: 1536, metric: "cosine"

  before_save fn() {
    this.embedding = embed(this.title + "\n" + this.body)
  }
end
```

### embed_batch(texts)

Embed many texts in a single request, returned in input order. Use it to
back-fill embeddings over an existing collection instead of one call per row.

**Parameters:**
- `texts` (Array<String>) - texts to embed

**Returns:** Array<Array<Float>> - one vector per input, in input order

**Example:**
```soli
articles = Article.where({ "embedding": null }).all
vectors  = embed_batch(articles.map(fn(a) a.title))
articles.each_with_index(fn(article, i) {
  article.embedding = vectors[i]
  article.save()
})
```

### llm_generate(system, user)

Chat completion via an OpenAI-compatible `chat/completions` endpoint.

**Parameters:**
- `system` (String) - system prompt (role/instructions)
- `user` (String) - user prompt

**Returns:** String - the model's completion text

**Environment:** `SOLI_LLM_URL` (default
`https://api.openai.com/v1/chat/completions`), `SOLI_LLM_API_KEY` (optional —
omitted from the request when unset, for keyless local servers), `SOLI_LLM_MODEL`
(default `gpt-4o-mini`), `SOLI_LLM_TEMPERATURE` and `SOLI_LLM_MAX_TOKENS`
(optional — only sent when set). Raises if the call fails.

**Example:**
```soli
summary = llm_generate(
  "You summarize support tickets in one sentence.",
  ticket.body
)
```

---

## Cryptography Functions

All cryptographic functions are available both as static methods on the `Crypto` class and as standalone functions.

### Hash Functions

> **⚠ Not for password storage.** SHA-256, SHA-512, and MD5 are general-purpose hashes — they are fast by design, which is exactly the wrong property for a password store (an attacker brute-forces them just as fast). For passwords, use [`Crypto.argon2_hash`](#cryptoargon2_hashpassword--argon2_hashpassword). For verifying tokens / MACs in constant time, use [`secure_compare`](#secure_compareab--cryptosecure_compareab).

#### Crypto.sha256(data) / sha256(data)

Computes SHA-256 hash of a string. Use for file checksums, ETags, content addressing — **not** for password hashing.

**Parameters:**
- `data` (String) - The data to hash

**Returns:** String - 64-character hex string (32 bytes)

**Example:**
```soli
hash = Crypto.sha256("hello")
# "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
```

#### Crypto.sha512(data) / sha512(data)

Computes SHA-512 hash of a string. Same caveats as `sha256` — **not** for password hashing.

**Parameters:**
- `data` (String) - The data to hash

**Returns:** String - 128-character hex string (64 bytes)

**Example:**
```soli
hash = Crypto.sha512("hello")
```

#### Crypto.md5(data) / md5(data)

Computes MD5 hash of a string. **Cryptographically broken** — collisions can be constructed cheaply. Use only for non-security checksums (e.g. content fingerprinting where adversarial collisions don't matter). **Never use for passwords or signatures.**

**Parameters:**
- `data` (String) - The data to hash

**Returns:** String - 32-character hex string (16 bytes)

**Example:**
```soli
hash = Crypto.md5("hello")
# "5d41402abc4b2a76b9719d911017c592"
```

#### Crypto.hmac(message, key) / hmac(message, key)

Computes HMAC-SHA256 message authentication code.

**Parameters:**
- `message` (String) - The message to authenticate
- `key` (String) - The secret key

**Returns:** String - 64-character hex string (32 bytes)

**Example:**
```soli
mac = Crypto.hmac("message", "secret_key")
# Use for API signature verification, webhook validation, etc.
```

#### secure_compare(a, b) / Crypto.secure_compare(a, b)

Constant-time string equality. Use whenever comparing two values where one comes from an untrusted source and timing-leak of the comparison would help an attacker — verifying an HMAC, a webhook signature, a CSRF token, a session-derived MAC. A naïve `a == b` short-circuits at the first differing byte and leaks the prefix length to a timing attacker.

**Parameters:**
- `a` (String)
- `b` (String)

**Returns:** Bool - `true` only if both strings have the same length **and** bytes. Length is not secret; equal-length inputs run in time proportional only to the length, not the position of any differing byte.

**Example:**
```soli
expected = Crypto.hmac(payload, getenv("WEBHOOK_SECRET"))
if secure_compare(expected, request_signature)
  # Trust the request
end
```

### Secure Random

Cryptographically secure random values drawn from the operating system entropy
source. Do **not** use `Math.random` for anything security-bearing — it is a
general-purpose PRNG, not a CSPRNG.

All three take a **byte** count, not a character count, and reject anything
outside `1..=1024`.

#### Crypto.random_hex(n)

**Parameters:**
- `n` (Int) - number of random **bytes**

**Returns:** String - `2n` lowercase hex characters.

`Crypto.random_hex(32)` produces a 64-character string, matching what
`openssl rand -hex 32` gives you — the form expected for `SOLI_ENCRYPTION_KEY`
and `JWT_SECRET`.

#### Crypto.random_bytes(n)

**Returns:** Array - `n` integers in `0..=255`. Compose with
`Base64.urlsafe_encode` or `Hex.encode` for other representations.

#### Crypto.random_token(n = 32)

**Returns:** String - unpadded URL-safe Base64 of `n` random bytes.

The default 32 bytes gives 256 bits of entropy in 43 URL-safe characters. This
is the right primitive for OAuth `state`, PKCE verifiers, authorization codes,
refresh tokens, and any opaque identifier that travels in a URL.

```soli
state = Crypto.random_token()
session_set("oauth_state", state)
```

### Tamper-Evidence (Hash Chains & Merkle Trees)

Two building blocks for verifiable, append-only data — audit logs, provenance
trails, and hash-chained ledgers.

#### Crypto.canonical_json(value)

Serializes a value to **canonical JSON** — object keys sorted lexicographically,
recursively — so the same logical content always produces the same bytes, and
therefore the same hash. Ordinary JSON serialization (`.to_json`) preserves
insertion order, which is not stable enough to hash; use this whenever you hash a
structured value.

**Parameters:**
- `value` (Any) - A JSON-shaped value (Hash, Array, String, Int, Float, Bool, null). Opaque values (functions, class instances, …) raise.

**Returns:** String - Deterministic JSON text.

**Example:**
```soli
Crypto.canonical_json({ "b": 1, "a": 2 })   # {"a":2,"b":1}
Crypto.canonical_json({ "a": 2, "b": 1 })   # {"a":2,"b":1}  (same bytes)

# Stable content hash of a record:
record = { "amount": 100, "to": "alice" }
hash = Crypto.sha256(Crypto.canonical_json(record))
```

#### Crypto.merkle_root(hashes)

Computes the **Merkle root** of an array of hex leaf hashes — a single hash that
proves the entire set. Nodes are combined pairwise as `sha256(left ‖ right)`; an
odd node is paired with itself. An empty array hashes the empty string; a single
leaf is its own root. The root changes if any leaf changes or is reordered.

**Parameters:**
- `hashes` (Array<String>) - Hex hash strings (e.g. each record's `sha256`).

**Returns:** String - 64-character hex Merkle root.

**Example:**
```soli
leaves = records.map(fn(r) Crypto.sha256(Crypto.canonical_json(r)))
root = Crypto.merkle_root(leaves)
# Publish `root` to prove the set is intact; re-derive it later to detect tampering.
```

#### Crypto.ledger_hash(prev_hash, seq, data)

The leaf hash of a hash-chained ledger record — a one-call shorthand for
`Crypto.sha256(prev_hash + ":" + str(seq) + ":" + Crypto.canonical_json(data))`.
Because it's a single definition, the code that writes records and the code that
verifies them share the exact same formula and can't drift apart.

**Parameters:**
- `prev_hash` (String) - The previous record's `hash` (or 64 zeros for the genesis record)
- `seq` (Int) - The record's monotonic sequence number
- `data` (Hash) - The record's user fields (canonicalized internally)

**Returns:** String - 64-character hex hash committing to the previous record, the sequence, and the content.

**Example:**
```soli
prev = "0000000000000000000000000000000000000000000000000000000000000000"
hash = Crypto.ledger_hash(prev, 0, { "amount": 100, "to": "alice" })
```

> **Tamper-evident ledgers.** Chain records so each commits to the one before it
> (`Crypto.ledger_hash(prev_hash, seq, data)`) — any later edit or deletion breaks
> the chain and is detectable by recomputing it. See the blog post
> [Tamper-Evident Audit Logs in Soli](/docs/blog/tamper-evident-ledgers) for a
> complete, verifiable example.

### Base64 Encoding

Base64 encoding and decoding is available via the **Base64 class**:

- `Base64.encode(data)` - Encodes a string to Base64
- `Base64.decode(data)` - Decodes a Base64 string
- `Base64.urlsafe_encode(data)` - Encodes to **unpadded** URL-safe Base64 (RFC 4648 §5)
- `Base64.urlsafe_decode(data)` - Decodes URL-safe Base64, with or without padding

The URL-safe variant uses `-` and `_` in place of `+` and `/` and never emits
`=` padding — the form required by JWS, JWK, PKCE and JWK thumbprints. Use it
whenever a value travels in a URL, a JWT, or a JSON Web Key:

```soli
# PKCE S256 challenge: sha256 returns hex, so decode to bytes before encoding.
challenge = Base64.urlsafe_encode(Hex.decode(Crypto.sha256(code_verifier)))
```

See the [Base64 class documentation](/docs/utility/base64) for details.

### Character Encodings (Charsets)

Soli strings are UTF-8. The **Encoding class** converts between UTF-8 and legacy
byte encodings (Latin-1 / ISO-8859-1 / Windows-1252, etc.), so you can import a
non-UTF-8 file without turning accented characters into `?`:

- `Encoding.decode(input, label)` - Decodes bytes (`Array<Int>`) or a string from
  `label` into a UTF-8 string.
- `Encoding.encode(string, label)` - Encodes a UTF-8 string into a byte array in
  `label`.

Labels follow the WHATWG Encoding Standard (`"latin1"`, `"iso-8859-1"`,
`"windows-1252"`, `"utf-8"`, …); `latin1`/`iso-8859-1` alias to `windows-1252`.
An unknown label raises.

```soli
# Import a Latin-1 file as UTF-8
raw  = slurp("clients.csv", "binary")
text = Encoding.decode(raw, "latin1")
# or in one step: slurp("clients.csv", "latin1")

# Export UTF-8 back to Latin-1
barf("clients.csv", Encoding.encode(text, "latin1"))
```

See the [Encoding class documentation](/docs/utility/encoding) for details.

### Password Hashing

#### Crypto.argon2_hash(password) / argon2_hash(password)

Hashes a password using Argon2id (recommended).

**Parameters:**
- `password` (String) - The password to hash

**Returns:** String - The hash string

**Example:**
```soli
hash = Crypto.argon2_hash("secretpassword")
# $argon2id$v=19$m=19456,t=2,p=1$...
```

#### Crypto.argon2_verify(password, hash) / argon2_verify(password, hash)

Verifies a password against an Argon2 hash.

**Parameters:**
- `password` (String) - The password to verify
- `hash` (String) - The stored hash

**Returns:** Bool - true if password matches

**Example:**
```soli
if Crypto.argon2_verify(user_input, stored_hash)
  println("Password correct!")
end
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
keypair = Crypto.x25519_keypair()
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
# Alice
alice = Crypto.x25519_keypair()

# Bob
bob = Crypto.x25519_keypair()

# Both compute the same shared secret
alice_secret = Crypto.x25519_shared_secret(alice["private"], bob["public"])
bob_secret = Crypto.x25519_shared_secret(bob["private"], alice["public"])
# alice_secret == bob_secret
```

### Ed25519 Signatures

#### Crypto.ed25519_keypair() / ed25519_keypair()

Generates a new Ed25519 signing key pair.

**Returns:** Hash - `{ "public": String, "private": String }` (hex-encoded, 64 chars each)

### XML Digital Signature Primitives

Low-level building blocks for RSA signatures and XML-DSig / SAML / WS-Security:
2048-bit modular exponentiation, PKCS#1 v1.5 padding, and exclusive XML
canonicalization. All octet inputs are hex strings (an optional `0x` prefix is
allowed) or arrays of byte-valued `Int`s; results are returned as hex strings.

> **⚠ These are primitives, not a turn-key signer.** They give you `m^e mod n`,
> the PKCS#1 padding frame, and a canonical byte stream — you compose the digest,
> the `DigestInfo`, and reference resolution yourself. Use a vetted library if one
> is available for your platform.

#### Crypto.modexp(base, exp, modulus)

Computes `base^exp mod modulus` over big-endian octet strings. The result is
left-padded with zero octets to the modulus width (`k = ceil(bits(modulus)/8)`),
matching the RSA convention where a signature or ciphertext is always `k` octets
wide — so the output drops straight into the PKCS#1 helpers.

**Returns:** String — big-endian hex, `k` octets wide. Errors if the modulus is zero.

```soli
# 4^13 mod 497 = 445 (0x01bd); modulus 0x01f1 is 2 octets, so output is 2 octets
Crypto.modexp("04", "0d", "01f1")   # => "01bd"
```

#### Crypto.pkcs1_pad(data, key_size, block_type?)

Applies PKCS#1 v1.5 padding (RFC 8017): `EM = 0x00 || BT || PS || 0x00 || data`,
producing an encoded message exactly `key_size` octets long. `block_type` defaults
to `1` (signature padding — `PS` is all `0xFF`); pass `2` for encryption padding
(`PS` is random non-zero octets). `data` must be at most `key_size - 11` octets.

**Returns:** String — the `key_size`-octet encoded message as hex.

#### Crypto.pkcs1_unpad(encoded_message)

Strips PKCS#1 v1.5 padding, returning the embedded data. Validates the
`0x00 || BT` prefix, the minimum 8-octet padding string, and (for block type 1)
that every padding octet is `0xFF`.

**Returns:** String — the recovered data octets as hex. Errors on malformed padding.

```soli
# RSA sign/verify round-trip: sign a digest with the private exponent d,
# verify by raising the signature to the public exponent e.
digest      = Crypto.sha256(canonical_xml)        # 32-byte hex digest
em          = Crypto.pkcs1_pad(digest, key_octets) # block type 1
signature   = Crypto.modexp(em, rsa_d, rsa_n)      # sign

recovered   = Crypto.pkcs1_unpad(Crypto.modexp(signature, rsa_e, rsa_n))
assert(recovered == digest)
```

#### Xml.c14n_exclusive(xml, inclusive_prefixes_or_options?)

Canonicalizes an XML document with the W3C **Exclusive XML Canonicalization 1.0**
algorithm (`http://www.w3.org/2001/10/xml-exc-c14n#`) — the canonical form
signed by XML-DSig. Empty elements expand to start/end pairs, attributes and
namespace declarations are sorted, the XML declaration and comments are removed,
and (the "exclusive" part) namespace declarations that are not visibly utilized
are dropped rather than inherited from ancestors. `DOCTYPE` declarations are
rejected (XXE defense).

The optional second argument is either the **InclusiveNamespaces PrefixList**
directly — a space-separated string (e.g. `"ds saml"`) or an array of prefixes,
`#default` selecting the default namespace — or an **options hash**:

| Key | Type | Meaning |
|-----|------|---------|
| `inclusive_prefixes` | String / Array | as above |
| `id` | String | canonicalize only the subtree of the element whose `Id`/`ID`/`id` attribute matches (inheriting the ancestor namespace context) — resolves a `Reference URI="#..."` |
| `enveloped_signature` | Bool | drop any descendant `<ds:Signature>` (the enveloped-signature transform) |

**Returns:** String — the canonical UTF-8 serialization.

```soli
Xml.c14n_exclusive("<?xml version=\"1.0\"?><doc b='2' a='1'/>")
# => "<doc a=\"1\" b=\"2\"></doc>"

# An ancestor namespace that the signed element doesn't use is dropped:
xml = "<n0:r xmlns:n0=\"http://a\" xmlns:n2=\"http://c\"><n1:e xmlns:n1=\"http://b\">x</n1:e></n0:r>"
Xml.c14n_exclusive(xml)
# => "<n0:r xmlns:n0=\"http://a\"><n1:e xmlns:n1=\"http://b\">x</n1:e></n0:r>"  (n2 removed)

# Canonicalize the SAML-referenced element with its signature stripped:
Xml.c14n_exclusive(saml_response, {"id": "_assertion1", "enveloped_signature": true})
```

> **Note:** comments are omitted (the no-comments form). See the source for full
> notes on attribute-value whitespace handling.

#### Xml.get_element_by_id(xml, id)

Returns the element whose `Id`/`ID`/`id` attribute equals `id`, serialized as a
standalone XML fragment with the inherited ancestor namespaces injected onto its
root (so it re-parses and re-canonicalizes identically to the in-context
subtree). Errors if no such element exists.

**Returns:** String — the element subtree as standalone XML.

#### Xml.get_elements_by_tag(xml, local_name)

Returns every element with the given local name (namespace prefix ignored),
each as a standalone XML fragment. Used to locate elements that have no `Id`,
such as `<ds:SignedInfo>`.

**Returns:** Array of String.

#### X509.public_key(cert)

Parses an X.509 certificate and extracts its RSA public key. Accepts PEM
(`-----BEGIN CERTIFICATE-----`), bare base64 (as found in a SAML metadata
`<ds:X509Certificate>`), hex, or a raw DER byte array.

**Returns:** Hash — `{ "algorithm": "RSA", "n": hex, "e": hex, "bits": Int }`.
The `n`/`e` hex strings drop straight into `Crypto.modexp`.

```soli
key = X509.public_key(idp_metadata_cert)
em  = Crypto.modexp(signature, key["e"], key["n"])   # RSA verify step
```

#### X509.fingerprint(cert, algorithm?)

Returns the certificate fingerprint (hash of the DER bytes) as hex. `algorithm`
is `"sha256"` (default) or `"sha1"`. Fingerprints the whole certificate; for TLS
pinning that survives renewal, pin the key with `X509.spki_pin` instead.

**Returns:** String — hex digest.

#### X509.spki_pin(cert)

The public-key pin for TLS certificate pinning:
`base64(SHA-256(SubjectPublicKeyInfo))`, returned as `"sha256/<base64>"` — the
form an Android Network Security Config `<pin-set>` or any HPKP-style pinner
expects.

It pins the **key**, not the certificate, which is what lets a pinned client
survive a certificate renewal: as long as the renewal reuses the key, the pin is
unchanged. Pin the certificate (or its `fingerprint`) instead and the client
breaks on every ~90-day rotation.

```soli
pin = X509.spki_pin(File.read("cert.pem"))
# => "sha256/UKm/R6MKhCiukXKhnWjBQSRBSWRwGQBLCCa/8w27Dxs="
```

> **Pinning is a footgun; treat it as one.** A wrong or lost pin **bricks the
> installed app** with no server-side fix. Always ship a **backup pin** (a
> second, offline key). For a public web app already on HSTS + Certificate
> Transparency, weigh whether the one threat it closes — a rogue or compromised
> CA — is worth the operational risk; browsers removed HPKP for this reason.
> Soli gives you the pin string but does not wire pinning into the shells by
> default.

**Returns:** String — `"sha256/<base64>"`.

#### Deflate.deflate(data) / Deflate.inflate(data)

Raw DEFLATE (RFC 1951, no zlib/gzip wrapper) — the compression used by the SAML
2.0 **HTTP-Redirect binding**. Byte conventions mirror `Base64`: `deflate`
takes a String (its UTF-8 bytes) or byte array and returns a byte array;
`inflate` returns a String when the result is valid UTF-8, else a byte array.
Designed to pipe through `Base64`:

```soli
# Inbound: decode a SAMLRequest from a redirect URL
xml = Deflate.inflate(Base64.decode(params["SAMLRequest"]))

# Outbound: encode one
param = Base64.encode(Deflate.deflate(authn_request_xml))
```

#### RsaKey.private_from_pem(pem)

Parses an RSA private key — PKCS#8 (`-----BEGIN PRIVATE KEY-----`) or PKCS#1
(`-----BEGIN RSA PRIVATE KEY-----`) PEM — so a Service Provider can **sign**
(the verification side, `X509.public_key`, only yields `(n, e)`).

**Returns:** Hash — `{ "algorithm": "RSA", "n": hex, "e": hex, "d": hex, "bits": Int }`.
Sign with the private exponent: `Crypto.modexp(padded, key["d"], key["n"])`.

#### RsaKey.public_from_pem(pem)

Parses a bare RSA **public** key — SPKI (`-----BEGIN PUBLIC KEY-----`) or
PKCS#1 (`-----BEGIN RSA PUBLIC KEY-----`) PEM. Use it when you hold a public
key rather than a certificate (`X509.public_key` covers that case): publishing
a JWKS, or verifying tokens signed by someone else.

**Returns:** Hash — `{ "algorithm": "RSA", "n": hex, "e": hex, "bits": Int }`.

```soli
# One JWKS entry
key = RsaKey.public_from_pem(getenv("SOLI_OIDC_PUBLIC_KEY"))
jwk = {
  "kty": "RSA", "use": "sig", "alg": "RS256",
  "n": Base64.urlsafe_encode(Hex.decode(key["n"])),
  "e": Base64.urlsafe_encode(Hex.decode(key["e"]))
}
```

#### Hex.encode(data) / Hex.decode(hex)

Bridges the hex world (`Crypto.modexp` / `sha256` / `pkcs1_*` all speak hex) and
the byte/base64 world (`Base64`, and XML-DSig's base64 `DigestValue` /
`SignatureValue`). `encode` takes a String/byte-array → hex; `decode` takes a hex
string (optional `0x` prefix) → byte array.

```soli
# hex digest -> base64 DigestValue
digest_value = Base64.encode(Hex.decode(Crypto.sha256(canonical_xml)))
# incoming base64 -> hex (to compare against a Crypto.* result)
Hex.encode(Base64.decode(incoming_b64))
```

#### Putting it together: signing an envelope

```soli
key = RsaKey.private_from_pem(sp_private_key_pem)

# 1. Digest the referenced element (enveloped transform; no Signature yet)
ref_canon  = Xml.c14n_exclusive(doc, {"id": "_obj1", "enveloped_signature": true})
digest_b64 = Base64.encode(Hex.decode(Crypto.sha256(ref_canon)))

# 2. Build SignedInfo (CanonicalizationMethod=exc-c14n, SignatureMethod=rsa-sha256,
#    Reference with enveloped+exc-c14n transforms and DigestValue=digest_b64), then sign it:
si_hash   = Crypto.sha256(Xml.c14n_exclusive(signed_info))
em        = Crypto.pkcs1_pad("3031300d060960864801650304020105000420" + si_hash, key["bits"] / 8)
sig_b64   = Base64.encode(Hex.decode(Crypto.modexp(em, key["d"], key["n"])))

# 3. Assemble <ds:Signature> (SignedInfo + <ds:SignatureValue>sig_b64</...> + KeyInfo)
#    and envelope it into the document. Verifies cleanly under any XML-DSig library.
```

#### Putting it together: verifying a SAML signature

```soli
# 1. Verify the Reference digest (enveloped transform + by-id exclusive c14n)
canonical = Xml.c14n_exclusive(saml, {"id": assertion_id, "enveloped_signature": true})
if Crypto.sha256(canonical) != digest_value_hex { return false }

# 2. Verify the signature over SignedInfo
signed_info = Xml.get_elements_by_tag(saml, "SignedInfo")[0]
si_hash     = Crypto.sha256(Xml.c14n_exclusive(signed_info))
key         = X509.public_key(idp_cert)
recovered   = Crypto.pkcs1_unpad(Crypto.modexp(signature_hex, key["e"], key["n"]))
# DigestInfo = SHA-256 DER prefix + the SignedInfo hash
recovered == "3031300d060960864801650304020105000420" + si_hash
```

### ID Generation

Four ID generators are available: UUID v4 / v7 (RFC 4122), ULID, and NanoID.

> **Which one?**
> - **`uuid_v7()` / `ulid()`** — time-sortable, ideal for DB primary keys. UUID v7
>   stays in the standard 36-char UUID shape; ULID is shorter (26 chars,
>   Crockford Base32) and case-insensitive.
> - **`uuid_v4()`** — fully random UUID. Use when you don't want creation time
>   leaking into the ID (public tokens, share links, anti-enumeration).
> - **`nanoid()`** — compact, URL-safe random IDs (default 21 chars). Customize
>   size and alphabet for short codes, slugs, or invite tokens.

#### UUID.v4() / uuid_v4()

Generates a random (version 4) UUID. 122 bits of randomness from the OS CSPRNG.

**Returns:** String — 36-character hyphenated UUID

**Example:**
```soli
id = uuid_v4()
# "c74d395d-5b75-41c3-b873-ca597e1ccaac"

token = UUID.v4()  # equivalent, via the UUID class
```

#### UUID.v7() / uuid_v7()

Generates a time-ordered (version 7) UUID. The high 48 bits are a Unix
millisecond timestamp; the remaining bits are random. Two UUIDs minted in the
same millisecond stay distinct, and any two UUIDs minted in different
milliseconds sort by creation time as strings.

**Returns:** String — 36-character hyphenated UUID

**Example:**
```soli
id = uuid_v7()
# "019e6ef2-ba58-7460-ad04-55e291a8c28b"
# Lexicographic order == creation order — great for DB primary keys.

# Using the UUID class
record_id = UUID.v7()
```

#### ULID.generate() / ulid()

Generates a ULID — a 128-bit identifier encoded as 26 Crockford Base32 chars
(`0-9 A-Z` minus `I L O U`). The high 48 bits are a Unix millisecond timestamp,
the remaining 80 bits are CSPRNG random. Sorts by creation time as a string and
is case-insensitive by spec. Shorter than a UUID, no dashes — friendly for URLs
and DB keys.

`ULID.new()` is provided as an alias for `ULID.generate()`.

**Returns:** String — 26-character ULID

**Example:**
```soli
id = ulid()
# "01KSQG6MAN1B4S05KRV29NWZPT"

record_id = ULID.generate()
```

#### NanoID.generate(size?, alphabet?) / nanoid(size?, alphabet?)

Generates a NanoID — a short, URL-safe, cryptographically random ID. Defaults
to 21 characters from the 64-char URL-safe alphabet (`A-Z a-z 0-9 _ -`), which
matches the original NanoID spec and is collision-resistant enough to replace
UUIDs in most contexts.

`NanoID.new(...)` is provided as an alias for `NanoID.generate(...)`.

**Parameters:**
- `size` (Int, optional) — Length of the generated ID. Must be 1-1024. Default: 21.
- `alphabet` (String, optional) — Character set to draw from. Must be 1-255
  unique characters. Default: URL-safe 64-char alphabet.

**Returns:** String — `size`-character random ID

**Example:**
```soli
id = nanoid()
# "XBwf0cjEQmwsx8YSQRCws"

short = nanoid(10)
# "hBbpwcRh4j"

# Custom alphabet for human-friendly short codes
# (Crockford-style, no easily-confused 0/O/1/I/L)
code = nanoid(8, "23456789ABCDEFGHJKMNPQRSTVWXYZ")
# "K7M3PQ2N"

slug = NanoID.generate(12)
```

> **Sizing tip:** with the default 64-char alphabet, 21 chars ≈ 126 bits of
> entropy — comparable to a v4 UUID. Drop to 12 chars (~71 bits) only when
> per-table uniqueness is enough; never go below 10 chars for anything
> security-sensitive.

---

## JWT Functions

### jwt_sign(payload, secret, options?)

Creates a signed JWT token.

**Parameters:**
- `payload` (Hash) - Claims to include in the token
- `secret` (String) - Secret key for signing. Must be at least **32 bytes** for HMAC algorithms (SEC-054). Asymmetric algorithms (RS256, EdDSA) use PEM keys via the `key` option instead. Load a high-entropy value from `.env`, e.g. generate it once with `openssl rand -hex 32` and reference it as `getenv("JWT_SECRET")`. Never commit the secret to source.
- `options` (Hash, optional) - Token options
  - `expires_in` (Int) - Expiration, in seconds from now
  - `algorithm` (String) - "HS256", "HS384", "HS512", "RS256", or "EdDSA"
  - `key` (String) - PEM-encoded private key for RS256/EdDSA algorithms
  - `kid` (String) - Key ID, written to the JWT **header**. Lets a verifier pick the right key out of a JWKS, which is what makes key rotation possible.
  - `typ` (String) - Overrides the header `typ` (default `"JWT"`). Set `"at+jwt"` for RFC 9068 access tokens.
  - `exp` (Int) - Expiration as an **absolute** Unix timestamp. Mutually exclusive with `expires_in` — supplying both raises, since they are different units and silently picking one would produce a token expiring at a time you never meant.
  - `nbf` (Int) - Not-before, as an absolute Unix timestamp
  - `aud` (String or Array) - Audience. RFC 7519 §4.1.3 allows either form.
  - `iss` (String) - Issuer
  - `jti` (String) - Unique token ID

Registered claims come from `options`; everything else in `payload` becomes a custom claim. `sub` is the exception — it is read from `payload`. An option always wins over a same-named key in the payload.

**Returns:** String - The JWT token

**Example:**
```soli
token = jwt_sign(
  { "sub": "user123", "role": "admin" },
  getenv("JWT_SECRET"),
  { "expires_in": 3600 }
)

# An OIDC id_token, signed with a rotatable key
id_token = jwt_sign(
  { "sub": user["_key"], "email": user["email"] },
  "",
  {
    "algorithm": "RS256",
    "key": getenv("SOLI_OIDC_PRIVATE_KEY"),
    "kid": active_kid,
    "iss": "https://op.example",
    "aud": client["client_id"],
    "expires_in": 600
  }
)
```

### jwt_verify(token, secret, options?)

Verifies and decodes a JWT token. **The verifier — not the token — chooses which algorithm is acceptable**, closing the classic JWT algorithm-confusion attack where an attacker who knew the verifier's RSA public key could sign an HS256 token using the public key bytes as an HMAC secret.

**Parameters:**
- `token` (String) - The JWT token
- `secret` (String) - Secret key used for signing. Same 32-byte minimum for HMAC algorithms. Asymmetric algorithms (RS256, EdDSA) use PEM keys via the `key` option.
- `options` (Hash, optional) - Verification options
  - `algorithm` (String) - Pin verification to a specific algorithm (`HS256`, `HS384`, `HS512`, `RS256`, `EdDSA`). The token's header `alg` must match exactly or the call rejects with `"token algorithm ... does not match expected"`.
  - `key` (String) - PEM-encoded public key for RS256/EdDSA algorithms. When `key` is provided without an explicit `algorithm`, the allowed set is `RS256` / `EdDSA` only — HMAC tokens are rejected (the algorithm-confusion attack vector).
  - `audience` (String or Array) - Expected `aud`. When set, `aud` becomes a **required** claim, so a token without one cannot slip through a check you believed was enforced.
  - `issuer` (String or Array) - Expected `iss`, likewise required once set.
  - `subject` (String) - Expected `sub`
  - `leeway` (Int) - Clock-skew tolerance in seconds (default 60)

When neither `algorithm` nor `key` is provided, the 2-arg form accepts only HMAC algorithms (`HS256`/`HS384`/`HS512`), matching the back-compat default.

> **Audience is opt-in.** `aud` is only checked when you pass `audience`; a token carrying an audience you never asked about verifies normally. Audience is caller-supplied policy, exactly like `iss`. If you issue tokens for more than one client, pass `audience` — otherwise a token minted for client A is accepted by client B.
>
> Passing **several** expected audiences means the token must carry *all* of them, not any one of them. To accept one of many, verify once per candidate.

**Returns:** Hash - Decoded payload, or `{ "error": true, "message": String }` on failure

**Example:**
```soli
# 2-arg form: HMAC only.
result = jwt_verify(token, getenv("JWT_SECRET"))
if has_key(result, "error")
  println("Invalid token: " + result["message"])
else
  println("User: " + result["sub"])
end

# Asymmetric verification: pin the algorithm explicitly.
result = jwt_verify(token, "", { "algorithm": "RS256", "key": rsa_public_pem })

# Verifying an OIDC id_token: check who issued it and who it was meant for.
claims = jwt_verify(id_token, "", {
  "algorithm": "RS256",
  "key": provider_public_pem,
  "issuer": "https://op.example",
  "audience": getenv("OIDC_CLIENT_ID")
})
```

### jwt_decode_unsafe(token)

Decode a JWT **without** verifying its signature or expiration. The result is wrapped as `{unverified: true, claims: {...}}` so it cannot be confused with a verified `jwt_verify` result. **Never trust these claims for authentication** — use `jwt_verify(token, secret)` for that.

The previous `jwt_decode(token)` returned the same shape as `jwt_verify`, which made `claims["sub"]` a silent auth bypass. It has been removed (SEC-029); calling it now raises a migration error pointing at this function.

**Parameters:**
- `token` (String) - The JWT token

**Returns:** Hash — `{unverified: true, claims: {...}}` on success, `{error: true, message: ...}` on a malformed token.

**Example:**
```soli
let result = jwt_decode_unsafe(token)
println(result["claims"]["sub"])  # Inspection only — DO NOT use for auth
```

---

## VAPID / Web Push Functions

Soli has native Web Push support — see [VAPID / Web Push Functions](/docs/builtins/vapid) for the full
reference.

### vapid_generate_keys()

Generates a fresh P-256 application server key pair. Returns
`{"public_key": String, "private_key": String}` (both base64url, no padding). Run once at setup time and
store the result in `.env`.

### vapid_sign(private_key, audience, subject, expiry_seconds?)

Signs an ES256 VAPID JWT for the `Authorization: vapid t=<jwt>, k=<public_key>` header. `audience` must
be the `scheme://host[:port]` origin of the push endpoint. Defaults to a 12 h expiry; RFC 8292 caps at
24 h.

### vapid_encrypt(payload, subscription, public_key, private_key)

Encrypts a push payload per RFC 8291 (`aes128gcm`). Returns
`{"ciphertext", "salt", "server_public_key"}`. A fresh ephemeral P-256 keypair is generated internally —
the VAPID identity keys are never reused for ECDH. The trailing `public_key` / `private_key` arguments
mirror `vapid_send`'s signature but are not consumed by encryption.

### vapid_send(subscription, payload, private_key, public_key, subject, options?)

End-to-end Web Push delivery: signs the JWT, encrypts the payload, and POSTs it to
`subscription["endpoint"]`. Options: `ttl` (Int, default 60), `urgency` (String), `topic` (String),
`expiry_seconds` (Int). Returns `{"status": Int, "body": String}` — 201 on success, 404/410 means the
subscription is dead and should be deleted from your store.

**Example:**

```soli
let result = vapid_send(
  subscription,
  json_stringify({ "title": "Hi", "body": "From Alice" }),
  getenv("VAPID_PRIVATE_KEY"),
  getenv("VAPID_PUBLIC_KEY"),
  "mailto:ops@example.com"
)
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
Regex.matches("^[a-z]+$", "hello")  # true
Regex.matches("^[0-9]+$", "hello")  # false
```

### Regex.find(pattern, string)

Finds the first match of a pattern in a string.

**Parameters:**
- `pattern` (String) - Regular expression pattern
- `string` (String) - String to search

**Returns:** Hash|null - `{ "match": String, "start": Int, "end": Int }` or null

**Example:**
```soli
result = Regex.find("[0-9]+", "abc123def")
println(result["match"])  # "123"
println(result["start"])  # 3
```

### Regex.find_all(pattern, string)

Finds all matches of a pattern in a string.

**Parameters:**
- `pattern` (String) - Regular expression pattern
- `string` (String) - String to search

**Returns:** Array - Array of match hashes

**Example:**
```soli
matches = Regex.find_all("[0-9]+", "a1b2c3")
# [{"match": "1", ...}, {"match": "2", ...}, {"match": "3", ...}]
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
Regex.replace("[0-9]+", "a1b2c3", "X")  # "aXb2c3"
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
Regex.replace_all("[0-9]+", "a1b2c3", "X")  # "aXbXcX"
```

### Regex.split(pattern, string)

Splits a string by a regex pattern.

**Parameters:**
- `pattern` (String) - Regular expression pattern
- `string` (String) - String to split

**Returns:** Array - Array of substrings

**Example:**
```soli
Regex.split("[,;]", "a,b;c,d")  # ["a", "b", "c", "d"]
```

### Regex.capture(pattern, string)

Finds the first match with named capture groups.

**Parameters:**
- `pattern` (String) - Regular expression with named groups `(?P<name>...)`
- `string` (String) - String to search

**Returns:** Hash|null - Match info plus named captures

**Example:**
```soli
result = Regex.capture(
  "(?P<year>[0-9]{4})-(?P<month>[0-9]{2})",
  "Date: 2024-01-15"
)
println(result["year"])   # "2024"
println(result["month"])  # "01"
```

### Regex.escape(string)

Escapes special regex characters in a string.

**Parameters:**
- `string` (String) - String to escape

**Returns:** String

**Example:**
```soli
Regex.escape("hello.world")  # "hello\\.world"
```

---

## JSON Class

The `JSON` class provides static methods for parsing and serializing JSON data.

### JSON.parse(string)

Parses a JSON string into a Soli value (Hash, Array, String, Int, Float, Bool, or null).

**Parameters:**
- `string` (String) - A valid JSON string

**Returns:** Any - The parsed value

**Example:**
```soli
data = JSON.parse('{"name": "Alice", "age": 30}')
println(data["name"])  # "Alice"

numbers = JSON.parse('[1, 2, 3, 4, 5]')
println(numbers[0])  # 1
```

### JSON.stringify(value)

Serializes a Soli value to a JSON string.

**Parameters:**
- `value` (Any) - A JSON-compatible value (Hash, Array, String, Int, Float, Bool, null)

**Returns:** String - The JSON string representation

**Example:**
```soli
json = JSON.stringify({ "name": "Alice", "scores": [95, 87] })
println(json)  # {"name":"Alice","scores":[95,87]}

arr = JSON.stringify([1, 2, 3])
println(arr)  # [1,2,3]
```

### JSON.parse_jsonp(string)

Unwraps a JSONP string — `callback({...});`, `callback([...])`, optionally with a
leading `/**/` guard — and parses the inner JSON into a Soli value. The padding
between the first `(` and last `)` is stripped, so parentheses inside JSON string
values are preserved.

**Parameters:**
- `string` (String) - A JSONP response body

**Returns:** Any - The parsed value. Raises if the string is not a valid
`callback(...)` wrapper or the inner JSON is malformed.

**Example:**
```soli
data = JSON.parse_jsonp('/**/cb({"name": "Alice", "age": 30});')
println(data["name"])  # "Alice"

# Pair it with a raw fetch when you don't want HTTP.get_jsonp:
body = HTTP.get("https://api.example.com/feed?callback=cb")
feed = JSON.parse_jsonp(body)
```

---

## Markdown Class

The `Markdown` class converts Markdown text to HTML. It supports standard Markdown syntax plus tables, strikethrough, and task lists.

### Markdown.to_html(markdown)

Converts a Markdown string to HTML. Use this for trusted Markdown authored by your application or developers. It preserves raw HTML allowed by Markdown.

**Parameters:**
- `markdown` (String) - Markdown source text

**Returns:** String - The rendered HTML

**Example:**
```soli
html = Markdown.to_html("# Hello World")
println(html)  # <h1>Hello World</h1>
```

### Markdown.to_safe_html(markdown)

Converts a Markdown string to HTML for user-generated content. Raw HTML is escaped, and unsafe link or image URLs such as `javascript:` are neutralized.

**Parameters:**
- `markdown` (String) - Markdown source text

**Returns:** String - The rendered safe HTML

**Example:**
```soli
html = Markdown.to_safe_html(user.bio)
```

**Supported syntax:**

```soli
# Headings
Markdown.to_html("# H1\n## H2\n### H3")

# Bold and italic
Markdown.to_html("**bold** and *italic*")

# Links
Markdown.to_html("[Soli](https://example.com)")

# Lists
Markdown.to_html("- item 1\n- item 2\n- item 3")

# Code blocks
Markdown.to_html("```\nlet x = 1\n```")

# Tables
Markdown.to_html("| Name | Age |\n|------|-----|\n| Alice | 30 |")

# Strikethrough
Markdown.to_html("~~removed~~")

# Blockquotes
Markdown.to_html("> This is a quote")
```

**Use with dynamic content:**

```soli
# From a database field
post = Post.find(1)
html = Markdown.to_safe_html(post.body)

# With string interpolation
title = "My Post"
html = Markdown.to_html("# #{title}\n\nSome content here.")
```

---

## SOAP Class

The `SOAP` class provides methods for making SOAP (Simple Object Access Protocol) calls and working with XML data.

> **Security — XML hardening.** The XML parser used by `SOAP.call` and `SOAP.parse` rejects DOCTYPE declarations outright (XXE / billion-laughs vector), caps element-nesting depth at 64, and caps accumulated text per element at 1 MiB. Legitimate SOAP responses are well below these limits; payloads that hit any cap return a parser error rather than risk runaway memory.

### SOAP.call(url, action, envelope, headers?)

Makes a SOAP request by performing an HTTP POST with the SOAP envelope.

**Parameters:**
- `url` (String) - The SOAP service endpoint URL
- `action` (String) - The SOAP action/method name
- `envelope` (String) - The complete SOAP envelope XML
- `headers` (Hash, optional) - Additional HTTP headers

**Returns:** Hash - Response with:
- `status` (Int) - HTTP status code
- `body` (String) - Raw XML response
- `parsed` (Hash) - Parsed XML as nested Hash structures

**Example:**
```soli
envelope = SOAP.wrap("<GetWeather><City>London</City></GetWeather>")
result = await(SOAP.call("https://weather.example.com/service", "GetWeather", envelope))

if result["status"] == 200
  temp = result["parsed"]["soap:Envelope"]["soap:Body"]["GetWeatherResponse"]["Temperature"]
  println("Temperature: " + temp)
end
```

### SOAP.wrap(body, namespace?, options?)

Wraps an XML body in a complete SOAP envelope with the standard SOAP 1.1 namespace.

**Parameters:**
- `body` (String) - The XML body content
- `namespace` (String, optional) - SOAP envelope namespace (default: SOAP 1.1)
- `options` (Hash, optional) - Wrapping options:
  - `escape` (Bool) - When `true`, XML-escapes the body before wrapping. Use this when `body` is **untrusted text** (user input, third-party data) that must appear as PCDATA, not as XML. Defaults to `false` so already-built XML fragments pass through verbatim.

**Returns:** String - Complete SOAP envelope XML

**Example:**
```soli
# Trusted XML fragment — pass through verbatim
body = "<GetWeather xmlns=\"http://example.com/weather\"><City>London</City></GetWeather>"
envelope = SOAP.wrap(body)

# Untrusted text — escape so "<" / "&" cannot break out
envelope = SOAP.wrap(user_supplied, null, { "escape": true })
```

> **Pick `escape: true` whenever `body` contains user input.** Without it, an attacker who controls `body` can inject arbitrary XML into the envelope.

### SOAP.parse(xml)

Parses an XML string into a nested Hash structure for easy access.

**Parameters:**
- `xml` (String) - XML string to parse

**Returns:** Hash - Nested Hash with element names as keys and text/attributes as values

**Example:**
```soli
xml = "<?xml version=\"1.0\"?><root><item>value</item></root>"
parsed = SOAP.parse(xml)
# Returns: { "root" => { "item" => { "_text" => "value" } } }
```

### SOAP.xml_escape(text)

Escapes special XML characters for safe inclusion in XML documents.

**Parameters:**
- `text` (String) - The text to escape

**Returns:** String - XML-escaped text (&lt;, &gt;, &amp;, &quot;, &apos;)

**Example:**
```soli
escaped = SOAP.xml_escape("<script>alert('xss')</script>")
# Returns: "&lt;script&gt;alert(&apos;xss&apos;)&lt;/script&gt;"
```

### Complete SOAP Example

```soli
# Build the SOAP request body
body = "<GetWeather xmlns=\"http://example.com/weather\"><City>London</City></GetWeather>"
envelope = SOAP.wrap(body)

# Make the SOAP call
result = await(SOAP.call(
  "https://weather.example.com/service",
  "http://example.com/weather/GetWeather",
  envelope,
  { "Authorization": "Bearer token123" }
))

# Handle the response
if result["status"] == 200
  response = result["parsed"]["soap:Envelope"]["soap:Body"]["GetWeatherResponse"]
  temp = response["Temperature"]
  condition = response["Condition"]
  
  println("Temperature: " + temp)
  println("Condition: " + condition)
else
  println("Error: " + result["body"])
end
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
path = getenv("PATH")
debug = getenv("DEBUG")
```

### hasenv(name)

Checks if an environment variable exists.

**Parameters:**
- `name` (String) - Variable name

**Returns:** Bool

**Example:**
```soli
if hasenv("DATABASE_URL")
  url = getenv("DATABASE_URL")
end
```

> `.env` and `.env.{APP_ENV}` in the application directory are auto-loaded once at server startup — no Soli-level call is needed.

---

## DateTime Class

The `DateTime` class provides a convenient way to work with dates and times. Create instances using static methods, then use instance methods to extract components or perform arithmetic.

### Static Methods

#### DateTime.now()

Gets the current local date and time.

**Returns:** DateTime - A DateTime instance

**Example:**
```soli
now = DateTime.now()
println(now.to_iso())  # "2024-01-15T10:30:00"
```

#### DateTime.utc()

Gets the current UTC date and time.

**Returns:** DateTime - A DateTime instance

**Example:**
```soli
utc = DateTime.utc()
println(utc.to_iso())  # "2024-01-15T15:30:00Z"
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
dt = DateTime.parse("2024-01-15T10:30:00Z")
date_only = DateTime.parse("2024-01-15")
```

#### DateTime.epoch()

Creates a DateTime at Unix epoch (1970-01-01 00:00:00 UTC).

**Returns:** DateTime - A DateTime instance

**Example:**
```soli
epoch = DateTime.epoch()
println(epoch.year())  # 1970
```

#### DateTime.from_unix(timestamp)

Creates a DateTime from a Unix timestamp (seconds since epoch).

**Parameters:**
- `timestamp` (Int) - Unix timestamp in seconds

**Returns:** DateTime - A DateTime instance

**Example:**
```soli
dt = DateTime.from_unix(1704067200)
println(dt.to_iso())  # "2024-01-01T00:00:00Z"
```

#### DateTime.microtime()

Gets the current Unix timestamp in microseconds as a Float.

**Returns:** Float - Microseconds since epoch

**Example:**
```soli
mt = DateTime.microtime()
println(mt)  # 1712832000000000.0
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
dt = DateTime.parse("2024-01-15")
println(dt.weekday())  # "monday"
```

### Instance Methods - Formatting

#### .to_unix()

Gets the Unix timestamp (seconds since epoch).

**Returns:** Int

**Example:**
```soli
dt = DateTime.now()
println(dt.to_unix())  # 1705315800
```

#### .to_iso()

Gets the date/time as an ISO 8601 string.

**Returns:** String

**Example:**
```soli
dt = DateTime.now()
println(dt.to_iso())  # "2024-01-15T10:30:00"
```

#### .format(pattern, locale?)

Formats the date/time using strftime pattern specifiers. Pass an optional locale to get localized month and day names.

**Parameters:**
- `pattern` (String) - strftime format pattern
- `locale` (String?) - Optional locale code: `"en"` (default), `"fr"`, `"es"`, `"de"`, `"it"`, `"pt"`

Common format specifiers:
- `%Y` - 4-digit year (2024)
- `%m` - 2-digit month (01-12)
- `%d` - 2-digit day (01-31)
- `%H` - 24-hour hour (00-23)
- `%M` - Minute (00-59)
- `%S` - Second (00-59)
- `%B` - Full month name (localized)
- `%b` - Abbreviated month name (localized)
- `%A` - Full weekday name (localized)
- `%a` - Abbreviated weekday name (localized)

**Returns:** String

**Example:**
```soli
dt = DateTime.parse("2024-01-15T10:30:00")
dt.format("%Y-%m-%d %H:%M:%S")  # "2024-01-15 10:30:00"
dt.format("%B %d, %Y")           # "January 15, 2024"
dt.format("%A")                  # "Monday"

# With locale for I18n
dt2 = DateTime.parse("2024-03-06T14:30:00Z")
dt2.format("%A %d %B %Y", "fr")  # "mercredi 06 mars 2024"
dt2.format("%A %d %B %Y", "es")  # "miércoles 06 marzo 2024"
dt2.format("%d %b %Y", "fr")     # "06 mars 2024"
```

### Instance Methods - Arithmetic

#### .add_days(n)

Adds days to the date. Use negative values to subtract.

**Parameters:**
- `n` (Int) - Number of days to add

**Returns:** DateTime - A new DateTime instance

**Example:**
```soli
today = DateTime.now()
tomorrow = today.add_days(1)
yesterday = today.add_days(-1)
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

### Instance Methods - Boundaries

#### .beginning_of_minute()

Truncates seconds and sub-seconds to zero, keeping the same minute.

**Returns:** DateTime - A new DateTime instance

**Example:**
```soli
dt = DateTime.parse("2024-06-15T10:30:45.123Z")
dt.beginning_of_minute().to_iso()  # "2024-06-15T10:30:00.000Z" (approximately)
```

#### .end_of_minute()

Sets seconds to 59 and milliseconds to 999, keeping the same minute.

**Returns:** DateTime - A new DateTime instance

#### .beginning_of_hour()

Sets minutes, seconds, and sub-seconds to zero, keeping the same hour.

**Returns:** DateTime - A new DateTime instance

#### .end_of_hour()

Sets minutes to 59, seconds to 59, and milliseconds to 999, keeping the same hour.

**Returns:** DateTime - A new DateTime instance

#### .beginning_of_day()

Sets hours, minutes, seconds, and sub-seconds to zero, keeping the same day.

**Returns:** DateTime - A new DateTime instance

#### .end_of_day()

Sets the time to 23:59:59.999, keeping the same day.

**Returns:** DateTime - A new DateTime instance

#### .beginning_of_month()

Sets the day to 1 and time to 00:00:00.000, keeping the same month and year.

**Returns:** DateTime - A new DateTime instance

**Example:**
```soli
dt = DateTime.parse("2024-06-15T10:30:45Z")
dt.beginning_of_month().day()    # 1
dt.beginning_of_month().hour()   # 0
```

#### .end_of_month()

Sets the date to the last day of the month and time to 23:59:59.999, keeping the same month and year.

**Returns:** DateTime - A new DateTime instance

**Example:**
```soli
dt = DateTime.parse("2024-06-15T10:30:45Z")
dt.end_of_month().day()    # 30
dt.end_of_month().hour()   # 23
```

#### .beginning_of_year()

Sets the date to January 1st and time to 00:00:00.000, keeping the same year.

**Returns:** DateTime - A new DateTime instance

#### .end_of_year()

Sets the date to December 31st and time to 23:59:59.999, keeping the same year.

**Returns:** DateTime - A new DateTime instance

### Comparison

Two `DateTime` instances can be compared with the standard operators (`<`,
`<=`, `>`, `>=`, `==`, `!=`). Comparison is by absolute instant — equality
holds when both refer to the same moment, regardless of which `DateTime`
object you're holding.

```soli
let a = DateTime.from_unix(1700000000)
let b = DateTime.from_unix(1700000000)
let later = a.add_hours(1)

a == b       # true — same instant, different instances
a < later    # true
later >= a   # true
```

### Helper Functions

These free-standing functions are also available alongside the `DateTime` class.

#### time_ago(timestamp)

Returns a human-readable relative time string. Uses the current I18n locale for translations (supports en, fr, de, es, it, pt, ja, zh).

**Parameters:**
- `timestamp` (Int|String) — Unix timestamp or date string

**Returns:** String

**Examples:**
```soli
time_ago(datetime_now() - 7200)  # "2 hours ago" (en)
                                 # "il y a 2 heures" (fr)
                                 # "vor 2 Stunden" (de)
time_ago(ts)                     # "5 seconds ago"
time_ago(ts)                     # "1 minute ago"
time_ago(ts)                     # "2 weeks ago"
time_ago(future_ts)              # "in the future"
```

### Complete Example

```soli
# Get current date/time
now = DateTime.now()
println("Current time: " + now.to_iso())

# Extract components
println("Year: " + now.year())
println("Month: " + now.month())
println("Day: " + now.day())
println("Weekday: " + now.weekday())

# Format output
println(now.format("%B %d, %Y at %H:%M"))

# Format with locale for I18n
println(now.format("%A %d %B %Y", "fr"))  # "lundi 15 janvier 2024"
println(now.format("%A %d %B %Y", "es"))  # "lunes 15 enero 2024"

# Date arithmetic
next_week = now.add_weeks(1)
last_month = now.add_months(-1)

# Parse a date string
birthday = DateTime.parse("1990-06-15")
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
timeout = Duration.seconds(30)
one_minute = Duration.seconds(60)
```

#### Duration.minutes(n)

Creates a duration from a number of minutes.

**Parameters:**
- `n` (Int) - Number of minutes

**Returns:** Duration

**Example:**
```soli
break_time = Duration.minutes(15)
```

#### Duration.hours(n)

Creates a duration from a number of hours.

**Parameters:**
- `n` (Int) - Number of hours

**Returns:** Duration

**Example:**
```soli
work_day = Duration.hours(8)
session_timeout = Duration.hours(1)
```

#### Duration.days(n)

Creates a duration from a number of days.

**Parameters:**
- `n` (Int) - Number of days

**Returns:** Duration

**Example:**
```soli
week = Duration.days(7)
trial_period = Duration.days(30)
```

#### Duration.weeks(n)

Creates a duration from a number of weeks.

**Parameters:**
- `n` (Int) - Number of weeks

**Returns:** Duration

**Example:**
```soli
fortnight = Duration.weeks(2)
```

#### Duration.of_seconds(n)

Creates a duration from a number of seconds. Alias for `Duration.seconds()`.

**Parameters:**
- `n` (Int | Float) - Number of seconds

**Returns:** Duration

#### Duration.of_minutes(n)

Creates a duration from a number of minutes.

**Parameters:**
- `n` (Int | Float) - Number of minutes

**Returns:** Duration

#### Duration.of_hours(n)

Creates a duration from a number of hours.

**Parameters:**
- `n` (Int | Float) - Number of hours

**Returns:** Duration

#### Duration.of_days(n)

Creates a duration from a number of days.

**Parameters:**
- `n` (Int | Float) - Number of days

**Returns:** Duration

#### Duration.of_weeks(n)

Creates a duration from a number of weeks.

**Parameters:**
- `n` (Int | Float) - Number of weeks

**Returns:** Duration

#### Duration.between(dt1, dt2)

Creates a Duration representing the difference between two DateTime instances.

**Parameters:**
- `dt1` (DateTime) - Start datetime
- `dt2` (DateTime) - End datetime

**Returns:** Duration - The duration from dt1 to dt2 (dt2 - dt1)

**Example:**
```soli
dt1 = DateTime.parse("2024-01-15T10:00:00Z")
dt2 = DateTime.parse("2024-01-15T11:30:00Z")
dur = Duration.between(dt1, dt2)
println(dur.total_minutes())  # 90
```

### Instance Methods

#### .total_seconds()

Gets the total duration in seconds.

**Returns:** Float

**Example:**
```soli
duration = Duration.hours(2)
println(duration.total_seconds())  # 7200
```

#### .total_minutes()

Gets the total duration in minutes.

**Returns:** Float

**Example:**
```soli
duration = Duration.hours(2)
println(duration.total_minutes())  # 120
```

#### .total_hours()

Gets the total duration in hours.

**Returns:** Float

**Example:**
```soli
duration = Duration.days(1)
println(duration.total_hours())  # 24
```

#### .total_days()

Gets the total duration in days.

**Returns:** Float

**Example:**
```soli
duration = Duration.weeks(1)
println(duration.total_days())  # 7
```

#### .to_string

Gets the duration as a formatted string.

**Returns:** String

**Example:**
```soli
duration = Duration.of_seconds(3661)
println(duration.to_string)  # "3661s"
```

#### .humanize(locale?)

Gets the duration as a human-readable compound string (e.g., "1 hour 1 minute"). Selects the most appropriate unit(s) based on the duration length — for sub-hour durations it combines minutes + seconds; for sub-day it combines hours + minutes; for longer durations it combines days + hours. The optional locale parameter overrides the current I18n locale for translation.

**Parameters:**
- `locale` (String, optional) - Locale code for translation (defaults to current I18n locale)

**Returns:** String

**Example:**
```soli
Duration.seconds(3661).humanize()    # "1 hour 1 minute"
Duration.seconds(1000).humanize()   # "16 minutes 40 seconds"
Duration.seconds(7200).humanize()   # "2 hours"
Duration.seconds(90).humanize()     # "1 minute 30 seconds"
Duration.minutes(5).humanize()       # "5 minutes"
Duration.humanize("fr")             # respects fr locale if translations exist
```

### Complete Example

```soli
# Create durations
timeout = Duration.seconds(30)
break_time = Duration.minutes(15)
work_day = Duration.hours(8)
trial = Duration.days(7)

# Convert to different units
println("Timeout: " + timeout.total_seconds() + " seconds")
println("Break: " + break_time.total_minutes() + " minutes")
println("Work day: " + work_day.total_hours() + " hours")
println("Trial: " + trial.total_days() + " days")

# Practical example: session expiry
session_duration = Duration.hours(1)
expiry_seconds = session_duration.to_seconds()
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
schema = {
  "name": V.string().required().min(2),
  "email": V.string().required().email(),
  "age": V.int().optional().min(0).max(150)
}

result = validate({
  "name": "Alice",
  "email": "alice@example.com"
}, schema)

if result["valid"]
  println("Data is valid!")
  println(result["data"])
else
  for error in result["errors"]
    println(error["field"] + ": " + error["message"])
  end
end
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
user_id = session_get("user_id")
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
# Logout user
session_destroy()
```

### session_regenerate()

Regenerates the session ID (for security after login).

**Returns:** String - New session ID

**Example:**
```soli
# After successful login
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

## Cookies

Cookies are automatically parsed from the `Cookie` header and exposed as a global `cookies` hash, defaulting to `{}` when no cookies are present.

### cookies

Global hash of parsed cookies. Available in controllers, middleware, and views.

**Type:** Hash

**Example:**
```soli
theme = cookies["theme"] or "light"
session_id = cookies.session_id
```

### set_cookie(name, value, options?)

Sets a response cookie sent to the client as a `Set-Cookie` header. Without
options only `Path=/` is set. The options hash accepts `max_age` (seconds;
`0` expires the cookie immediately), `expires`, `http_only`, `secure`,
`same_site` (`"Lax"`/`"Strict"`/`"None"`), `path`, and `domain`, plus the
`signed`/`encrypted` sealing options (below). Unknown keys raise so a typo
can't silently weaken a cookie.

**Parameters:**
- `name` (String) - Cookie name
- `value` (String) - Cookie value (any JSON-serializable value with `signed`/`encrypted`)
- `options` (Hash, optional) - Cookie attributes and sealing options

**Returns:** null

**Example:**
```soli
set_cookie("theme", "dark")

# Persistent remember-me cookie (what `soli generate auth` scaffolds):
set_cookie("remember_token", user["_key"] + ":" + token, {
  "max_age": 30 * 86400,
  "http_only": true,
  "same_site": "Lax"
})

# Expire it on logout:
set_cookie("remember_token", "", { "max_age": 0, "http_only": true })
```

### Signed and encrypted cookies

`{"signed": true}` seals the value with HMAC-SHA256 (readable on the client
as base64url JSON, but tamper-proof); `{"encrypted": true}` seals it with
AES-256-GCM (opaque). Sealed values accept any JSON-serializable value, not
just strings. Both keys are HKDF-derived from `SOLI_SESSION_SECRET` (32+
chars; sealing raises without it), the cookie **name** is bound into the
seal so values can't be swapped between cookies, and a `max_age` is embedded
as an expiry inside the payload. The two options are mutually exclusive.

```soli
set_cookie("prefs", {"theme": "dark"}, {"encrypted": true, "max_age": 86400})
set_cookie("uid", 42, {"signed": true})
```

### read_cookie(name, options?)

Reads a cookie back, verifying/decrypting sealed values. The options state
the trust requirement: `{"signed": true}` or `{"encrypted": true}` returns
the decoded value only when it was validly sealed by your server under that
name and mode — tampered, expired, forged (attacker-set bare values) or
mode-mismatched cookies all return `nil`, indistinguishable from an absent
cookie. Without options it returns the raw string value (like
`cookies[name]`). Sees cookies written by `set_cookie` earlier in the same
request.

**Parameters:**
- `name` (String) - Cookie name
- `options` (Hash, optional) - `{"signed": true}` or `{"encrypted": true}`

**Returns:** the decoded value, or `nil`

**Example:**
```soli
prefs = read_cookie("prefs", {"encrypted": true})   # {"theme": "dark"}
uid = read_cookie("uid", {"signed": true})          # 42 — verified, not forgeable
theme = read_cookie("theme")                        # raw string or nil
```

### csrf_token()

Returns the per-session CSRF token, creating it (and the session) on first
use. Views usually don't call this directly — `form_with(...).open()`,
`button_to`, and `csrf_field()` embed it as a hidden `_csrf_token` input, and
`csrf_meta_tag()` exposes it to JS clients that send the `X-CSRF-Token`
header. The server verifies a supplied token against the session with a
constant-time compare and rejects mismatches with 403. See
[Forms & CSRF](/docs/core-concepts/forms).

**Returns:** String — a 32-hex-char token

**Example:**
```soli
# In a layout, for fetch/htmx clients:
# <body hx-headers='{"X-CSRF-Token": "<%= csrf_token() %>"}'>
token = csrf_token()
```

---

## Background Jobs and Cron

Soli ships with a SolidB-backed queue and cron system. Define a handler in `app/jobs/{name}_job.sl` (`class {Name}Job` with a `static def perform(args: Hash)`), then enqueue or schedule it. Full guide: [jobs.md](jobs.md).

### Job class

Every user-defined `XJob` class also gets these static helpers injected automatically.

#### Job.enqueue(handler, args, queue_or_opts?)

Enqueues a job by handler name. Returns the SolidB job id. The trailing argument is either a queue-name string or an options hash `{ queue, priority, max_retries }` — `priority` is an Int and higher runs first.

```soli
job_id = Job.enqueue("WelcomeEmailJob", { "user_id": 42 })
Job.enqueue("WelcomeEmailJob", { "user_id": 42 }, { "queue": "mailers", "priority": 10 })
```

#### Job.enqueue_in(handler, duration, args, queue_or_opts?)

Enqueues with a relative delay. `duration` accepts `"5 minutes"`, `"1 hour"`, `"2 days"`, etc., or a number of seconds.

```soli
Job.enqueue_in("WelcomeEmailJob", "30 minutes", { "user_id": 42 })
```

#### Job.enqueue_at(handler, datetime, args, queue_or_opts?)

Enqueues to run at a specific ISO-8601 timestamp.

```soli
Job.enqueue_at("WelcomeEmailJob", "2026-05-01T08:00:00Z", { "user_id": 42 })
```

#### Job.cancel(job_id)

Cancels an enqueued (not yet started) job. Returns Bool.

#### Job.list(queue?)

Returns the list of jobs in a queue. Defaults to the configured default queue.

#### Job.queues()

Returns the list of queue names known to SolidB.

### Per-class facade methods

Each user-defined `XJob` class gets:

| Method | Behavior |
|--------|----------|
| `XJob.perform_later(args, queue_or_opts?)` | Enqueues into SolidB. Returns the job id. |
| `XJob.perform_in(duration, args, queue_or_opts?)` | Enqueues with a relative delay. |
| `XJob.perform_at(datetime, args, queue_or_opts?)` | Enqueues to run at an ISO-8601 timestamp. |
| `XJob.schedule_cron(name, expr, args?)` | Idempotently registers a cron entry that triggers this class. |

The trailing `queue_or_opts` argument is either a queue-name string or an options hash `{ queue, priority, max_retries }` (higher `priority` runs first).

```soli
WelcomeEmailJob.perform_later({ "user_id": 42 })
WelcomeEmailJob.perform_in("5 minutes", { "user_id": 42 })
WelcomeEmailJob.perform_later({ "user_id": 42 }, "mailers")
WelcomeEmailJob.perform_later({ "user_id": 42 }, { "queue": "mailers", "priority": 10 })
```

### Cron class

#### Cron.schedule(name, expr, handler, args?)

Idempotent upsert by name. Calling twice with the same name updates the existing entry rather than creating a duplicate.

```soli
Cron.schedule("nightly_report", Cron.daily_at("03:00"), "ReportJob", {})
```

#### Cron.list()

Returns all cron entries.

#### Cron.update(id, fields)

Updates an existing cron entry. Pass a hash of fields.

#### Cron.delete(id)

Deletes a cron entry by id. Returns Bool.

### Cron expression helpers

Pure string builders. No SolidB writes.

| Helper | Cron string |
|--------|-------------|
| `Cron.every("5 minutes")` | `*/5 * * * *` |
| `Cron.every("1 hour")` | `0 * * * *` |
| `Cron.every("2 hours")` | `0 */2 * * *` |
| `Cron.every("1 day")` | `0 0 */1 * *` |
| `Cron.hourly()` | `0 * * * *` |
| `Cron.daily_at("03:00")` | `0 3 * * *` |
| `Cron.weekly_at("monday", "09:00")` | `0 9 * * 1` |

### Declarative `static cron`

A class can declare a `static cron` field; on boot, worker 0 upserts a cron entry named after the class (snake_case, e.g. `nightly_report_job`).

```soli
class NightlyReportJob {
  static cron = Cron.daily_at("03:00")

  static def perform(args: Hash) {
    Report.generate()
  }
}
```

Removing the field does not auto-delete the SolidB entry — call `Cron.delete(id)` explicitly.

### Configuration env vars

| Variable | Purpose | Default |
|----------|---------|---------|
| `SOLI_JOBS_DATABASE` | SolidB database for queues and cron | `SOLIDB_DATABASE` then `default` |
| `SOLI_JOBS_DEFAULT_QUEUE` | Queue name when none is supplied | `default` |
| `SOLI_JOBS_CALLBACK_URL` | URL SolidB POSTs to when a job fires | `http://127.0.0.1:3000/_jobs/run` |
| `SOLI_JOBS_SECRET` | **Required.** HMAC-SHA256 key used to sign and verify job callbacks (`X-Job-Signature` header). The `/_jobs/run/:name` route is not registered if unset — see [Jobs / Signed Callbacks](jobs.md#security-signed-callbacks) | unset |

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
test("addition works correctly", fn()
  assert_eq(1 + 1, 2)
end)
```

#### describe(description, callback)

Groups related tests.

**Parameters:**
- `description` (String) - Group description
- `callback` (Function) - Function containing tests

**Example:**
```soli
describe("Calculator", fn()
  test("adds numbers", fn()
    assert_eq(add(1, 2), 3)
  end)

  test("subtracts numbers", fn()
    assert_eq(subtract(5, 3), 2)
  end)
end)
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

## Test Helpers

### with_transaction(block)

Runs a block inside a SolidB transaction and **always rolls back** when the block finishes (test-only). Unlike `Model.transaction { }`, it never commits.

```soli
with_transaction(fn() {
  Factory.insert("user")
  assert_eq(User.count(), 1)
})
assert_eq(User.count(), 0)
```

### freeze_time(timestamp) / travel_to(timestamp) / unfreeze_time()

Pins `datetime_now()` to a fixed Unix timestamp (int or parseable date string). Cleared by `unfreeze_time()` and automatically before each test example.

```soli
freeze_time(1_700_000_000)
travel_to("2024-06-15")
unfreeze_time()
```

---

## Factory Functions

Factories help create test data. `Factory.create` returns hashes; use `Factory.insert` to persist through a bound model.

### Factory.define(name, data_or_block)

Defines a factory template as a static hash or a callable block.

**Example:**
```soli
Factory.define("user", {
  "email": "user#{n}@test.com",
  "name": "Test User"
})

Factory.define("post", fn() {
  return {"title": "Post #{Factory.sequence("post")}"}
})
```

String values may include `#{n}` — replaced with a per-factory auto-incrementing counter on each `create`.

### Factory.create(name)

Creates an instance from a factory.

**Parameters:**
- `name` (String) - Factory name

**Returns:** Hash - Created data

**Example:**
```soli
user = Factory.create("user")
```

### Factory.create_with(name, overrides)

Creates an instance with overridden attributes.

**Parameters:**
- `name` (String) - Factory name
- `overrides` (Hash) - Attributes to override

**Returns:** Hash

**Example:**
```soli
admin = Factory.create_with("user", { "role": "admin" })
```

### Factory.create_list(name, count)

Creates multiple instances.

**Parameters:**
- `name` (String) - Factory name
- `count` (Int) - Number to create

**Returns:** Array

**Example:**
```soli
users = Factory.create_list("user", 5)
```

### Factory.sequence(name)

Gets the next value in a sequence.

**Parameters:**
- `name` (String) - Sequence name

**Returns:** Int - Next sequence value (starts at 0)

**Example:**
```soli
Factory.sequence("user_id")  # 0
Factory.sequence("user_id")  # 1
Factory.sequence("user_id")  # 2
```

### Factory.bind(name, model_class)

Associates a factory name with a model class for `Factory.insert`.

### Factory.insert(name, overrides?)

Builds factory attributes (running callable templates and `#{n}` interpolation) then calls `Model.create`. Returns the persisted record.

```soli
Factory.bind("user", User)
user = Factory.insert("user", {"email": "custom@example.com"})
```

### Factory.clear

Clears all factory definitions, model bindings, and sequences.

---

## I18n Functions

The `I18n` class provides internationalization support.

### I18n.locale()

Gets the current locale.

**Returns:** String

**Example:**
```soli
println(I18n.locale())  # "en"
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

### Locale files

Soli auto-loads every `*.yml` (and `*.yaml`) file under `config/locales/` at server boot. The top-level YAML key is the locale name; nested keys form the dotted lookup path used by `I18n.translate`. A single file may declare multiple locales, and several files may extend the same locale (e.g. `en.yml`, `accounts.en.yml`, …).

```yaml
# config/locales/en.yml
en:
  app:
    welcome: Welcome
    greeting: "Hello, {name}!"
  errors:
    not_found: Not found
```

```yaml
# config/locales/fr.yml
fr:
  app:
    welcome: Bienvenue
    greeting: "Bonjour, {name} !"
  errors:
    not_found: Introuvable
```

Resolution falls back to `en` when the active locale has no entry, and finally returns the literal key (e.g. `"app.welcome"`) so typos surface during development.

### I18n.translate(key, locale_or_values?, values?)

Translates a key against the auto-loaded locale tree.

**Parameters:**
- `key` (String) — dotted lookup path (e.g. `"app.welcome"`).
- 2nd arg — optional locale override (String) **or** interpolation values (Hash).
- 3rd arg — optional interpolation values (Hash) when an explicit locale was passed.

Placeholders inside translated strings use `{name}` syntax. Unknown placeholders are left intact so missing data is visible while iterating.

**Returns:** String — the translated text, or the key itself if no translation resolves.

**Example:**
```soli
I18n.translate("app.welcome")                          # "Welcome"
I18n.translate("app.welcome", "fr")                    # "Bienvenue"
I18n.translate("app.greeting", { name: "Alice" })      # "Hello, Alice!"
I18n.translate("app.greeting", "fr", { name: "Alice" }) # "Bonjour, Alice !"
```

### I18n.plural(key, count, locale_or_values?, values?)

Picks the matching plural form for a count. Resolves `<key>_zero` (count == 0), `<key>_one` (count == 1), or `<key>_other`. The same locale-or-values disambiguation as `translate` applies. `count` is auto-injected into the interpolation values, so messages can reference `{count}` directly.

```yaml
# config/locales/en.yml
en:
  items_zero: No items
  items_one: One item
  items_other: "{count} items"
```

```soli
I18n.plural("items", 0)   # "No items"
I18n.plural("items", 1)   # "One item"
I18n.plural("items", 5)   # "5 items"
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
I18n.format_number(1234.56)  # "1234.56"

I18n.set_locale("fr")
I18n.format_number(1234.56)  # "1234,56"
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
I18n.format_currency(1234.56, "USD", "en")  # "$1,234.56"
I18n.format_currency(1234.56, "EUR", "fr")  # "1.234,56"
```

### I18n.format_date(timestamp, locale?)

Formats a date according to locale conventions.

**Parameters:**
- `timestamp` (Int) - Unix timestamp
- `locale` (String, optional) - Override locale

**Returns:** String

**Example:**
```soli
I18n.format_date(ts, "en")  # "01/15/2024"
I18n.format_date(ts, "fr")  # "15/01/2024"
I18n.format_date(ts, "de")  # "15.01.2024"
```

### I18n.cache_table(locale, table)

Stashes a translation `table` (a hash) in a per-worker-thread cache, keyed by `locale`, and returns it. Built for app-level i18n where each locale's table is produced by an expensive call (e.g. a view helper that returns a large hash literal). View helpers run in an isolated per-thread environment with nowhere to memoize, so this gives them a place to build the table once per thread instead of on every lookup.

The cache is thread-local and cleared automatically when view helpers hot-reload (`--dev`), so editing a translation file takes effect on the next render.

**Parameters:**
- `locale` (String) - Cache key (a locale code).
- `table` (Hash) - The translation table to store.

**Returns:** Hash - the same `table`, so you can `return I18n.cache_table(locale, build())` in one line.

**Example:**
```soli
def locale_table(locale)
    cached = I18n.cached_table(locale)
    return cached unless cached.nil?
    return I18n.cache_table(locale, build_table(locale))  # built once per thread
end
```

### I18n.cached_table(locale)

Returns the table previously stored for `locale` via `I18n.cache_table`, or `null` if nothing has been cached yet on the current thread. The returned hash shares the cache's storage — treat it as read-only.

**Parameters:**
- `locale` (String) - Cache key (a locale code).

**Returns:** Hash, or `null` on a miss.

**Example:**
```soli
I18n.cache_table("fr", { "greeting": "Bonjour" })
I18n.cached_table("fr")   # { "greeting": "Bonjour" }
I18n.cached_table("ja")   # null  (nothing cached for "ja" yet)
```

---

## Control Flow

### break

Exits the innermost enclosing loop immediately. Works in both `while` and `for`
loops, and supports postfix conditions (`break if cond` / `break unless cond`).

`break` propagates out of nested blocks, `if` branches and `try`/`catch` — a
`finally` block still runs before the loop exits. A `break` inside a lambda or
function body does **not** break an outer loop; it is absorbed at the function
boundary.

**Example:**
```soli
for i in range(0, 10)
  break if i == 5
  println(i)
end
# prints: 0, 1, 2, 3, 4

# Also valid inside `while`, and out of nested blocks
idx = 0
while true
  if items[idx].nil?
    break
  end
  idx = idx + 1
end
```

> **Note:** handlers containing `break` are not compiled by the bytecode VM —
> they fall back to the tree-walking interpreter automatically (the same
> precedent as safe navigation `&.`). Behavior is identical; only the JIT path
> is skipped.

### next

Skips to the next iteration of a loop.

**Example:**
```soli
for i in range(0, 5)
  if i == 2
    next
  end
  println(i)
end
# prints: 0, 1, 3, 4
```

### await

Awaits an asynchronous operation (used internally for async HTTP).

---

## Cache Functions

Persistent caching backed by SoliKV with automatic TTL expiration. Data persists across restarts and is shared between server instances. All cache keys are prefixed with `soli:cache:` to isolate them from other KV data.

**Configuration:** Set `SOLIKV_RESP_HOST` (default: `localhost`), `SOLIKV_RESP_PORT` (default: `6380`), and optionally `SOLIKV_TOKEN` in your `.env` file, or use `Cache.configure(host, token?)`.

### Cache.set(key, value, ttl_seconds?)

Stores a value in the cache.

**Parameters:**
- `key` (String) - Cache key
- `value` (Any) - Value to cache (JSON serialized)
- `ttl_seconds` (Int, optional) - Time to live in seconds (default: 3600)

**Returns:** null

### Cache.get(key)

Retrieves a value from the cache.

**Parameters:**
- `key` (String) - Cache key

**Returns:** Any|null - Cached value or null if not found/expired

### Cache.delete(key)

Removes a value from the cache.

**Returns:** Bool - true if key was removed

### Cache.has(key)

Checks if a key exists in the cache.

**Returns:** Bool

### Cache.clear

Removes all cache entries (only keys with the `soli:cache:` prefix).

**Returns:** null

### Cache.clear_expired()

No-op. SoliKV handles TTL expiration automatically.

**Returns:** null

### Cache.keys

Returns all cache keys (prefix stripped).

**Returns:** Array

### Cache.size

Returns the number of cache entries.

**Returns:** Int

### Cache.ttl(key)

Gets the remaining TTL for a key in seconds.

**Returns:** Int|null

### Cache.touch(key, ttl)

Sets or updates the TTL for an existing key.

**Returns:** Bool

### Cache.configure(host, token?)

Programmatically configure the SoliKV connection.

**Parameters:**
- `host` (String) - SoliKV URL
- `token` (String, optional) - Bearer token

**Returns:** null

### Cache.fetch(key, ttl?) do...end

Cache-aside pattern: returns cached value on hit, or executes the block on miss, caches and returns the result.

**Parameters:**
- `key` (String) - Cache key
- `ttl` (Int, optional) - Time to live in seconds (default: 3600)
- `do...end` - Block to execute on cache miss

**Returns:** Any - Cached value or block result

```soli
# Basic usage
user = Cache.fetch("user:123") do
    User.find(123)
end

# With TTL (5 minutes)
user = Cache.fetch("user:123", 300) do
    User.find(123)
end

# Without block — acts like Cache.get()
user = Cache.fetch("user:123")
```

---

## KV Class

Full-featured key-value store backed by SoliKV. Supports strings, counters, lists, sets, hashes, sorted sets, bitmaps, and HyperLogLog with Redis-compatible commands. Unlike Cache, KV operates on raw keys without any prefix.

**Configuration:** Same as Cache — `SOLIKV_RESP_HOST`, `SOLIKV_RESP_PORT`, `SOLIKV_TOKEN`, or `KV.configure(host, token?)`.

### Basic Operations

- **KV.set(key, value, ttl?)** — Store a value. Optional TTL in seconds. Returns null.
- **KV.get(key)** — Retrieve a value. Returns null if missing.
- **KV.delete(key)** — Delete a key. Returns Bool.
- **KV.exists(key)** — Check if key exists. Returns Bool.
- **KV.keys(pattern?)** — List keys matching glob pattern (default `"*"`). Returns Array. **Denied by default** — set `SOLI_KV_ALLOW_ADMIN=1` to enable (see [Admin denylist](#admin-denylist)).
- **KV.type(key)** — Get the type of a key. Returns String.
- **KV.rename(key, newkey)** — Rename a key.

### Strings

- **KV.setnx(key, value)** — Set only if the key does not exist. Returns Bool (true if set).
- **KV.getset(key, value)** — Set a new value and return the previous one (or nil).
- **KV.getdel(key)** — Get a value and delete the key in one step. Returns the value or nil.
- **KV.append(key, value)** — Append to a string value. Returns the new length.
- **KV.strlen(key)** — Length of the string value. Returns Int.
- **KV.mget(...keys)** — Get many values at once. Returns an Array (nil for missing keys).
- **KV.mset(key, value, ...)** — Set many key/value pairs atomically. Returns nil.

### TTL Operations

- **KV.ttl(key)** — Remaining TTL in seconds, or null.
- **KV.expire(key, seconds)** — Set TTL on existing key. Returns Bool.
- **KV.pexpire(key, milliseconds)** — Set TTL in milliseconds. Returns Bool.
- **KV.expireat(key, unix_timestamp)** — Expire at an absolute Unix time (seconds). Returns Bool.
- **KV.pttl(key)** — Remaining TTL in milliseconds, or null.
- **KV.persist(key)** — Remove TTL. Returns Bool.
- **KV.touch(...keys)** — Update last-access time. Returns the number of keys that existed.
- **KV.unlink(...keys)** — Delete keys without blocking. Returns the number removed.

### Counters

- **KV.incr(key)** / **KV.decr(key)** — Increment/decrement by 1. Returns new value.
- **KV.incrby(key, amount)** / **KV.decrby(key, amount)** — Increment/decrement by amount. Returns new value.
- **KV.incrbyfloat(key, amount)** — Increment by a floating-point amount. Returns the new value as Float.

### Lists

- **KV.lpush(key, ...values)** / **KV.rpush(key, ...values)** — Push to head/tail. Returns new length.
- **KV.lpop(key)** / **KV.rpop(key)** — Pop from head/tail.
- **KV.lrange(key, start, stop)** — Get range of elements (use `0, -1` for all).
- **KV.llen(key)** — List length.
- **KV.lindex(key, index)** — Element at index (negative counts from the tail). Returns the element or nil.
- **KV.lset(key, index, value)** — Set the element at index. Returns nil.
- **KV.lrem(key, count, value)** — Remove `count` occurrences of `value`. Returns the number removed.
- **KV.ltrim(key, start, stop)** — Trim the list to the given range. Returns nil.
- **KV.rpoplpush(source, dest)** — Pop from `source`'s tail and push to `dest`'s head. Returns the moved element or nil.

### Sets

- **KV.sadd(key, ...members)** / **KV.srem(key, ...members)** — Add/remove set members.
- **KV.smembers(key)** — All members.
- **KV.sismember(key, member)** — Check membership. Returns Bool.
- **KV.smismember(key, ...members)** — Check several members at once. Returns an Array of Bool.
- **KV.scard(key)** — Set cardinality.
- **KV.spop(key, count?)** — Remove and return a random member (or an Array if `count` is given).
- **KV.srandmember(key, count?)** — Return a random member without removing it (or an Array if `count` is given).
- **KV.sinter(...keys)** / **KV.sunion(...keys)** / **KV.sdiff(...keys)** — Intersection / union / difference of sets. Return an Array.
- **KV.smove(source, dest, member)** — Move a member between sets. Returns Bool.

### Hashes

- **KV.hset(key, field, value)** — Set hash field.
- **KV.hsetnx(key, field, value)** — Set a field only if it doesn't exist. Returns Bool.
- **KV.hget(key, field)** — Get hash field.
- **KV.hmget(key, ...fields)** — Get several fields. Returns an Array (nil for missing fields).
- **KV.hgetall(key)** — Get all fields as a Hash.
- **KV.hvals(key)** — All field values. Returns an Array.
- **KV.hdel(key, ...fields)** — Delete fields.
- **KV.hexists(key, field)** — Check field existence. Returns Bool.
- **KV.hkeys(key)** — All field names.
- **KV.hlen(key)** — Number of fields.
- **KV.hincrby(key, field, amount)** — Increment a field by an integer amount. Returns the new value.
- **KV.hincrbyfloat(key, field, amount)** — Increment a field by a floating-point amount. Returns the new value as Float.

### Sorted Sets

Sorted sets keep members ordered by an associated floating-point **score** — ideal for leaderboards, priority queues, and time-ordered feeds.

- **KV.zadd(key, score, member, ...)** — Add one or more score/member pairs. Returns the number of new members.
- **KV.zrem(key, ...members)** — Remove members. Returns the number removed.
- **KV.zscore(key, member)** — Score of a member as Float (or nil).
- **KV.zincrby(key, amount, member)** — Increment a member's score. Returns the new score as Float.
- **KV.zrank(key, member)** / **KV.zrevrank(key, member)** — Rank (0-based) ascending / descending, or nil.
- **KV.zcard(key)** — Number of members.
- **KV.zcount(key, min, max)** — Number of members with score in `[min, max]`.
- **KV.zrange(key, start, stop, with_scores?)** / **KV.zrevrange(key, start, stop, with_scores?)** — Members by rank, ascending / descending. Pass `true` for `with_scores` to interleave scores. Returns an Array.
- **KV.zrangebyscore(key, min, max)** — Members with score in `[min, max]`. Returns an Array.

```soli
# A simple leaderboard
KV.zadd("scores", 100, "alice", 80, "bob", 120, "carol")
KV.zincrby("scores", 25, "bob")             # bob now 105
KV.zrevrange("scores", 0, 2, true)          # top 3 with scores
KV.zrank("scores", "carol")                 # ascending rank
```

### Bitmaps

- **KV.setbit(key, offset, value)** — Set the bit at `offset` to 0 or 1. Returns the previous bit.
- **KV.getbit(key, offset)** — Get the bit at `offset`. Returns 0 or 1.
- **KV.bitcount(key)** — Number of set bits. Returns Int.

### HyperLogLog

A HyperLogLog estimates the number of *distinct* items in a stream (its
**cardinality**) using a fixed ~12 KB sketch — no matter whether you add a hundred
items or a hundred million. It trades a small, bounded error (~0.81% standard error)
for memory that stays constant. Reach for it to count unique visitors, IP addresses,
search terms, or events at scale, where an exact `Set` would grow without bound.

- **KV.pfadd(key, ...elements)** — Add one or more elements to the HLL at `key`
  (created on first use). Returns `1` if the estimate likely changed, else `0`.
- **KV.pfcount(key, ...keys)** — Estimated cardinality. Pass several keys to get the
  cardinality of their **union** without modifying any of them.
- **KV.pfmerge(destkey, ...sourcekeys)** — Merge the source HLLs into `destkey`
  (the union). Returns nil.

```soli
# Count unique visitors for the day without storing every id
KV.pfadd("visitors:2026-06-24", "alice", "bob", "alice")
KV.pfadd("visitors:2026-06-24", "carol")
KV.pfcount("visitors:2026-06-24")            # => ~3 (an estimate, not exact)

# Union several days into a rolling total
KV.pfmerge("visitors:week", "visitors:2026-06-23", "visitors:2026-06-24")
KV.pfcount("visitors:week")                  # => ~unique across both days

# Or estimate the union on the fly, without writing a merged key
KV.pfcount("visitors:2026-06-23", "visitors:2026-06-24")
```

> The count is a probabilistic **estimate** (~0.81% standard error), not an exact
> figure — use it when "roughly how many uniques" is good enough and memory matters.

### Server

- **KV.ping** — Check connectivity.
- **KV.dbsize** — Total number of keys.
- **KV.flushdb** — Delete all keys. **Denied by default** (see [Admin denylist](#admin-denylist)).
- **KV.cmd(...args)** — Run any raw SoliKV command. The first argument is the command verb, which is filtered through the [admin denylist](#admin-denylist).
- **KV.configure(host, token?)** — Configure connection.

### Admin Denylist

`KV.cmd`, `KV.flushdb`, and `KV.keys` refuse destructive or keyspace-wide commands by default. The verb is matched (case-insensitive) against:

```
FLUSHALL  FLUSHDB  KEYS  SCAN
CONFIG  DEBUG  SHUTDOWN  MONITOR  CLIENT
SLAVEOF  REPLICAOF  BGREWRITEAOF  BGSAVE  SAVE
CLUSTER  FAILOVER  RESET  ACL
SCRIPT  EVAL  EVALSHA  FUNCTION
```

A blocked call returns an error rather than running the command. To enable raw admin access, set `SOLI_KV_ALLOW_ADMIN=1` (or `true` / `yes`) in the environment of the process that needs it. Only do this on a privately-deployed admin process — a worker reachable from user traffic should leave this unset, so that a controller bug or template injection cannot reach `KV.cmd("FLUSHALL")`.

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
# Create a rate limiter for API access
limiter = RateLimiter("api:user123", 100, 60)
```

### Instance Methods

**limiter.allowed()**

Checks if a request is allowed under the rate limit.

**Returns:** Bool - true if allowed, false if rate limited

**Example:**
```soli
limiter = RateLimiter("ip:" + req["headers"]["X-Forwarded-For"], 100, 60)
if !limiter.allowed()
  return { "status": 429, "body": "Too Many Requests" }
end
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
limiter = RateLimiter("api:" + api_key, 1000, 3600)
status = limiter.status()
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

> **SEC-056: on by default in production.** As of v0.6x, security headers are enabled out of the box: every response carries `X-Frame-Options: SAMEORIGIN` and `X-Content-Type-Options: nosniff` even without an explicit preset call. `--dev` mode flips them off so the dev bar's inline scripts and the dev REPL aren't second-guessed by a CSP the operator didn't choose.
>
> The `secure_headers()` preset now also sets HSTS (1-year `max-age` + `includeSubDomains`); `secure_headers_strict()` keeps its tighter `DENY` + CSP defaults.

### enable_security_headers()

Enables automatic security header injection on all responses. **Already on by default in production** (SEC-056); call this only to re-enable headers you previously disabled.

**Returns:** Bool

### disable_security_headers()

Disables automatic security header injection. Use to opt out of the default-on behavior — for example, if you serve only API endpoints behind a proxy that already sets all hardening headers.

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
# Generates: default-src 'self' 'https://trusted-cdn.com'
```

### set_hsts(max_age, include_subdomains?, preload?)

Sets the Strict-Transport-Security header.

**Parameters:**
- `max_age` (Int) - Max age in seconds
- `include_subdomains` (Bool, optional) - Include subdomains flag (default: true)
- `preload` (Bool, optional) - Add preload directive (default: false)

**Example:**
```soli
set_hsts(31536000, true, false)  # 1 year, include subdomains
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

Applies the recommended security headers for web apps: `X-Frame-Options: SAMEORIGIN`, `X-Content-Type-Options: nosniff`, `Referrer-Policy: strict-origin-when-cross-origin`, a tight `Permissions-Policy` (no geolocation/microphone/camera), **and HSTS with a 1-year `max-age` + `includeSubDomains`** (SEC-056).

### secure_headers_basic()

Applies basic security headers (X-Frame-Options, X-Content-Type-Options).

### secure_headers_strict()

Applies strict security headers including HSTS, a same-origin CSP, `X-Frame-Options: DENY`, and Cross-Origin-Embedder-Policy.

### secure_headers_api()

Applies minimal security headers suitable for JSON APIs.

### reset_security_headers()

Resets all security header configuration.

### get_security_headers()

Gets the current security headers configuration.

**Returns:** Hash - Current security headers

---

## Trust Proxy

Controls whether the server honors `X-Forwarded-Proto` and `X-Forwarded-Host`
on incoming requests. These headers govern two security-sensitive decisions:

- The `Secure` flag on the session cookie (set when the request scheme is
  `https`).
- The host portion of `*_url` named-route helpers (used to build absolute
  URLs in emails, redirects, etc.).

**By default, trust-proxy is OFF.** A request reaching the Soli process
directly cannot influence either decision via these headers — the cookie
`Secure` flag stays off and `*_url` falls back to the `Host` header.
This is the safe default for a directly-exposed deployment.

Enable trust-proxy only when your deployment terminates TLS at a trusted
proxy hop (Caddy, nginx, ALB, etc.) **and** that proxy is configured to
strip inbound `X-Forwarded-*` headers from clients before adding its own.

### enable_trust_proxy()

Enables honoring of `X-Forwarded-Proto` and `X-Forwarded-Host`.

**Returns:** Bool

**Example:**
```soli
# config/application.sl
enable_trust_proxy()
```

### disable_trust_proxy()

Restores the safe default: ignore `X-Forwarded-*` headers.

**Returns:** Bool

### trust_proxy_enabled()

Returns whether trust-proxy is currently on.

**Returns:** Bool

### `SOLI_TRUST_PROXY` environment variable

The startup default can also come from the environment. Truthy values
(`1`, `true`, `yes`, case-insensitive) flip the gate on; the function calls
still override at runtime.

```bash
# .env.production
SOLI_TRUST_PROXY=1
```

---

## Request Body Limit

Caps the size of incoming request bodies to prevent memory-exhaustion DoS.
The cap applies to every non-GET/HEAD request: the server short-circuits to
**413 Payload Too Large** either via the `Content-Length` header (no bytes
read) or mid-stream once the running total crosses the limit (catches
chunked uploads that don't declare a length).

The default limit is **8 MiB** (8 × 1024 × 1024 bytes). Apps that accept
larger uploads — e.g. document upload, image processing — should raise it
explicitly, ideally only on routes that need it (via per-action checks)
rather than globally.

### set_max_body_size(bytes)

Sets the maximum buffered request body size, in bytes.

**Parameters:**
- `bytes` (Int) - New limit in bytes (must be non-negative)

**Returns:** Int - the value just set

**Example:**
```soli
# config/application.sl
set_max_body_size(32 * 1024 * 1024)  # 32 MiB cap for file uploads
```

### max_body_size()

Returns the current limit in bytes.

**Returns:** Int

### `SOLI_MAX_BODY_SIZE` environment variable

The startup default can also come from the environment. Value is the limit
in bytes; non-numeric or negative values are ignored and the 8 MiB default
stands.

```bash
# .env.production
SOLI_MAX_BODY_SIZE=33554432   # 32 MiB
```

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
files = parse_multipart(req)
for file in files
  println("Uploaded: " + file["filename"] + " (" + str(file["size"]) + " bytes)")
end
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
result = upload_to_solidb(req, "uploads", "avatar", "localhost:5678")
if has_key(result, "blob_id")
  println("Uploaded: " + result["filename"])
  println("Blob ID: " + result["blob_id"])
end
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

## SolidB Standalone Functions

Global functions for connecting to SolidB without creating an instance.

### solidb_connect(address)

Connect to a SolidB server and ping it.

**Parameters:**
- `address` (String) - SolidB server address (e.g., `localhost:5678`)

**Returns:** String - Connection confirmation with ping response

**Example:**
```soli
result = solidb_connect("localhost:5678")
# Returns: "Connected (ping: timestamp)"
```

### solidb_ping(address)

Ping a SolidB server.

**Parameters:**
- `address` (String) - SolidB server address

**Returns:** String - Timestamp from server

### solidb_auth(address, database, username, password)

Authenticate with a SolidB server.

**Parameters:**
- `address` (String) - SolidB server address
- `database` (String) - Database name
- `username` (String) - Username
- `password` (String) - Password

**Returns:** String - "Authenticated" on success

### solidb_query(address, database, sdbql, bindvars?)

Execute a SDBQL query against a SolidB database.

**Parameters:**
- `address` (String) - SolidB server address
- `database` (String) - Database name
- `sdbql` (String) - SDBQL query string
- `bindvars` (Hash, optional) - Bind variables for the query

**Returns:** Array - Query results as array of hashes

**Example:**
```soli
results = solidb_query("localhost:5678", "myapp", "FOR doc IN collection RETURN doc")
```

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
db = Solidb("localhost:5678", "myapp")
blob_id = db.store_blob("avatars", image_data_base64, "photo.jpg", "image/jpeg")
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

## SOAP Class

The `SOAP` class provides methods for making SOAP (Simple Object Access Protocol) calls. SOAP is a protocol for exchanging structured information in web services.

### Static Methods

#### SOAP.call(url, action, envelope, headers?)

Makes a SOAP request to a web service.

**Parameters:**
- `url` (String) - The SOAP service endpoint URL
- `action` (String) - The SOAPAction header value
- `envelope` (String) - The SOAP envelope XML string
- `headers` (Hash, optional) - Additional HTTP headers

**Returns:** Hash - Response with `status`, `headers`, and `body` keys

**Example:**
```soli
envelope = '''<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
 <soap:Body>
  <GetUser xmlns="http://example.com/service">
   <id>123</id>
  </GetUser>
 </soap:Body>
</soap:Envelope>'''

result = SOAP.call(
 "https://example.com/service",
 "GetUser",
 envelope,
 {"Content-Type": "text/xml; charset=utf-8"}
)

if result["status"] == 200
  println("Response: " + result["body"])
end
```

#### SOAP.wrap(body, namespace?, options?)

Wraps an XML body in a SOAP envelope.

**Parameters:**
- `body` (String) - The XML body content
- `namespace` (String, optional) - SOAP namespace (default: SOAP 1.1)
- `options` (Hash, optional) - `{ "escape": true }` to XML-escape the body before wrapping. Use this whenever `body` is untrusted text rather than a trusted XML fragment.

**Returns:** String - Complete SOAP envelope XML

**Example:**
```soli
body = "<GetCustomer><id>42</id></GetCustomer>"
envelope = SOAP.wrap(body)

# Escape user-supplied content
envelope = SOAP.wrap(user_input, null, { "escape": true })
```

#### SOAP.parse(xml_string)

Parses a SOAP XML response into a Hash.

**Parameters:**
- `xml_string` (String) - The SOAP response XML

**Returns:** Hash - Parsed response with extracted data

**Example:**
```soli
response = SOAP.parse(xml_response)
customer_name = response["Body"]["GetCustomerResponse"]["Name"]
```

#### SOAP.xml_escape(string)

Escapes special XML characters in a string.

**Parameters:**
- `string` (String) - The string to escape

**Returns:** String - XML-safe string with <, >, &, ", ' encoded

**Example:**
```soli
safe = SOAP.xml_escape("Tom & Jerry <test>")
# Returns: "Tom &amp; Jerry &lt;test&gt;"
```

#### SOAP.to_xml(hash, root_element?)

Converts a Hash (including nested hashes and arrays) to XML.

**Parameters:**
- `hash` (Hash) - The hash to convert to XML
- `root_element` (String, optional) - Root element name (default: "root")

**Returns:** String - XML string

**Special keys:**
- `@attr_name` - Creates an attribute on the element
- `_text` - Sets the text content of the element

**Example with attributes:**
```soli
data = {
  "user" => {
    "@id" => "123",
    "name" => "John",
    "email" => "john@example.com",
    "address" => {
      "street" => "123 Main St",
      "city" => "Boston"
    },
    "tags" => ["admin", "user"]
  }
}

xml = SOAP.to_xml(data, "users")
# Returns:
# <users>
#   <user id="123">
#     <name>John</name>
#     <email>john@example.com</email>
#     <address>
#       <street>123 Main St</street>
#       <city>Boston</city>
#     </address>
#     <tags_0>admin</tags_0>
#     <tags_1>user</tags_1>
#   </user>
# </users>
```

**Example with _text for element content:**
```soli
data = {
  "product" => {
    "name" => "Laptop",
    "description" => {
      "_text" => "A high-performance laptop with 16GB RAM"
    },
    "price" => "999.99"
  }
}

xml = SOAP.to_xml(data, "catalog")
# Returns:
# <catalog>
#   <product>
#     <name>Laptop</name>
#     <description>A high-performance laptop with 16GB RAM</description>
#     <price>999.99</price>
#   </product>
# </catalog>
```

---

## System Class

The `System` class provides methods for executing system commands.

`System.run` and `System.run_sync` execute a program directly — they do **not** invoke a shell, so metacharacters in arguments are passed through verbatim. Use `System.shell` / `System.shell_sync` (or backtick command substitution) for explicit shell semantics.

### System.run(command)

Runs a command asynchronously and returns a Future. **No shell is invoked.**

**Parameters:**
- `command` (String | Array<String>) - Either a whitespace-split command string with no shell metacharacters, or an argv array `[program, arg1, arg2, ...]`. The argv form is the safe choice when arguments may contain user-controlled values.

**Returns:** Future<Hash> - A future that resolves to `{ stdout: String, stderr: String, exit_code: Int }`

**Errors:** Throws if the string form contains shell metacharacters (`| > < & ; $ ( ) ` ' " * ? [ ] { } ~`). Use `System.shell()` for those, or pass an argv array.

**Example:**
```soli
let result = System.run("echo hello")
# result is a Future that auto-resolves when used

# Access properties directly (auto-resolves)
print(result.stdout)   # "hello"
print(result.exit_code) # 0

# Argv form — safe with user input, no shell expansion
let safe = System.run(["convert", filename, "out.png"])

# Or resolve manually
let output = await(result)
print(output["stdout"])
```

### System.run_sync(command)

Runs a command synchronously (blocking). **No shell is invoked.**

**Parameters:**
- `command` (String | Array<String>) - Same rules as `System.run`.

**Returns:** Hash - `{ stdout: String, stderr: String, exit_code: Int }`

**Example:**
```soli
let result = System.run_sync(["ls", "-la"])
print(result["stdout"])
print(result["exit_code"])
```

### System.shell(command)

Runs a command asynchronously through `sh -c <command>`. Use this when you explicitly want shell features (pipes, redirection, globbing, etc.). **Never pass unsanitised user input here** — every metacharacter is interpreted by the shell. For request-controlled arguments use `System.run([program, arg1, ...])` with an argv array instead, which never invokes a shell. The `smell/dangerous-server-builtin` lint flags `System.shell` calls in `app/controllers/`, `app/middleware/`, and `app/views/`.

**Parameters:**
- `command` (String) - The shell command to execute

**Returns:** Future<Hash>

**Example:**
```soli
let listing = System.shell("ls *.sl | wc -l")
print(listing.stdout)
```

### System.shell_sync(command)

Synchronous variant of `System.shell`. Returns a Hash directly.

```soli
let result = System.shell_sync("grep pattern file.txt")
print(result["exit_code"])
```

### Command Substitution

Backtick syntax is syntactic sugar for `System.shell()` — the literal command is sent through `sh -c`:

```soli
let result = `echo hello`
print(result.stdout)  # "hello"

# Shell features work directly
let files = `ls *.sl`
print(files.stdout)

let status = `grep pattern file`
if status.exit_code != 0
  println("Pattern not found")
end
```

Backticks accept literal source-code commands only (no string interpolation). For commands that include user input, build an argv array and call `System.run` instead.

---

## Image Class

The Image class provides image manipulation capabilities using pure Rust (no system dependencies). All transform methods return a **new Image** instance, so you can chain operations fluently.

### Loading Images

#### Image.new(path)

Loads an image from a file path. Supports JPEG, PNG, GIF, BMP, ICO, TIFF, and WebP.

**Parameters:**
- `path` (String) - Path to the image file

**Returns:** Image instance

**Example:**
```soli
img = Image.new("photo.jpg")
println(img.width)   # 1920
println(img.height)  # 1080
```

#### Image.from_buffer(base64_string)

Loads an image from a base64-encoded string. Useful for processing images received from HTTP requests, S3, or databases.

**Parameters:**
- `base64_string` (String) - Base64-encoded image data

**Returns:** Image instance

**Example:**
```soli
data = S3.get_object("my-bucket", "photo.jpg")
# Encode raw bytes to base64 first, then:
img = Image.from_buffer(base64_data)
```

### Properties

#### img.width

Returns the image width in pixels.

**Returns:** Int

#### img.height

Returns the image height in pixels.

**Returns:** Int

**Example:**
```soli
img = Image.new("photo.jpg")
println("Size: " + str(img.width) + "x" + str(img.height))
```

### Resizing

#### img.resize(width, height)

Resizes the image to the specified dimensions using Lanczos3 filtering (high quality).

**Parameters:**
- `width` (Int) - Target width in pixels
- `height` (Int) - Target height in pixels

**Returns:** New Image instance

**Example:**
```soli
resized = Image.new("photo.jpg").resize(800, 600)
resized.to_file("photo_resized.jpg")
```

#### img.thumbnail(max_size)

Creates a thumbnail that fits within a square of the given size, preserving the original aspect ratio.

**Parameters:**
- `max_size` (Int) - Maximum width or height in pixels

**Returns:** New Image instance

**Example:**
```soli
thumb = Image.new("photo.jpg").thumbnail(200)
thumb.to_file("thumb.jpg")
```

### Cropping

#### img.crop(x, y, width, height)

Crops a rectangular region from the image. Coordinates must be non-negative.

**Parameters:**
- `x` (Int) - Left offset (>= 0)
- `y` (Int) - Top offset (>= 0)
- `width` (Int) - Crop width in pixels
- `height` (Int) - Crop height in pixels

**Returns:** New Image instance

**Example:**
```soli
cropped = Image.new("photo.jpg").crop(100, 50, 400, 300)
cropped.to_file("cropped.jpg")
```

### Transforms

#### img.grayscale()

Converts the image to grayscale.

**Returns:** New Image instance

#### img.flip_horizontal()

Flips the image horizontally (mirror).

**Returns:** New Image instance

#### img.flip_vertical()

Flips the image vertically.

**Returns:** New Image instance

#### img.rotate90()

Rotates the image 90 degrees clockwise.

**Returns:** New Image instance

#### img.rotate180()

Rotates the image 180 degrees.

**Returns:** New Image instance

#### img.rotate270()

Rotates the image 270 degrees clockwise (90 degrees counter-clockwise).

**Returns:** New Image instance

#### img.invert

Inverts all colors in the image.

**Returns:** New Image instance

**Example:**
```soli
img = Image.new("photo.jpg")
  .grayscale()
  .flip_horizontal()
  .rotate90()
img.to_file("transformed.jpg")
```

### Adjustments

#### img.blur(sigma)

Applies a Gaussian blur.

**Parameters:**
- `sigma` (Float or Int) - Blur intensity (higher = more blur)

**Returns:** New Image instance

**Example:**
```soli
blurred = Image.new("photo.jpg").blur(3.5)
```

#### img.brightness(value)

Adjusts image brightness.

**Parameters:**
- `value` (Int) - Brightness adjustment (positive = brighter, negative = darker)

**Returns:** New Image instance

#### img.contrast(value)

Adjusts image contrast.

**Parameters:**
- `value` (Float or Int) - Contrast adjustment (positive = more contrast, negative = less)

**Returns:** New Image instance

#### img.hue_rotate(degrees)

Rotates the hue of all pixels.

**Parameters:**
- `degrees` (Int) - Hue rotation in degrees

**Returns:** New Image instance

**Example:**
```soli
adjusted = Image.new("photo.jpg")
  .brightness(20)
  .contrast(1.5)
  .hue_rotate(90)
adjusted.to_file("adjusted.jpg")
```

### Output Settings

#### img.quality(n)

Sets the output quality for JPEG encoding. Only affects JPEG output.

**Parameters:**
- `n` (Int) - Quality from 1 (worst) to 100 (best), default is 85

**Returns:** New Image instance

#### img.format(fmt)

Sets the output format.

**Parameters:**
- `fmt` (String) - Format name: `"jpeg"`, `"png"`, `"gif"`, `"bmp"`, `"ico"`, `"tiff"`, `"webp"`

**Returns:** New Image instance

**Example:**
```soli
# Convert PNG to JPEG at 70% quality
img = Image.new("photo.png")
  .format("jpeg")
  .quality(70)
img.to_file("photo.jpg")
```

### Saving & Exporting

#### img.to_file(path)

Saves the image to a file. The format is determined by: the format set via `.format()`, or the file extension, or PNG as fallback.

**Parameters:**
- `path` (String) - Output file path

**Returns:** Boolean - `true` on success

**Example:**
```soli
Image.new("photo.jpg").thumbnail(200).to_file("thumb.jpg")
```

#### img.to_buffer()

Encodes the image to a base64 string. Useful for storing in databases, sending in HTTP responses, or passing to S3.

**Returns:** String - Base64-encoded image data

**Example:**
```soli
img = Image.new("photo.jpg").thumbnail(100)
base64_data = img.to_buffer()

# Store in S3
S3.put_object("my-bucket", "thumb.jpg", base64_data, {
  "content_type": "image/jpeg"
})
```

### Parallel Processing

Image transforms are CPU-bound. To process several images concurrently, build a **plan** with `Image.plan(path)`, chain the same transform methods you would on a regular image, and pass an array of plans to `Image.process_all(...)`. Each plan runs on its own thread; the call returns when every plan finishes.

A plan is *lazy* — methods just record operations. Nothing decodes, transforms, or writes until you call `plan.run()` (one plan, current thread) or `Image.process_all([...])` (many plans, in parallel).

#### Image.plan(path)

Creates a new lazy plan that will read its source from `path` when executed.

**Parameters:**
- `path` (String) - Path to the image file (read at execution time)

**Returns:** ImagePlan instance

#### Plan instance methods

A plan supports the same transform methods as `Image`:

`resize(w, h)`, `thumbnail(size)`, `crop(x, y, w, h)`, `grayscale()`, `flip_horizontal()`, `flip_vertical()`, `rotate90()`, `rotate180()`, `rotate270()`, `blur(sigma)`, `brightness(n)`, `contrast(n)`, `invert()`, `hue_rotate(degrees)`, `format(fmt)`, `quality(n)`.

Each call returns a **new ImagePlan instance** with the operation appended.

Plan-only methods:

- `save_to(path)` — terminal; the plan will save to `path` when executed.
- `run()` — executes the plan synchronously on the current thread. Returns `true` if `save_to` was set, otherwise an `Image` instance.
- `src()` — returns the source path.
- `ops_count()` — returns the number of recorded operations.

#### Image.process_all(plans)

Runs an array of plans in parallel (one OS thread per plan) and returns an array of results in the same order.

**Parameters:**
- `plans` (Array of ImagePlan) - Plans to execute concurrently

**Returns:** Array, where each entry is:
- `true` if the plan had `save_to(path)` and the file was written
- An `Image` instance if the plan had no `save_to`
- A hash `{"error": "..."}` if that plan failed (other plans still complete)

**Example — fan out thumbnails:**
```soli
# Generate three thumbnail variants in parallel
results = Image.process_all([
  Image.plan("uploads/raw.jpg").thumbnail(800).save_to("public/large.jpg"),
  Image.plan("uploads/raw.jpg").thumbnail(400).save_to("public/medium.jpg"),
  Image.plan("uploads/raw.jpg").thumbnail(100).save_to("public/small.jpg"),
])

# results == [true, true, true] on success
```

**Example — process many files:**
```soli
let plans = files.map(fn(f) {
  Image.plan(f).resize(1200, 900).quality(80).save_to("processed/" + basename(f))
})
let results = Image.process_all(plans)

# Inspect failures
for r in results
  println("error: " + r.error) if r.error != null
end
```

**Example — collect transformed images without saving:**
```soli
let images = Image.process_all([
  Image.plan("a.jpg").grayscale(),
  Image.plan("b.jpg").rotate90().resize(200, 200),
])
println(images[0].width)
let buf = images[1].to_buffer()
```

> A single chain on `Image.new(...)` is fully synchronous and runs on one thread. Use `Image.plan(...)` + `Image.process_all([...])` only when you have multiple images and want them to run concurrently.

### Complete Examples

**Thumbnail generation in a controller:**
```soli
def upload
  file = req.files["avatar"]
  # `file["data"]` is a base64-encoded string. Image.from_buffer expects
  # base64, so pass it through directly. Use `file["size"]` for the raw
  # byte count instead of `file["data"].length`.
  img = Image.from_buffer(file["data"])

  # Create multiple sizes
  large = img.resize(800, 800)
  medium = img.thumbnail(400)
  small = img.thumbnail(100)

  large.to_file("public/uploads/avatar_large.jpg")
  medium.to_file("public/uploads/avatar_medium.jpg")
  small.to_file("public/uploads/avatar_small.jpg")

  return { "status": 200, "body": "Upload complete" }
end
```

**Image processing pipeline:**
```soli
img = Image.new("raw_photo.jpg")
  .resize(1200, 900)
  .brightness(10)
  .contrast(1.2)
  .quality(85)

img.to_file("processed.jpg")

# Also create a grayscale thumbnail
img.grayscale().thumbnail(200).to_file("thumb_gray.jpg")
```

**Format conversion:**
```soli
# Convert all PNGs to WebP
files = S3.list_objects("images", "photos/")
for file in files
  if file.ends_with(".png")
    data = S3.get_object("images", file)
    img = Image.from_buffer(data)
      .format("webp")
      .quality(80)
    new_key = file.replace(".png", ".webp")
    S3.put_object("images", new_key, img.to_buffer(), {
      "content_type": "image/webp"
    })
  end
end
```

---

## See Also

- [Soli Language Reference](/docs/soli-language) - Core language syntax and features
- [Testing Guide](/docs/testing) - Complete testing documentation
- [Validation Guide](/docs/validation) - Input validation in depth
- [Authentication Guide](/docs/authentication) - JWT authentication patterns
