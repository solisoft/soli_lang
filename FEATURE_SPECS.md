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
├── main.soli
├── utils/
│   ├── mod.soli
│   ├── math.soli
│   └── strings.soli
└── models/
    └── user.soli
```

### Import Syntax
```soli
// Import entire module
import "./utils/math.soli";
print(utils.add(1, 2));

// Import specific items
from "./utils/strings" import {uppercase, lowercase};

// Re-export from module
// In utils/mod.soli:
pub mod strings;
pub mod math;

// Export declarations
pub fn helper() { }
pub class MyClass { }
```

### Module Scope
```soli
// utils.soli
let private_var = "hidden";
pub fn public_fn() { }
```

---

## Implementation Priority

1. **Phase 1 (Foundation)**
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

4. **Phase 4 (Modern Features)**
   - Spread/Rest operators
   - Comprehensions
   - Async/Await

5. **Phase 5 (Modules)**
   - Import/Export system
   - Module resolution
   - Module scope

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
