# Soli Language Enhancement Specifications

This document specifies the new language features to be implemented.

---

## 1. Pattern Matching

### Syntax
```soli
match expression {
    pattern => expression,
    pattern if condition => expression,
    _ => expression  // wildcard
}
```

### Supported Patterns

**Literal Patterns:**
```soli
match x {
    42 => "forty-two",
    "hello" => "greeting",
    true => "truthy",
    null => "nothing",
}
```

**Variable Patterns:**
```soli
match user {
    {name, age} => name + " is " + str(age),
}
```

**Typed Patterns:**
```soli
match value {
    s: String => "String: " + s,
    n: Int => "Int: " + str(n),
}
```

**Nested Patterns:**
```soli
match data {
    {user: {name, email}, posts: [first, ...]} => name,
}
```

**Array Patterns:**
```soli
match list {
    [] => "empty",
    [x] => "single: " + x,
    [first, second, ...rest] => first + " and " + second,
}
```

**Guard Clauses:**
```soli
match x {
    n: Int if n > 0 => "positive",
    n: Int if n < 0 => "negative",
    0 => "zero",
}
```

### Destructuring
Extract values from hashes and arrays:
```soli
let user = {"name": "Alice", "age": 30, "city": "Paris"};
let {name, age} = user;  // name = "Alice", age = 30

let items = [1, 2, 3, 4, 5];
let [first, second, ...rest] = items;  // first = 1, second = 2, rest = [3, 4, 5]
```

---

## 2. Closures / Lambdas

### Syntax
```soli
fn(params) { body }
fn(params) => expression  // concise form
```

### Examples
```soli
let add = fn(a, b) { a + b };
let double = fn(x) => x * 2;
let apply = fn(f, x) { f(x) };

// Higher-order functions
[1, 2, 3].map(fn(x) { x * 2 });
[1, 2, 3].filter(fn(x) { x > 1 });

// Closure captures environment
let multiplier = 10;
let times10 = fn(x) { x * multiplier };
```

---

## 3. Exception Handling

### Syntax
```soli
try {
    // code that might throw
    risky_operation()
} catch error {
    // handle error
    print("Error: " + str(error))
} finally {
    // always runs
    cleanup()
}
```

### Throwing Exceptions
```soli
throw "error message"
throw MyError.new("details")

// Custom error classes
class MyError {
    message: String;
    
    new(msg: String) {
        this.message = msg;
    }
    
    fn toString() -> String {
        return this.message;
    }
}
```

### Built-in Error Types
```soli
class ValueError extends Error { }
class TypeError extends Error { }
class KeyError extends Error { }
class IndexError extends Error { }
```

---

## 4. Default Parameters

### Syntax
```soli
fn greet(name: String, prefix: String = "Hello") -> String {
    return prefix + " " + name;
}

greet("Alice");                    // "Hello Alice"
greet("Bob", "Welcome");          // "Welcome Bob"
```

### Default Expressions
```soli
fn create_user(name: String, role: String = "user", active: Bool = true) {
    // ...
}
```

---

## 5. Spread / Rest Operators

### Spread in Arrays
```soli
let a = [1, 2, 3];
let b = [0, ...a, 4];  // [0, 1, 2, 3, 4]
```

### Spread in Hashes
```soli
let defaults = {"host": "localhost", "port": 8080};
let config = {"port": 3000, ...defaults};
// {"host": "localhost", "port": 3000}
```

### Rest in Parameters
```soli
fn sum(...numbers: Int[]) -> Int {
    let total = 0;
    for n in numbers {
        total = total + n;
    }
    return total;
}

sum(1, 2, 3, 4, 5);  // 15
```

### Rest in Destructuring
```soli
let [head, ...tail] = [1, 2, 3, 4, 5];
// head = 1, tail = [2, 3, 4, 5]

let {name, ...rest} = {"name": "Alice", "age": 30, "city": "Paris"};
// name = "Alice", rest = {"age": 30, "city": "Paris"}
```

---

## 6. String Interpolation

### Syntax
```soli
let name = "Alice";
let age = 30;
let message = "User \(name) is \(age) years old";
// "User Alice is 30 years old"
```

### Expressions in Interpolation
```soli
let x = 10;
let y = 20;
let result = "\(x) + \(y) = \(x + y)";
// "10 + 20 = 30"
```

### Multi-line Strings
```soli
let template = @"
Hello \(name),
Your order #\(order_id) is ready.
Total: $\(total)
";
```

---

## 7. Async/Await

### Syntax
```soli
let result = await some_async_operation();

// With error handling
let result = try {
    await risky_async_call()
} catch error {
    default_value
}
```

### Async Functions
```soli
async fn fetch_data(url: String) -> Any {
    let response = await http_get(url);
    return json_parse(response);
}

// Calling async functions
let data = await fetch_data("https://api.example.com/data");
```

---

## 8. Comprehensions

### List Comprehensions
```soli
let squares = [x * x for x in numbers];
let evens = [x for x in numbers if x % 2 == 0];
let pairs = [a + b for a in [1,2,3] for b in [4,5,6]];
```

### Hash Comprehensions
```soli
let squares = {x: x * x for x in numbers};
let filtered = {k: v for (k, v) in hash.items() if v > 0};
```

---

## 8. Iteration Methods (map, filter, each)

Array and hash methods for functional-style iteration.

### Array Methods

#### `map` - Transform elements
```soli
let numbers = [1, 2, 3, 4, 5];

// Expression body (implicit return)
let doubled = numbers.map(fn(x) x * 2);
// [2, 4, 6, 8, 10]

// Block body (explicit return)
let squares = numbers.map(fn(x) {
    return x * x;
});
// [1, 4, 9, 16, 25]
```

#### `filter` - Select elements
```soli
let numbers = [1, 2, 3, 4, 5];

// Keep elements where function returns truthy
let evens = numbers.filter(fn(x) x % 2 == 0);
// [2, 4]

// With block syntax
let large = numbers.filter(fn(n) {
    return n > 3;
});
// [4, 5]
```

#### `each` - Side effects
```soli
let numbers = [1, 2, 3];

// Execute function for each element, returns original array
numbers.each(fn(x) print(x));
// Prints: 1, 2, 3
// Returns: [1, 2, 3]
```

### Chaining
```soli
let numbers = [1, 2, 3, 4, 5];

// Chain multiple operations
let result = numbers
    .map(fn(x) x * 2)
    .filter(fn(x) x > 5);
// [6, 8, 10]
```

### Hash Methods

Hash methods pass a `[key, value]` array to the callback.

#### Hash `map`
```soli
let scores = {"Alice": 90, "Bob": 85};

// Transform values, return [new_key, new_value]
let curved = scores.map(fn(pair) {
    return [pair[0], pair[1] + 10];
});
// {"Alice": 100, "Bob": 95}
```

#### Hash `filter`
```soli
let scores = {"Alice": 90, "Bob": 85, "Charlie": 95};

// Keep entries where function returns truthy
let winners = scores.filter(fn(pair) {
    return pair[1] >= 90;
});
// {"Alice": 90, "Charlie": 95}
```

#### Hash `each`
```soli
let user = {"name": "Alice", "age": 30};

user.each(fn(pair) {
    print(pair[0] + ": " + pair[1]);
});
// Prints: "name: Alice", "age: 30"
```

---

## 9. Modules / Imports

### File Structure
```
myproject/
├── main.sl
├── utils/
│   ├── mod.sl
│   ├── math.sl
│   └── strings.sl
└── models/
    └── user.sl
```

### Import Syntax
```soli
// Import entire module
import "./utils/math.sl";
print(utils.add(1, 2));

// Import specific items
from "./utils/strings" import {uppercase, lowercase};

// Re-export from module
// In utils/mod.sl:
pub mod strings;
pub mod math;

// Export declarations
pub fn helper() { }
pub class MyClass { }
```

### Module Scope
```soli
// utils.sl
let private_var = "hidden";
pub fn public_fn() { }
```

---

## Implementation Priority

1. **Phase 1 (Foundation) - COMPLETED**
   - Closures/Lambdas
   - Default Parameters
   - String Interpolation
   - Iteration Methods (map, filter, each)

2. **Phase 2 (Pattern Matching)**
   - Pattern matching expression
   - Destructuring
   - Guards

3. **Phase 3 (Error Handling)**
   - try/catch/throw
   - Built-in error types
   - Custom error classes

4. **Phase 4 (Modern Features) - COMPLETED**
   - Spread/Rest operators
   - Comprehensions
   - Async/Await
   - Nullish Coalescing Operator (`??`)

5. **Phase 5 (Modules)**
   - Import/Export system
   - Module resolution
   - Module scope

---

## 10. Constants

### Syntax
```soli
const PI = 3.14159;
const GREETING: String = "Hello!";
const CONFIG = {"host": "localhost", "port": 3000};
```

### Characteristics
```soli
const VALUE = 42;
// VALUE = 100;  // Error: cannot reassign constant

const ARR = [1, 2, 3];
// ARR[0] = 100;  // Error: cannot reassign constant

const H = {"key": "value"};
// H["key"] = "new";  // Error: cannot reassign constant

// const works in function scope
fn calculate() {
    const PI = 3.14159;
    return PI * radius * radius;
}
```

### Constants in Expressions
```soli
const A = 10;
const B = 20;
const SUM = A + B;  // 30

const NAMES = ["Alice", "Bob"];
const COUNT = len(NAMES);  // 2
```

---

## 11. Static Blocks

### Syntax
```soli
class MyClass {
    static {
        this.counter = 0;
        this.config = {"timeout": 30};
    }
}
```

### Examples
```soli
class Config {
    static {
        Config.timeout = 30;
        Config.max_retries = 3;
        Config.initialized = true;
    }
}

class MathHelper {
    static {
        MathHelper.result = get_value();
    }
}
```

### Static Block Features
```soli
class Processor {
    static {
        Processor.initialized = true;
        Processor.start_time = 100;
        Processor.end_time = 200;
    }
}

// Control flow in static blocks
class Config {
    static {
        if (true) {
            Config.value = "yes";
        } else {
            Config.value = "no";
        }
    }
}

// Loops in static blocks
class Counter {
    static {
        Counter.sum = 0;
        for (i in [1, 2, 3, 4, 5]) {
            Counter.sum = Counter.sum + i;
        }
    }
}
```

---

## 12. Nullish Coalescing Operator

### Syntax
```soli
let result = null ?? "default";    // "default"
let result = "value" ?? "default"; // "value"
```

### Behavior
```soli
// Returns right operand only if left is null
assert_eq(null ?? "fallback", "fallback");
assert_eq("value" ?? "fallback", "value");

// Falsy values (0, false, "") are NOT considered null
assert_eq(0 ?? 100, 0);
assert_eq(false ?? true, false);
assert_eq("" ?? "default", "");
```

### Chaining
```soli
let result = null ?? null ?? "final";
// "final"

let result = null ?? null ?? "third" ?? "fourth";
// "third"
```

### With Other Operators
```soli
// Combined with arithmetic
let result = null ?? 5 + 3;  // 8

// Precedence with logical operators
let result = true && (null ?? "fallback");  // "fallback"
let result = false || (null ?? "fallback"); // "fallback"

// In function parameters
fn greet(name) {
    return "Hello, " + (name ?? "Guest") + "!";
}

greet(null);   // "Hello, Guest!"
greet("Alice"); // "Hello, Alice!"
```

### Nested Usage
```soli
// Nested nullish coalescing
let config = null;
let result = (config ?? { db: null }).db ?? "sqlite";
// "sqlite"

let config2 = { db: "postgresql" };
let result2 = (config2 ?? { db: null }).db ?? "sqlite";
// "postgresql"

// In array literals
let arr = [1, null ?? 2, 3];  // [1, 2, 3]

// In hash literals
let h = { a: null ?? 1, b: "value" ?? 2 };  // {a: 1, b: "value"}
```

---

## 13. Chainable Collection Methods

### String Methods
```soli
let s = "hello world";

s.length();       // 11
s.upcase();       // "HELLO WORLD"
s.downcase();     // "hello world"
s.trim();         // "hello world"
s.contains("wor"); // true
s.starts_with("hello"); // true
s.ends_with("world");   // true
s.split(" ");     // ["hello", "world"]
s.index_of("wor"); // 6
s.substring(0, 5); // "hello"
s.replace("world", "soli"); // "hello soli"
s.lpad(10);       // " hello world"
s.lpad(10, "*");  // "***hello world"
s.rpad(10);       // "hello world "
s.rpad(10, "*");  // "hello world***"
```

### Array Methods
```soli
let arr = [1, 2, 3];

arr.to_string();  // "[1, 2, 3]"
arr.length();     // 3
arr.push(4);      // [1, 2, 3, 4]
arr.pop();        // 3, arr is now [1, 2]
arr.get(0);       // 1
arr.get(-1);      // 3 (negative index)
arr.clear();      // []
arr.first();      // 1
arr.last();       // 3
arr.reverse();    // [3, 2, 1]
arr.uniq();       // removes duplicates
arr.take(2);      // first 2 elements
arr.drop(2);      // without first 2 elements
arr.sum();        // 6
arr.min();        // 1
arr.max();        // 3
arr.empty?();     // false
arr.include?(2);  // true
arr.join("-");    // "1-2-3"
arr.zip([4, 5]);  // [[1, 4], [2, 5]]
```

### Hash Methods
```soli
let h = {"name": "test", "value": 42};

h.to_string();    // hash representation
h.length();       // 2
h.get("name");    // "test"
h.set("key", "new");
h.has_key("name"); // true
h.keys();         // ["name", "value"]
h.values();       // ["test", 42]
h.delete("name"); // returns deleted value
h.merge({"extra": 100}); // combines hashes
h.entries();      // [["name", "test"], ["value", 42]]
h.clear();        // {}
```

---

## 14. Base64 Utility

### Functions
```soli
let encoded = Base64.encode("hello");
// "aGVsbG8="

let decoded = Base64.decode("aGVsbG8=");
// "hello"

// Round-trip
let original = "Hello, World! 123";
let encoded = Base64.encode(original);
let decoded = Base64.decode(encoded);
assert_eq(decoded, original);

// Special characters
Base64.encode("Hello\nWorld\t!");
// "SGVsbG8KV29ybGQhIQ=="

// URL-safe strings
Base64.encode("foo/bar?query=value");
// correctly encodes and decodes
```

---

## 15. State Machines

### StateMachine Class
```soli
import "stdlib/state_machine.sl";

let states = ["pending", "processing", "completed", "failed"];
let transitions = [
    {"event": "start", "from": "pending", "to": "processing"},
    {"event": "finish", "from": "processing", "to": "completed"},
    {"event": "fail", "from": "processing", "to": "failed"},
    {"event": "retry", "from": "failed", "to": "processing"},
];

let machine = new StateMachine("pending", states, transitions);

// Check state
machine.current_state();  // "pending"
machine.is("pending");    // true
machine.is_in(["pending", "processing"]); // true

// Check available transitions
machine.can("start");         // true
machine.can("finish");        // false
machine.available_events();   // ["start"]

// Transition
let result = machine.transition("start");
// {"success": true, "from": "pending", "to": "processing", "event": "start"}
machine.current_state();  // "processing"

// Context storage
machine.set("user_id", "123");
machine.get("user_id");   // "123"

// History
machine.history();         // all transitions
machine.last_transition(); // last transition details
```

### StateMachineBuilder
```soli
import "stdlib/state_machine.sl";

let machine = state_machine()
    .initial("pending")
    .states_list(["pending", "active", "done"])
    .transition("activate", "pending", "active")
    .transition("complete", "active", "done")
    .build();
```

### StateMachineBuilder API
```soli
let builder = state_machine();

builder.initial("off");                    // Set initial state
builder.states_list(["off", "on"]);        // Define all states
builder.transition("turn_on", "off", "on"); // Single source
builder.transition(["off", "broken"], "on", "off"); // Multiple sources
builder.build();                           // Create StateMachine instance
```

---

## Examples of Combined Features

### Elegant Data Processing
```soli
let users = [
    {"name": "Alice", "age": 30, "city": "Paris"},
    {"name": "Bob", "age": 25, "city": "London"},
    {"name": "Charlie", "age": 35, "city": "Paris"},
];

// Comprehensions + pattern matching
let paris_users = [
    {name, city} 
    for user in users 
    if user["city"] == "Paris"
];

// With destructuring
let result = match users {
    [] => "No users",
    [{name, age: a}, ...rest] if a > 28 => name + " and " + str(len(rest)) + " others",
    users => "Found " + str(len(users)) + " users",
};
```

### Async with Error Handling
```soli
async fn fetch_user(id: String) -> Any {
    try {
        let user = await db.get("users", id);
        return match user {
            null => throw UserNotFoundError.new(id),
            user => user,
        };
    } catch error {
        log_error(error);
        return null;
    }
}
```

### Spread + Comprehensions
```soli
let base_config = {"host": "localhost", "port": 8080};
let env_config = getenv("APP_CONFIG");
let final_config = {
    ...base_config,
    ...json_parse(env_config),
    "debug": true,
};

let debug_keys = [k for (k, v) in final_config if v == true];
```

### Constants + Nullish Coalescing + Chainable Methods
```soli
// Constants for configuration
const DEFAULT_TIMEOUT = 30;
const DEFAULT_RETRIES = 3;

fn process_config(user_config) {
    // Nullish coalescing with defaults
    let timeout = user_config.timeout ?? DEFAULT_TIMEOUT;
    let retries = user_config.retries ?? DEFAULT_RETRIES;
    let debug = user_config.debug ?? false;

    // Chainable methods with null safety
    let name = user_config.name ?? "Unnamed";
    let upper_name = name.upcase();
    let trimmed_name = upper_name.trim();

    // Array methods with defaults
    let tags = user_config.tags ?? [];
    let first_tag = tags.first() ?? "general";

    return {
        "timeout": timeout,
        "retries": retries,
        "debug": debug,
        "name": trimmed_name,
        "first_tag": first_tag,
    };
}
```

### State Machine + Constants + Nullish Coalescing
```soli
import "stdlib/state_machine.sl";

// Constants for state machine configuration
const INITIAL_STATE = "idle";
const FINAL_STATES = ["completed", "cancelled"];

fn create_order_machine() {
    let states = ["idle", "pending", "processing", "shipped", "completed", "cancelled"];
    let transitions = [
        {"event": "submit", "from": "idle", "to": "pending"},
        {"event": "process", "from": "pending", "to": "processing"},
        {"event": "ship", "from": "processing", "to": "shipped"},
        {"event": "deliver", "from": "shipped", "to": "completed"},
        {"event": "cancel", "from": ["idle", "pending"], "to": "cancelled"},
    ];

    let machine = new StateMachine(INITIAL_STATE, states, transitions);
    machine.set("order_id", generate_order_id());
    machine.set("created_at", current_timestamp());

    return machine;
}

// Usage with nullish coalescing
fn get_order_status(machine) {
    let state = machine.current_state();
    let order_id = machine.get("order_id") ?? "unknown";

    return "Order \(order_id) is \(state)";
}
```

### Base64 + String Methods + Constants
```soli
const API_BASE_URL = "https://api.example.com";

fn encode_auth_header(username, password) {
    let credentials = username + ":" + password;
    let encoded = Base64.encode(credentials);
    return "Basic " + encoded;
}

fn decode_token(token) {
    // Remove "Bearer " prefix if present
    let clean_token = token.starts_with("Bearer ")
        ? token.substring(7, len(token))
        : token;

    let decoded = Base64.decode(clean_token);
    let parts = decoded.split(":");

    return {
        "username": parts[0] ?? "",
        "password": parts[1] ?? "",
    };
}
```
