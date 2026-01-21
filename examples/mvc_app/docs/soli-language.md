# Soli Language Reference

This guide provides a complete reference to the Soli programming language, covering syntax, types, control flow, functions, classes, and more.

## Table of Contents

1. [Introduction](#introduction)
2. [Variables & Types](#variables--types)
3. [Operators](#operators)
4. [Control Flow](#control-flow)
5. [Functions](#functions)
6. [Arrays](#arrays)
7. [Hashes](#hashes)
8. [Classes & OOP](#classes--oop)
9. [Pattern Matching](#pattern-matching)
10. [Pipeline Operator](#pipeline-operator)
11. [Modules](#modules)
12. [Built-in Functions](#built-in-functions)
13. [DateTime & Duration](#datetime--duration)

---

## Introduction

Soli is a modern, statically-typed programming language designed for clarity and expressiveness. It combines object-oriented programming with functional concepts like the pipeline operator.

### Key Features

- **Static Typing with Inference**: Type safety without verbose annotations
- **Pipeline Operator**: Chain function calls for readable data transformation
- **Object-Oriented Programming**: Classes, inheritance, and interfaces
- **Pattern Matching**: Powerful destructuring and matching capabilities

### Hello World

```soli
print("Hello, World!");
```

---

## Variables & Types

### Variable Declaration

Variables are declared using the `let` keyword:

```soli
let name = "Alice";
let age = 30;
let temperature = 98.6;
```

### With Type Annotations

You can explicitly specify types:

```soli
let name: String = "Alice";
let age: Int = 30;
let temperature: Float = 98.6;
let isActive: Bool = true;
```

### Primitive Types

| Type | Description | Example |
|------|-------------|---------|
| `Int` | 64-bit signed integer | `42`, `-100`, `9_000_000` |
| `Float` | 64-bit floating-point | `3.14159`, `0.001`, `2.5e10` |
| `String` | UTF-8 string | `"Hello"`, `"Line 1\nLine 2"` |
| `Bool` | Boolean | `true`, `false` |
| `Null` | Absence of value | `null` |

### Type Inference

Soli infers types when possible:

```soli
let x = 5;          // Int
let y = 3.14;       // Float
let z = "hello";    // String
let flag = true;    // Bool
let nums = [1, 2];  // Int[]
```

### Arrays

Arrays hold multiple values of the same type:

```soli
let numbers: Int[] = [1, 2, 3, 4, 5];
let names = ["Alice", "Bob", "Charlie"];

// Access elements
print(numbers[0]);  // 1
print(names[2]);    // "Charlie"

// Modify elements
numbers[0] = 10;

// Negative indices
print(numbers[-1]);  // Last element
```

### Hashes

Hashes are ordered key-value collections:

```soli
let person = {
    "name": "Alice",
    "age": 30,
    "city": "New York"
};

// Access
print(person["name"]);  // "Alice"

// Modify
person["age"] = 31;

// Missing keys return null
print(person["email"]);  // null

// Alternative syntax
let scores = {"Alice" => 95, "Bob" => 87};
```

### Scope

Variables are block-scoped:

```soli
let x = 1;

if (true) {
    let y = 2;      // y only visible in this block
    let x = 3;      // shadows outer x
    print(x);       // 3
}

print(x);           // 1
```

---

## Operators

### Arithmetic Operators

| Operator | Description | Example |
|----------|-------------|---------|
| `+` | Addition | `5 + 3 = 8` |
| `-` | Subtraction | `5 - 3 = 2` |
| `*` | Multiplication | `5 * 3 = 15` |
| `/` | Division | `6 / 2 = 3` |
| `%` | Modulo | `5 % 2 = 1` |

### Comparison Operators

| Operator | Description |
|----------|-------------|
| `==` | Equal to |
| `!=` | Not equal to |
| `<` | Less than |
| `<=` | Less than or equal |
| `>` | Greater than |
| `>=` | Greater than or equal |

### Logical Operators

```soli
let age = 25;
let hasLicense = true;

// AND
if (age >= 18 && hasLicense) {
    print("Can drive");
}

// OR
if (isWeekend || isHoliday) {
    print("Day off!");
}

// NOT
if (!isRaining) {
    print("No umbrella needed");
}
```

### String Concatenation

```soli
let greeting = "Hello, " + "World!";  // "Hello, World!"
let msg = "Value: " + 42;             // "Value: 42"
```

### Type Coercion

```soli
// Int to Float in mixed arithmetic
let result = 5 + 3.0;  // result is Float: 8.0

// Any type to String with concatenation
let msg = "Value: " + 42;  // "Value: 42"
```

---

## Control Flow

### If/Else

```soli
let age = 18;

if (age >= 18) {
    print("Adult");
}

let score = 75;

if (score >= 60) {
    print("Pass");
} else {
    print("Fail");
}

// Else if chain
let grade = 85;

if (grade >= 90) {
    print("A");
} else if (grade >= 80) {
    print("B");
} else if (grade >= 70) {
    print("C");
} else {
    print("F");
}
```

### While Loop

```soli
let i = 0;
while (i < 5) {
    print(i);
    i = i + 1;
}
// Output: 0, 1, 2, 3, 4
```

### For Loop

```soli
let fruits = ["apple", "banana", "cherry"];

for (fruit in fruits) {
    print(fruit);
}

// With range
for (i in range(0, 5)) {
    print(i);
}
// Output: 0, 1, 2, 3, 4

// Nested loops
for (i in range(1, 4)) {
    for (j in range(1, 4)) {
        print(str(i) + " x " + str(j) + " = " + str(i * j));
    }
}
```

### Postfix Conditionals

Ruby-style postfix `if` and `unless`:

```soli
let x = 10;
print("big") if (x > 5);

let y = 3;
print("small") unless (y > 5);
```

### Ternary Operator

```soli
let x = 10;
let size = x > 5 ? "large" : "small";
// "large"

// Nested
let grade = 85;
let letter = grade >= 90 ? "A"
             : grade >= 80 ? "B"
             : grade >= 70 ? "C"
             : "F";
```

### Truthiness

Falsy values: `false`, `null`, `0`, `""`, `[]`

Everything else is truthy:

```soli
if ("hello") {
    print("Non-empty string is truthy");
}

if ([1, 2, 3]) {
    print("Non-empty array is truthy");
}
```

---

## Functions

### Basic Syntax

```soli
fn functionName(param1: Type1, param2: Type2) -> ReturnType {
    // function body
    return value;
}
```

### Examples

```soli
// No parameters, no return
fn sayHello() {
    print("Hello!");
}

// With parameters
fn greet(name: String) {
    print("Hello, " + name + "!");
}

// With return value
fn add(a: Int, b: Int) -> Int {
    return a + b;
}

// Void functions
fn logMessage(msg: String) {
    print("[LOG] " + msg);
}

// Early return
fn absolute(x: Int) -> Int {
    if (x < 0) {
        return -x;
    }
    return x;
}
```

### Recursive Functions

```soli
fn factorial(n: Int) -> Int {
    if (n <= 1) {
        return 1;
    }
    return n * factorial(n - 1);
}

print(factorial(5));  // 120

fn fibonacci(n: Int) -> Int {
    if (n <= 1) {
        return n;
    }
    return fibonacci(n - 1) + fibonacci(n - 2);
}

print(fibonacci(10));  // 55
```

### Higher-Order Functions

```soli
fn apply(x: Int, f: (Int) -> Int) -> Int {
    return f(x);
}

fn double(x: Int) -> Int {
    return x * 2;
}

let result = apply(5, double);  // 10
```

### Pipeline with Functions

The pipeline operator `|>` passes the left value as the first argument:

```soli
fn double(x: Int) -> Int { return x * 2; }
fn addOne(x: Int) -> Int { return x + 1; }

let result = 5 |> double() |> addOne();  // 11
```

---

## Arrays

### Creating Arrays

```soli
let numbers = [1, 2, 3, 4, 5];
let names = ["Alice", "Bob", "Charlie"];

// With type annotation
let scores: Int[] = [95, 87, 92];
let words: String[] = [];  // Empty array
```

### Array Methods

```soli
let numbers = [1, 2, 3, 4, 5];

// map - transform
let doubled = numbers.map(fn(x) x * 2);
print(doubled);  // [2, 4, 6, 8, 10]

// filter - select
let evens = numbers.filter(fn(x) x % 2 == 0);
print(evens);  // [2, 4]

// each - side effects
numbers.each(fn(x) print(x));
// Prints: 1, 2, 3, 4, 5

// Chaining
let result = numbers
    .map(fn(x) x * 2)
    .filter(fn(x) x > 5);
print(result);  // [6, 8, 10]
```

### Array Functions

```soli
let arr = [1, 2, 3, 4, 5];

len(arr);              // 5
push(arr, 6);          // Add element
pop(arr);              // Remove and return last
range(0, 5);           // [0, 1, 2, 3, 4]
```

---

## Hashes

### Creating Hashes

```soli
let person = {
    "name": "Alice",
    "age": 30,
    "city": "New York"
};

// Alternative syntax
let scores = {"Alice" => 95, "Bob" => 87};
```

### Hash Functions

```soli
let person = {"name": "Alice", "age": 30, "city": "Paris"};

len(person);               // 3
keys(person);              // [name, age, city]
values(person);            // [Alice, 30, Paris]
has_key(person, "name");   // true
delete(person, "age");     // Removes age key
merge(h1, h2);             // Combine hashes
entries(person);           // [[name, Alice], [age, 30], [city, Paris]]
clear(person);             // {}
```

### Hash Methods

```soli
let scores = {"Alice": 90, "Bob": 85, "Charlie": 95};

// map - transform entries
let curved = scores.map(fn(pair) {
    let key = pair[0];
    let value = pair[1];
    return [key, value + 10];
});
print(curved);  // {Alice: 100, Bob: 95, Charlie: 105}

// filter - select entries
let passing = scores.filter(fn(pair) {
    return pair[1] >= 90;
});
print(passing);  // {Alice: 90, Charlie: 95}
```

### Iterating Over Hashes

```soli
let prices = {"apple": 1.50, "banana": 0.75, "orange": 2.00};

// Iterate with entries
for (pair in entries(prices)) {
    let item = pair[0];
    let price = pair[1];
    print(item + " costs $" + str(price));
}

// Iterate keys
for (name in keys(prices)) {
    print(name + ": " + str(prices[name]));
}

// Iterate values
let total = 0;
for (price in values(prices)) {
    total = total + price;
}
```

---

## Classes & OOP

### Basic Class

```soli
class Person {
    name: String;
    age: Int;

    new(name: String, age: Int) {
        this.name = name;
        this.age = age;
    }

    fn greet() -> String {
        return "Hello, I'm " + this.name;
    }
}

let person = new Person("Alice", 30);
print(person.greet());  // Hello, I'm Alice
print(person.name);     // Alice
```

### Constructors

```soli
class Rectangle {
    width: Float;
    height: Float;

    new(w: Float, h: Float) {
        this.width = w;
        this.height = h;
    }

    fn area() -> Float {
        return this.width * this.height;
    }
}

let rect = new Rectangle(10.0, 5.0);
print(rect.area());  // 50.0
```

### Inheritance

```soli
class Animal {
    name: String;

    new(name: String) {
        this.name = name;
    }

    fn speak() -> String {
        return this.name + " makes a sound";
    }
}

class Dog extends Animal {
    breed: String;

    new(name: String, breed: String) {
        this.name = name;
        this.breed = breed;
    }

    fn speak() -> String {
        return this.name + " barks!";
    }
}

let dog = new Dog("Buddy", "Golden Retriever");
print(dog.speak());  // Buddy barks!
```

### Interfaces

```soli
interface Drawable {
    fn draw() -> String;
    fn getColor() -> String;
}

class Circle implements Drawable {
    radius: Float;
    color: String;

    new(r: Float, color: String) {
        this.radius = r;
        this.color = color;
    }

    fn draw() -> String {
        return "Circle with radius " + str(this.radius);
    }

    fn getColor() -> String {
        return this.color;
    }
}

// Multiple interfaces
class Rectangle implements Drawable, Resizable {
    // ...
}
```

### Visibility Modifiers

```soli
class User {
    public name: String;
    private password: String;
    protected email: String;

    new(name: String, password: String, email: String) {
        this.name = name;
        this.password = password;
        this.email = email;
    }

    public fn getName() -> String {
        return this.name;
    }

    private fn hashPassword() -> String {
        return "hashed:" + this.password;
    }
}
```

| Modifier | Access |
|----------|--------|
| `public` | Accessible from anywhere |
| `private` | Only within the class |
| `protected` | Within class and subclasses |

### Static Members

```soli
class MathUtils {
    static PI: Float = 3.14159;

    static fn square(x: Float) -> Float {
        return x * x;
    }

    static fn cube(x: Float) -> Float {
        return x * x * x;
    }
}

print(MathUtils.PI);           // 3.14159
print(MathUtils.square(4.0));  // 16
```

---

## Pattern Matching

Pattern matching provides a powerful way to destructure and match against values.

### Basic Match

```soli
let x = 42;
let result = match x {
    42 => "the answer",
    _ => "something else",
};
print(result);  // "the answer"
```

### Literal Patterns

```soli
let status = "active";
match status {
    "active" => "User is active",
    "pending" => "Awaiting approval",
    "banned" => "Access denied",
    _ => "Unknown status",
};
```

### Guard Clauses

```soli
let n = 5;
match n {
    n if n > 0 => "positive",
    n if n < 0 => "negative",
    0 => "zero",
};
```

### Array Patterns

```soli
let numbers = [1, 2, 3];
match numbers {
    [] => "empty array",
    [first] => "single element: " + str(first),
    [first, second] => "two elements",
    [first, second, ...rest] => "first two: " + str(first) + ", " + str(second),
};
```

### Hash Patterns

```soli
let user = {"name": "Alice", "age": 30};
match user {
    {} => "empty object",
    {name: n} => "name is: " + n,
    {name: n, age: a} => n + " is " + str(a) + " years old",
};
```

### Nested Patterns

```soli
let data = {
    "user": {"name": "Alice", "email": "alice@example.com"},
    "posts": [{"title": "Hello"}, {"title": "World"}]
};

match data {
    {user: {name: n}, posts: [first, ...rest]} => {
        n + " wrote " + str(len(rest) + 1) + " posts";
    },
    _ => "no match",
};
```

### Type-Based Dispatch

```soli
let value: Any = getSomeValue();
match value {
    s: String => "Got a string: " + s,
    n: Int => "Got an integer: " + str(n),
    b: Bool => "Got a boolean: " + str(b),
    _ => "Unknown type",
};
```

---

## Pipeline Operator

The pipeline operator `|>` is one of Soli's most distinctive features. It chains function calls in a readable, left-to-right manner.

### Basic Usage

```soli
fn double(x: Int) -> Int { return x * 2; }
fn addOne(x: Int) -> Int { return x + 1; }
fn square(x: Int) -> Int { return x * x; }

let result = 5 |> double() |> addOne() |> square();
print(result);  // (5 * 2 + 1)^2 = 121
```

### With Multiple Arguments

```soli
fn add(a: Int, b: Int) -> Int {
    return a + b;
}

fn multiply(a: Int, b: Int) -> Int {
    return a * b;
}

// 5 |> add(3) means add(5, 3)
let result = 5 |> add(3) |> multiply(2);  // (5 + 3) * 2 = 16
```

### With Array Methods

```soli
let numbers = [1, 2, 3, 4, 5];

let result = numbers
    .filter(fn(x) x % 2 == 0)
    .map(fn(x) x * 2)
    .each(fn(x) print(x));
```

### With Hash Methods

```soli
let scores = {"Alice": 90, "Bob": 85, "Charlie": 95};

let passingNames = scores
    .filter(fn(pair) pair[1] >= 90)
    .map(fn(pair) pair[0]);

print(passingNames);  // [Alice, Charlie]
```

### Real-World Example

```soli
fn getUser(id: Int) -> Any {
    // Fetch user from database
    return {"id": id, "name": "Alice", "posts": []};
}

fn getPosts(userId: Int) -> Any {
    // Fetch posts
    return [{"title": "Post 1"}, {"title": "Post 2"}];
}

fn formatUserData(user: Any) -> Any {
    return {
        "name": user["name"],
        "postCount": len(user["posts"])
    };
}

let userData = getUser(1)
    |> fn(u) { u["posts"] = getPosts(u["id"]); return u; }()
    |> formatUserData();

print(userData);  // {name: Alice, postCount: 2}
```

---

## Modules

### Export Declarations

```soli
// math.soli
export fn add(a: Int, b: Int) -> Int {
    return a + b;
}

export fn multiply(a: Int, b: Int) -> Int {
    return a * b;
}

// Private - not exported
fn internal_helper(x: Int) -> Int {
    return x + 1;
}
```

### Import Statements

```soli
// Import all exports
import "./math.soli";
let result = add(2, 3);

// Named imports
import { add, multiply } from "./math.soli";
let sum = add(1, 2);
let product = multiply(3, 4);

// Aliased imports
import { add as sum, multiply as mul } from "./math.soli";
let result = sum(1, 2);
```

### Package Configuration (soli.toml)

```toml
[package]
name = "my-app"
version = "1.0.0"
description = "My awesome Soli application"
main = "src/main.soli"

[dependencies]
utils = { path = "./lib/utils" }
```

### Example Project Structure

```
my-project/
├── soli.toml
├── src/
│   ├── main.soli
│   └── utils.soli
└── lib/
    ├── math/
    │   ├── mod.soli
    │   ├── basic.soli
    │   └── advanced.soli
    └── utils/
        └── index.soli
```

---

## Built-in Functions

### I/O Functions

| Function | Description |
|----------|-------------|
| `print(...)` | Print values to stdout |
| `input(prompt?)` | Read line from stdin |

### Type Conversion

| Function | Description |
|----------|-------------|
| `str(x)` | Convert to string |
| `int(x)` | Convert to integer |
| `float(x)` | Convert to float |
| `type(x)` | Get type name |

### Array Functions

| Function | Description |
|----------|-------------|
| `len(x)` | Get length of array/string/hash |
| `push(arr, val)` | Add element to array |
| `pop(arr)` | Remove and return last element |
| `range(start, end)` | Create range array |

### Math Functions

| Function | Description |
|----------|-------------|
| `abs(x)` | Absolute value |
| `min(a, b)` | Minimum of two values |
| `max(a, b)` | Maximum of two values |
| `sqrt(x)` | Square root |
| `pow(a, b)` | Exponentiation |
| `clock()` | Current time in seconds |

### Hash Functions

| Function | Description |
|----------|-------------|
| `keys(h)` | Get all keys |
| `values(h)` | Get all values |
| `has_key(h, k)` | Check if key exists |
| `delete(h, k)` | Remove and return value |
| `merge(h1, h2)` | Combine hashes |
| `entries(h)` | Get [key, value] pairs |
| `clear(h)` | Remove all entries |

### File I/O

| Function | Description |
|----------|-------------|
| `barf(path, content)` | Write file (text or binary) |
| `slurp(path, mode?)` | Read file (text or binary) |

### HTTP Functions

| Function | Description |
|----------|-------------|
| `http_get(url)` | GET request (async) |
| `http_get_json(url)` | GET and parse JSON (async) |
| `http_post(url, body)` | POST request (async) |
| `http_request(...)` | Generic HTTP request (async) |

### Regex Functions

| Function | Description |
|----------|-------------|
| `regex_match(pattern, string)` | Check if string matches pattern |
| `regex_find(pattern, string)` | Find first match |
| `regex_find_all(pattern, string)` | Find all matches |
| `regex_replace(pattern, string, replacement)` | Replace first match |
| `regex_replace_all(pattern, string, replacement)` | Replace all matches |
| `regex_split(pattern, string)` | Split by pattern |
| `regex_capture(pattern, string)` | Find with named groups |
| `regex_escape(string)` | Escape regex characters |

### Cryptographic Functions

| Function | Description |
|----------|-------------|
| `argon2_hash(password)` | Hash password with Argon2id |
| `argon2_verify(password, hash)` | Verify password against hash |
| `x25519_keypair()` | Generate X25519 key pair, returns `{private, public}` |
| `x25519_shared_secret(private, public)` | Compute shared secret from key pair |
| `x25519_public_key(private)` | Derive public key from private key |
| `x25519(basepoint, scalar)` | Perform X25519 scalar multiplication |
| `ed25519_keypair()` | Generate Ed25519 key pair for digital signatures |

### X25519 Key Exchange Example

```soli
// Alice generates her key pair
let alice_keys = x25519_keypair();
let alice_private = alice_keys["private"];
let alice_public = alice_keys["public"];

// Bob generates his key pair
let bob_keys = x25519_keypair();
let bob_private = bob_keys["private"];
let bob_public = bob_keys["public"];

// They exchange public keys and compute shared secret
let alice_shared = x25519_shared_secret(alice_private, bob_public);
let bob_shared = x25519_shared_secret(bob_private, alice_public);

// Both now have the same shared secret (for key derivation)
print(alice_shared == bob_shared);  // true
```

### Ed25519 Key Pair Example

```soli
// Generate Ed25519 key pair for digital signatures
let keys = ed25519_keypair();
let private_key = keys["private"];  // 64-char hex string
let public_key = keys["public"];    // 64-char hex string
```

### HTML Functions

| Function | Description |
|----------|-------------|
| `html_escape(string)` | Escape HTML special characters |
| `html_unescape(string)` | Unescape HTML entities |
| `sanitize_html(string)` | Remove dangerous HTML (XSS prevention) |

### JSON Functions

| Function | Description |
|----------|-------------|
| `json_parse(s)` | Parse JSON string |
| `json_stringify(v)` | Convert to JSON |

---

## DateTime & Duration

The `DateTime` and `Duration` classes provide comprehensive date and time functionality.

### Creating DateTime Instances

```soli
// Current local time
let now = datetime_now();

// Current UTC time
let utc_now = DateTime.utc();

// Parse from string
let parsed = DateTime.parse("2024-01-15T10:30:00");
```

### DateTime Instance Methods

| Method | Return Type | Description |
|--------|-------------|-------------|
| `year()` | Int | Get year (4-digit) |
| `month()` | Int | Get month (1-12) |
| `day()` | Int | Get day of month (1-31) |
| `hour()` | Int | Get hour (0-23) |
| `minute()` | Int | Get minute (0-59) |
| `second()` | Int | Get second (0-59) |
| `weekday()` | String | Get day name ("monday"-"sunday") |
| `to_unix()` | Int | Get Unix timestamp (seconds since epoch) |
| `to_iso()` | String | Get ISO 8601 formatted string |
| `to_string()` | String | Get human-readable string |
| `add(dur)` | DateTime | Add a Duration to this DateTime |
| `sub(dur)` | DateTime | Subtract a Duration from this DateTime |

### DateTime Arithmetic

```soli
let now = DateTime.utc();
let tomorrow = now.add(Duration.days(1));
let yesterday = now.sub(Duration.days(1));
let next_week = now.add(Duration.weeks(1));
```

### DateTime Example

```soli
let now = DateTime.utc();
print("Year: " + str(now.year()));
print("Month: " + str(now.month()));
print("Day: " + str(now.day()));
print("Weekday: " + now.weekday());
print("Unix timestamp: " + str(now.to_unix()));
```

### Duration Class

```soli
// Duration between two DateTimes
let start = DateTime.parse("2024-01-01T00:00:00");
let end = DateTime.parse("2024-01-02T12:00:00");
let dur = Duration.between(start, end);

// Duration from value
let dur = Duration.seconds(3600);      // 1 hour
let dur = Duration.minutes(60);        // 60 minutes
let dur = Duration.hours(24);          // 24 hours
let dur = Duration.days(7);            // 7 days
let dur = Duration.weeks(2);           // 2 weeks
```

### Duration Instance Methods

| Method | Return Type | Description |
|--------|-------------|-------------|
| `total_seconds()` | Float | Total duration in seconds |
| `total_minutes()` | Float | Total duration in minutes |
| `total_hours()` | Float | Total duration in hours |
| `total_days()` | Float | Total duration in days |
| `total_weeks()` | Float | Total duration in weeks |
| `to_string()` | String | Human-readable string |

### Example

```soli
let start = DateTime.parse("2024-01-01T00:00:00");
let end = DateTime.parse("2024-01-02T12:00:00");
let dur = Duration.between(start, end);

print("Hours: " + str(dur.total_hours()));  // 36.0
print("Days: " + str(dur.total_days()));    // 1.5
```

---

### Variables & Types

1. **Use type inference** when the type is obvious
2. **Add annotations** when clarity helps
3. **Use meaningful names**

### Functions

1. **Single responsibility**: Each function should do one thing well
2. **Descriptive names**: Use verbs for actions
3. **Limit parameters**: Consider using objects for many parameters

### Arrays & Hashes

1. **Initialize with known values** when possible
2. **Check bounds** before accessing by index
3. **Use `for-in`** for simple iteration
4. **Use `has_key()`** when unsure if a hash key exists

### Classes

1. **Single Responsibility**: Each class should have one purpose
2. **Prefer Composition**: Use composition over deep inheritance hierarchies
3. **Program to Interfaces**: Depend on abstractions, not concrete classes
4. **Encapsulate State**: Use private fields with public methods

### Control Flow

1. **Avoid deep nesting**: Extract complex conditions into functions
2. **Use early returns**: Simplify logic flow
3. **Prefer for-in over while** for iteration when possible

---

## Quick Reference

### Common Patterns

**Hello World:**
```soli
print("Hello, World!");
```

**Function definition:**
```soli
fn add(a: Int, b: Int) -> Int {
    return a + b;
}
```

**Class definition:**
```soli
class Person {
    name: String;

    new(name: String) {
        this.name = name;
    }

    fn greet() -> String {
        return "Hello, I'm " + this.name;
    }
}
```

**If/else:**
```soli
if (condition) {
    // code
} else if (otherCondition) {
    // code
} else {
    // code
}
```

**For loop:**
```soli
for (item in collection) {
    // code
}
```

**While loop:**
```soli
while (condition) {
    // code
}
```

**Pattern match:**
```soli
match value {
    pattern1 => result1,
    pattern2 => result2,
    _ => defaultResult,
}
```

**Pipeline:**
```soli
value |> function1() |> function2();
```

**Array iteration:**
```soli
array.map(fn(x) x * 2);
array.filter(fn(x) x > 0);
array.each(fn(x) print(x));
```

**Hash iteration:**
```soli
hash.map(fn(pair) [pair[0], pair[1] * 2]);
hash.filter(fn(pair) pair[1] > 0);
```

---

## Next Steps

- Explore the [MVC Framework](/docs/introduction/)
- Learn about [Routing](/docs/routing/)
- Understand [Controllers](/docs/controllers/)
- Master [Views](/docs/views/)
- Implement [Middleware](/docs/middleware/)
