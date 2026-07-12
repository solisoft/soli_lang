# Soli Language Reference

A comprehensive guide to the Soli programming language, covering syntax, types, control flow, functions, classes, and advanced features.

---

## Table of Contents

1. [Getting Started](#getting-started)
2. [Variables & Types](#variables--types)
3. [Operators](#operators)
4. [Control Flow](#control-flow)
5. [Error Handling](#error-handling)
6. [Functions](#functions)
7. [Collections](#collections)
8. [Classes & OOP](#classes--oop)
9. [Pattern Matching](#pattern-matching)
10. [Enums](#enums)
11. [Pipeline Operator](#pipeline-operator)
12. [Modules](#modules)
13. [Built-in Functions](#built-in-functions)
14. [DateTime & Duration](#datetime--duration)
15. [Linting](#linting)

---

## Getting Started

### What is Soli?

Soli is a modern, statically-typed programming language designed for clarity and expressiveness. It combines object-oriented programming with functional concepts like the pipeline operator, making it ideal for web development and data processing.

### Hello World

```soli
# The classic first program
print("Hello, World!");
```

### Your First Soli Program

```soli
# A simple program that calculates and displays results
def calculate_area(radius: Float) -> Float
  3.14159 * radius * radius
end

radius = 5.0;
area = calculate_area(radius);
print("The area of a circle with radius " + str(radius) + " is " + str(area));
# Output: The area of a circle with radius 5.0 is 78.53975
```

### Running Soli Code

```bash
# Run a single file
soli run hello.sl

# Run with hot reload (development)
soli serve

# Run tests
soli test

# Lint code for style issues
soli lint              # all .sl files in current dir (recursive)
soli lint src/         # lint a directory
soli lint app/main.sl  # lint a single file
```

---

## Variables & Types

### Variable Declaration

Variables are declared using the `let` keyword. Soli uses block scoping.

```soli
# Basic variable declarations
name = "Alice";           # String
age = 30;                 # Int
temperature = 98.6;       # Float
is_active = true;         # Bool
nothing = null;           # Null
```

### Type Annotations

You can explicitly specify types for better documentation and type safety.

```soli
# Explicit type annotations
let name: String = "Alice";
let age: Int = 30;
let temperature: Float = 98.6;
let is_active: Bool = true;
let scores: Int[] = [95, 87, 92];
let user: Hash = {"name": "Alice", "age": 30};
```

### Primitive Types

Soli provides five primitive types:

```soli
# Int - 64-bit signed integer
count = 42;
negative = -100;
large = 9_000_000;  # Underscores for readability

# Float - 64-bit floating-point
pi = 3.14159;
small = 0.001;
scientific = 2.5e10;  # 25000000000.0

# String - UTF-8 text
greeting = "Hello, World!";
multiline = "Line 1\nLine 2\tTabbed";
raw = r"Path: C:\Users\name";  # Raw string (no escape processing)

# Multiline strings
poem = """The fog comes
on little cat feet.""";

story = [[Once upon
a time in
the wild west.]];

# Command substitution - execute shell commands
files = `ls *.sl`;        # Returns Future<{stdout, stderr, exit_code}>
output = files.stdout;     # Auto-resolves when accessed
code = files.exit_code;    # Exit code (0 = success)

# Bool - Boolean values
is_valid = true;
is_complete = false;

# Null - Absence of value
missing = null;
```

### Type Inference

Soli automatically infers types when not explicitly specified:

```soli
# Type inference examples
x = 5;              # Inferred as Int
y = 3.14;           # Inferred as Float
z = "hello";        # Inferred as String
flag = true;        # Inferred as Bool
nums = [1, 2, 3];   # Inferred as Int[]
person = {"name": "Alice"};  # Inferred as Hash

# You can always add annotations even with inference
id = 123;  # Int - inferred
let user_id: Int = 123;  # Explicit annotation, still Int
```

### Constants

Use `const` for values that should never change:

```soli
const MAX_CONNECTIONS = 100;
const DEFAULT_TIMEOUT = 30;
const PI = 3.14159265359;

# const values cannot be reassigned
# MAX_CONNECTIONS = 200;  # This would cause an error
```

### Scope

Variables in Soli are block-scoped:

```soli
x = 1;

if true
  y = 2;      # y is only visible in this block
  x = 3;      # This shadows the outer x
  print(x);       # Output: 3 (inner x)
end

print(x);           # Output: 1 (outer x)

# Loop scope
for i in range(0, 3)
  print(i);       # i is visible only within the loop
end
# print(i);         # Error: i is not defined
```

### Shadowing

Variable shadowing allows inner blocks to redefine outer variables:

```soli
message = "outer";

if true
  message = "inner";
  print(message);  # "inner"
end

print(message);      # "outer"

# Common use case: transforming data
data = get_data();
if data != null
  data = process(data);  # Transform while keeping same name
  print(data);
end
```

---

## Operators

### Arithmetic Operators

```soli
a = 10;
b = 3;

# Basic arithmetic
print(a + b);   # 13  (addition)
print(a - b);   # 7   (subtraction)
print(a * b);   # 30  (multiplication)
print(a / b);   # 3.3333333333333335  (division - always float!)
print(a % b);   # 1   (modulo)

# Integer division requires special handling
int_result = int(a / b);  # 3
remainder = a % b;        # 1

# Compound assignment
counter = 0;
counter = counter + 1;  # 1
counter += 1;           # 2 (shorthand)
counter *= 2;           # 4
# Also: -=, /=, %=
```

### Conditional Assignment Operators

Soli supports three conditional assignment operators that only assign when the
target meets a condition. They follow the short-circuit semantics of their
matching binary operators.

```soli
# ||=  Assign only if the current value is falsy (null or false)
name = null;
name ||= "Anonymous";   # name is now "Anonymous"

name = "Alice";
name ||= "Anonymous";   # name stays "Alice"

# ??=  Assign only if the current value is null
port = null;
port ??= 8080;          # port is now 8080

flag = false;
flag ??= true;          # flag stays false (only null triggers ??=)

# &&=  Assign only if the current value is truthy
user = {"name": "Alice"};
user &&= load_full_profile(user);  # only runs when user is truthy

# Common idiom: lazy default for hash keys
cache = {};
cache["key"] ||= expensive_lookup();   # compute once, reuse on repeat
```

| Operator | Equivalent to            | Use when                                    |
|----------|--------------------------|---------------------------------------------|
| `a ||= b` | `a = a || b`            | You want a fallback for falsy values        |
| `a &&= b` | `a = a && b`            | You want to update only when already set    |
| `a ??= b` | `a = a ?? b`            | You want a default only for `null` (keeps `false`/`0`) |

The right-hand side is **not evaluated** when the condition fails, so it's
safe to use expensive expressions on the right.

### Comparison Operators

```soli
x = 5;
y = 10;

# Equality
print(x == y);   # false
print(x != y);   # true

# Ordering
print(x < y);    # true
print(x <= y);   # true
print(x > y);    # false
print(x >= y);   # false

# String comparison
print("apple" == "apple");  # true
print("apple" < "banana");  # true (lexicographic)

# Array comparison (element-wise)
print([1, 2, 3] == [1, 2, 3]);  # true
print([1, 2] < [1, 2, 3]);      # true (shorter is "less")
```

### Logical Operators

```soli
age = 25;
has_license = true;
is_weekend = false;

# AND - both conditions must be true
if age >= 18 && has_license
  print("Can drive");
end

# OR - at least one condition must be true
if is_weekend || is_holiday
  print("Day off!");
end

# NOT - negates the condition
if !is_raining
  print("No umbrella needed");
end

# Chained conditions
score = 85;
if score >= 90 && attendance >= 80
  print("Grade: A");
elsif score >= 80 || extra_credit > 10
  print("Grade: B");
end
```

### String Operations

```soli
# Concatenation
greeting = "Hello, " + "World!";    # "Hello, World!"
message = "Value: " + 42;           # "Value: 42" (auto-conversion)
path = "/home/" + "user";           # "/home/user"

# String methods
text = "  Hello, World!  ";
print(text.trim());        # "Hello, World!" (removes whitespace)
print(text.upper());       # "  HELLO, WORLD!  "
print(text.lower());       # "  hello, world!  "
print(text.len());         # 18

# Substring operations
s = "Hello, World!";
print(s.sub(0, 5));        # "Hello" (from index 0, length 5)
print(s.find("World"));    # 7 (index of first occurrence)
print(s.contains("Hello"));  # true
print(s.starts_with("Hell"));  # true
print(s.ends_with("!"));      # true
print(s.casecmp?("hello, world!"));  # true (case-insensitive equality)

# String transformation
snake_case = "HelloWorld".snake_case();  # "hello_world"
camel_lower = "hello_world".camelize();      # "helloWorld" (lower-camel)
camel_upper = "hello_world".camelize(true);  # "HelloWorld" (PascalCase)
slug = "Café & Restaurant".slugify;    # "cafe-restaurant" (lowercases, folds accents, hyphenates)
entities = "été·".html_entities();     # "&#233;t&#233;&#183;" (non-ASCII -> numeric HTML entities, ASCII untouched)

# String successor
next_id = "a".succ;    # "b" (increments with carry, wraps z->aa, 9->10)
next_id = "9".succ;    # "10"
next_id = "z".next;    # "aa" (.next is alias for .succ)

# String interpolation
name = "World";
greeting = "Hello #{name}!";           # "Hello World!"
a = 2;
b = 3;
result = "Sum is #{a + b}";             # "Sum is 5"
first = "John";
last = "Doe";
full = "#{first} #{last}";              # "John Doe"
text = "hello";
upper = "Upper: #{text.upper()}";        # "Upper: HELLO"
items = ["Alice", "Bob"];
first_item = "First: #{items[0]}";       # "First: Alice"
person = {"name": "Charlie"};
person_name = "Name: #{person["name"]}"; # "Name: Charlie"
```

### Type Coercion

```soli
# Int to Float in mixed arithmetic
let result = 5 + 3.0;      # result is Float: 8.0
let price = 10 / 3;        # 3.3333333333333335 (float division)

# Explicit conversion
let str_num = "42";
let num = int(str_num);    # 42
let f = float("3.14");     # 3.14

let n = 123;
let s = str(n);            # "123" (any type to string)
```

### Null-Safe Operations

```soli
user = {"name": "Alice", "email": null};

# Traditional null check
email = user["email"];
if email == null
  email = "unknown";
end

# Null coalescing operator
display_email = user["email"] ?? "unknown";

# Chaining with null values
city = user["address"]["city"] ?? "Unknown City";
# If any key in the chain is null/missing, returns "Unknown City"

# Safe navigation operator (&.)
# Access properties or call methods on values that might be null
user = get_user()  # might return null

name = user&.name              # null if user is null, otherwise user.name
city = user&.address&.city     # chain for nested access
greeting = user&.greet()       # null if user is null, otherwise calls greet()
display = user&.name ?? "Anon" # combine with ?? for defaults
```

---

## Control Flow

### If/Else Statements

```soli
age = 18;

# Simple if
if age >= 18
  print("Adult");
end

# If-else
score = 75;
if score >= 60
  print("Pass");
else
  print("Fail");
end

# Else-if chain
grade = 85;
let letter;
if grade >= 90
  letter = "A";
elsif grade >= 80
  letter = "B";
elsif grade >= 70
  letter = "C";
elsif grade >= 60
  letter = "D";
else
  letter = "F";
end
print(letter);  # "B"

# Nested conditions
is_weekend = true;
is_holiday = false;
has_plans = true;

if is_weekend
  if is_holiday
    print("Holiday vacation!");
  elsif has_plans
    print("Busy with plans");
  else
    print("Relaxing at home");
  end
end
```

### The `then` Keyword

The `then` keyword is an **optional** separator between the condition and the body of `if` and `elsif` statements. It improves readability, especially for single-line conditionals.

```soli
# Multi-line with then
if age >= 18 then
  print("Adult")
end

# Single-line with then
if user != null then print("Welcome") end

# Works with elsif
if x > 100 then
  print("big")
elsif x > 10 then
  print("medium")
else
  print("small")
end

# Parentheses and then can be combined
if (score >= 90) then
  grade = "A"
end
```

> **Note:** `then` is purely syntactic sugar. Both `if condition then ... end` and `if condition ... end` are equivalent. Parentheses around the condition are also optional.

### While Loops

```soli
# Basic while loop
i = 0;
while i < 5
  print("Count: " + str(i));
  i = i + 1;
end
# Output:
# Count: 0
# Count: 1
# Count: 2
# Count: 3
# Count: 4

# While with complex condition
data = [1, 2, 3, 4, 5];
sum = 0;
idx = 0;
while idx < len(data) && data[idx] < 4
  sum = sum + data[idx];
  idx = idx + 1;
end
print("Sum: " + str(sum));  # 6 (1+2+3)

# Do-while equivalent (using break)
count = 0;
loop
  count = count + 1;
  print("Iteration: " + str(count));
  if count >= 3
    break;
  end
end
```

### For Loops

```soli
# Iterate over array
let fruits = ["apple", "banana", "cherry"];
for fruit in fruits
  print(fruit);
end
# Output: apple, banana, cherry

# Iterate with index — element first, then index variable
for fruit, i in fruits
  print(str(i) + ": " + fruit);
end
# Output:
# 0: apple
# 1: banana
# 2: cherry

# Iterate with range
for i in range(0, 5)
  print(i);  # 0, 1, 2, 3, 4
end

# Range with step
for i in range(0, 10, 2)
  print(i);  # 0, 2, 4, 6, 8
end

# Nested loops
for i in range(1, 4)
  for j in range(1, 4)
    print(str(i) + " x " + str(j) + " = " + str(i * j));
  end
end
# Output: Multiplication table 1x1 through 3x3

# Iterate backwards
let arr = [1, 2, 3, 4, 5];
for i in range(len(arr) - 1, -1, -1)
  print(arr[i]);  # 5, 4, 3, 2, 1
end

# Break and next
let numbers = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
let sum = 0;
for n in numbers
  if n % 2 == 0
    next
  end
  if n > 7
    break;  # Stop at first number > 7
  end
  sum = sum + n;
end
print("Sum of odd numbers < 7: " + str(sum));  # 1+3+5 = 9
```

### Postfix Conditionals

Ruby-style postfix conditionals for concise single statements:

```soli
x = 10;
print("big") if (x > 5);

y = 3;
print("small") unless (y > 5);

# More examples
status = "active";
print("Welcome!") if (status == "active");
print("Account locked") unless (status != "banned");

items = [];
print("Empty") if (len(items) == 0);
```

### Ternary Operator

```soli
# Basic ternary
x = 10;
size = x > 5 ? "large" : "small";
print(size);  # "large"

# Nested ternary
grade = 85;
letter = grade >= 90 ? "A"
      : grade >= 80 ? "B"
      : grade >= 70 ? "C"
      : grade >= 60 ? "D"
      : "F";
print(letter);  # "B"

# In assignments
max_val = a > b ? a : b;
status = is_valid ? "valid" : "invalid";
```

### Match Expression

Soli's powerful pattern matching (covered in detail in the Pattern Matching section):

```soli
# Basic match
x = 42;
result = match x {
  42 => "the answer",
  _ => "something else",
};
print(result);  # "the answer"

# Type matching
let value: Any = "hello";
match value {
  s: String => "String: " + s,
  n: Int => "Int: " + str(n),
  _ => "Unknown",
};
```

### Truthiness

Soli follows common truthiness rules:

```soli
# Falsy values
falsy_values = [false, null, 0, 0.0, "", []];

# Truthy values (everything else)
if "hello"
  print("Non-empty string is truthy");
end

if [1, 2, 3]
  print("Non-empty array is truthy");
end

if 42
  print("Non-zero number is truthy");
end

# Practical examples
config = get_config();
if config
  print("Config loaded: " + str(config));
end

items = get_items() ?? [];
if len(items) > 0
  print("Found " + str(len(items)) + " items");
end
```

---

## Error Handling

### Try / Catch / Finally

Soli provides `try`/`catch`/`finally` for exception handling, using `end`-delimited blocks (just like `if`, `while`, and `for`). Ruby-style aliases are supported: `begin` for `try`, `rescue` for `catch`, and `ensure` for `finally`. The aliases are interchangeable with the canonical keywords; `soli fmt` normalizes them to `try`/`catch`/`finally`.

```soli
# Basic try/catch
try
  result = 10 / 0;
catch e
  print("Error: " + str(e));
end

# With finally (always runs)
try
  data = read_file("config.sl");
  print(data);
catch e
  print("Failed to read file: " + str(e));
finally
  print("Cleanup done");
end

# Try/finally without catch
try
  process_data();
finally
  close_connection();
end

# Ruby-style aliases: `begin` for `try`, `rescue` for `catch`, `ensure` for `finally`
begin
  risky_operation();
rescue e
  print("Error: " + str(e));
ensure
  print("Cleanup done");
end
```

A `rescue` that opens a new line inside a `begin`/`try` body is always a catch
clause. The postfix `rescue` modifier (`expr rescue fallback`) is unaffected — it
still works inline, including inside a `begin` body:

```soli
begin
  value = (10 / 0) rescue 99   # postfix modifier: value becomes 99
rescue e
  value = -1
end
```

### Catch Variable Syntax

The catch variable can be written with or without parentheses:

```soli
# Both are equivalent
catch e
  print(e);
end

catch (e)
  print(e);
end
```

### Throwing Exceptions

Use `throw` to raise an exception:

```soli
def divide(a: Int, b: Int) -> Int
  if b == 0
    throw "Division by zero";
  end
  a / b
end

try
  result = divide(10, 0);
catch e
  print("Caught: " + str(e));  # "Caught: Division by zero"
end
```

### Typed Catch

Catch specific error types by class name. Multiple `catch` blocks are tried in order:

```soli
class NotFoundError
  message: String
  new(msg: String)
    this.message = msg
  end
end

class ValidationError
  message: String
  new(msg: String)
    this.message = msg
  end
end

try
  throw new NotFoundError("User not found")
catch NotFoundError e
  print("404: " + e.message)
catch ValidationError e
  print("Invalid: " + e.message)
catch e
  print("Unknown: " + str(e))
end
```

**Subclass matching:** A typed catch walks the inheritance chain, so `catch AppError` also catches subclasses of `AppError`:

```soli
class AppError
  message: String
  new(msg: String)
    this.message = msg
  end
end

class NotFoundError < AppError
  new(msg: String)
    super(msg)
  end
end

try
  throw new NotFoundError("missing")
catch AppError e
  print("App error: " + e.message)  # Catches NotFoundError too
end
```

**Rules:**
- Typed catches only match class instances (strings, ints, etc. won't match a typed catch)
- Put more specific types first — catches are tried in order
- A bare `catch e` catches everything (catch-all)
- If no typed catch matches and there is no catch-all, the exception re-throws to the outer scope

### Brace Syntax

Try/catch also supports brace-delimited blocks:

```soli
try {
  result = risky_operation();
} catch (e) {
  print("Error: " + str(e));
} finally {
  cleanup();
}
```

---

## Functions

### Function Declaration

Functions are declared with the `fn` keyword. You can also use `def` as an alias (Ruby-style).

```soli
# No parameters — parentheses are optional
def say_hello
  print("Hello!");
end

# Equivalent with explicit empty parentheses
def say_hello()
  print("Hello!");
end

# `def` works exactly like `fn`
def greet(name: String)
  print("Hello, " + name + "!");
end

# With return value
def add(a: Int, b: Int) -> Int
  a + b
end

# Void function (explicit)
def log_message(msg: String)
  print("[LOG] " + msg);
end

# With type annotations on return
def multiply(a: Float, b: Float) -> Float
  a * b
end
```

### Function Examples

```soli
# Calculate factorial
def factorial(n: Int) -> Int
  if n <= 1
    return 1;
  end
  n * factorial(n - 1)
end

print(factorial(5));  # 120

# Calculate Fibonacci
def fibonacci(n: Int) -> Int
  return n if n <= 1
  fibonacci(n - 1) + fibonacci(n - 2)
end

print(fibonacci(10));  # 55

# Check if a number is prime
def is_prime(n: Int) -> Bool
  if n < 2
    return false;
  end
  if n == 2
    return true;
  end
  if n % 2 == 0
    return false;
  end
  i = 3;
  while i * i <= n
    if n % i == 0
      return false;
    end
    i = i + 2;
  end
  true
end

# Find maximum in array
def find_max(arr: Int[]) -> Int
  if len(arr) == 0
    return 0;  # or panic for empty array
  end
  max = arr[0];
  for i in range(1, len(arr))
    if arr[i] > max
      max = arr[i];
    end
  end
  max
end
```

### Early Returns

```soli
def process_user(user: Hash) -> Hash
  # Validate required fields
  if !has_key(user, "name")
    return {"error": "Name is required"};
  end
  if !has_key(user, "email")
    return {"error": "Email is required"};
  end

  # Validate email format
  email = user["email"];
  if !email.contains("@")
    return {"error": "Invalid email format"};
  end

  # Process user data
  processed = user;
  processed["status"] = "active";
  processed["created_at"] = DateTime.utc().to_iso();

  processed
end
```

### Higher-Order Functions

Functions can accept other functions as parameters and return functions:

```soli
# Function as parameter
def apply(x: Int, f: (Int) -> Int) -> Int
  f(x)
end

def double(x: Int) -> Int
  x * 2
end

def square(x: Int) -> Int
  x * x
end

result = apply(5, double);   # 10
squared = apply(5, square);  # 25

# Passing anonymous functions
def transform_array(arr: Int[], transformer: (Int) -> Int) -> Int[]
  result = [];
  for item in arr
    push(result, transformer(item));
  end
  result
end

numbers = [1, 2, 3, 4, 5];
doubled = transform_array(numbers, fn(x) x * 2);  # [2, 4, 6, 8, 10]

# Function that returns a function
def multiplier(factor: Int) -> (Int) -> Int
  def closure(x: Int) -> Int
    x * factor
  end
  closure
end

times_two = multiplier(2);
print(times_two(5));   # 10
print(times_two(10));  # 20

times_three = multiplier(3);
print(times_three(5));  # 15
```

### Closures

```soli
# Counter using closure
def make_counter() -> () -> Int
  let count = 0;
  def counter() -> Int
    count = count + 1;
    count
  end
  counter
end

let counter1 = make_counter();
let counter2 = make_counter();

print(counter1());  # 1
print(counter1());  # 2
print(counter1());  # 3

print(counter2());  # 1
print(counter2());  # 2

# Closure capturing variables
def make_greeter(greeting: String) -> (String) -> String
  def greet(name: String) -> String
    greeting + ", " + name + "!"
  end
  greet
end

let say_hello = make_greeter("Hello");
let say_hola = make_greeter("Hola");

print(say_hello("Alice"));  # "Hello, Alice!"
print(say_hola("Bob"));     # "Hola, Bob!"
```

### Default Parameters

```soli
def greet(name: String, greeting: String = "Hello") -> String
  greeting + ", " + name + "!"
end

print(greet("Alice"));              # "Hello, Alice!"
print(greet("Bob", "Hi"));          # "Hi, Bob!"
print(greet("Charlie", "Welcome")); # "Welcome, Charlie!"

# Optional parameters
def create_user(name: String, email: String = null, role: String = "user") -> Hash
  user = {"name": name, "role": role};
  if email != null
    user["email"] = email;
  end
  user
end

user1 = create_user("Alice");
user2 = create_user("Bob", "bob@example.com");
user3 = create_user("Charlie", "charlie@example.com", "admin");
```

### Named Parameters

You can call functions using named parameters with the colon syntax:

```soli
def configure(host: String = "localhost", port: Int = 8080, debug: Bool = false) -> Void
  print("Connecting to #{host}:#{port} with debug=#{debug}");
end

configure();                              # Using all defaults
configure(host: "example.com");           # Only specify host
configure(port: 3000, debug: true);       # Named parameters in any order
configure("example.com", port: 443);     # Mixed: positional then named
configure(host: "api.example.com", port: 443, debug: true);  # All named
```

#### Ruby-Style Calls Without Parentheses

You can also call methods on objects without parentheses, using Ruby-style syntax:

```soli
# With parentheses (standard)
user.update(name: "Bob", age: 30);
user.save();
puts("Hello world");

# Without parentheses (Ruby-style)
user.update name: "Bob", age: 30;
user.save;
puts "Hello world";
```

This works for:
- Method calls on objects with named arguments: `obj.method arg: value`
- Method calls on objects without arguments: `obj.method`
- Standalone function calls with named arguments: `fn_name arg: value`

#### Rules

1. Named arguments use the colon syntax: `parameter_name: value`
2. Named arguments must come after all positional arguments
3. Duplicate named arguments cause a runtime error
4. Unknown parameter names cause a runtime error
5. Default values are used for any parameters not provided

```soli
# Error: positional argument after named argument
configure(port: 3000, "example.com");  # Parser error

# Error: duplicate named argument
configure(port: 3000, port: 8080);     # Runtime error

# Error: unknown parameter name
configure(unknown: 123);               # Runtime error
```

#### Use Cases

Named parameters are useful when:

- A function has many parameters with defaults
- You want to skip optional parameters in the middle
- Code readability is important (named params are self-documenting)
- API calls where parameter order might change

```soli
def http_request(
  method: String = "GET",
  url: String,
  headers: Hash = {},
  body: Any = null,
  timeout: Int = 30
) { ... }

# Clear and readable - specify only what changes
http_request(
  method: "POST",
  url: "https://api.example.com/users",
  body: {"name": "Alice"}
);
```

### Variadic Functions

```soli
def sum(numbers: Int[]) -> Int
  total = 0;
  for n in numbers
    total = total + n;
  end
  total
end

print(sum([1, 2, 3, 4, 5]));  # 15

# Using spread operator
nums = [1, 2, 3];
print(sum(nums));             # 6
print(sum([...nums, 4, 5]));  # 15

# Variadic-like with array
def format_list(items: String[], separator: String = ", ", final_separator: String = "and") -> String
  len = len(items);
  if len == 0
    return "";
  end
  if len == 1
    return items[0];
  end
  if len == 2
    return items[0] + " " + final_separator + " " + items[1];
  end
  result = "";
  for i in range(0, len - 1)
    result = result + items[i] + separator;
  end
  result + final_separator + " " + items[len - 1]
end

print(format_list(["apple", "banana", "cherry"]));  # "apple, banana and cherry"
print(format_list(["one", "two"]));                 # "one and two"
```

### Universal Methods on Function Values

Functions are first-class values, and the universal predicates available on every other type work on them too. Useful in defensive view partials where a local might resolve to a function, a string, or be undefined.

```soli
f = fn(x) { x + 1 };

f.nil?       # false — a function value is never null
f.blank?     # false
f.present?   # true
f.class      # "Function"
f.inspect    # "<function>"
```

**Zero-arg caveat:** a zero-parameter function auto-invokes on bare access. `let g = fn() { 42 }; g.class` evaluates `g()` first, so `.class` sees the return value, not the function itself. Use a multi-arg function if you need to inspect the function value.

---

## Collections

### Arrays

#### Creating Arrays

```soli
# Basic array creation
numbers = [1, 2, 3, 4, 5];
names = ["Alice", "Bob", "Charlie"];
mixed = [1, "two", 3.0, true];

# Type-annotated arrays
let scores: Int[] = [95, 87, 92, 88, 90];
let words: String[] = [];  # Empty array

# Array from range
range_arr = range(1, 10);  # [1, 2, 3, 4, 5, 6, 7, 8, 9]
step_arr = range(0, 10, 2);  # [0, 2, 4, 6, 8]

# Initialize with default value
zeros = [];
for _ in range(0, 5)
  push(zeros, 0);
end  # [0, 0, 0, 0, 0]

# Percent literal arrays - quick string/symbol/number arrays (with decimals)
words = %w[foo bar baz];    # ["foo", "bar", "baz"]
keys = %i[get post put];    # [:get, :post, :put]
nums = %n[1 2.5 3.5D];     # [1, 2.5, 3.5]
empty_w = %w[];            # []
empty_i = %i[];            # []
empty_n = %n[];            # []
```

#### Percent Literal Arrays

Soli provides `%w[]`, `%i[]`, and `%n[]` as sugar syntax for creating arrays of strings, symbols, or numbers without quotes.

```soli
# %w[] - Array of strings
words = %w[demo test production];
words;  # ["demo", "test", "production"]

# %i[] - Array of symbols
methods = %i[get post put delete];
methods;  # [:get, :post, :put, :delete]

# %n[] - Array of numbers (integers, floats, and decimals with D suffix)
numbers = %n[1 2.5 3.5D 4];
numbers;  # [1, 2.5, 3.5, 4]

# Equivalent to regular arrays
%w[a b c] == ["a", "b", "c"]
%i[a b c] == [:a, :b, :c]
%n[1 2 3] == [1, 2, 3]
%n[1.5D 2.5D] == [1.5D, 2.5D]
```

Elements are separated by whitespace (spaces, tabs, or newlines). No commas are needed.

```soli
# Works with newlines for readability
const HTTP_METHODS = %i[
  get
  post
  put
  delete
];

# Number arrays useful for coordinates, indices, etc.
const COORDINATES = %n[0 0 10 10];
tags = %w[ruby javascript python elixir];
```

#### Array Access and Modification

```soli
fruits = ["apple", "banana", "cherry", "date"];

# Access elements
print(fruits[0]);   # "apple"
print(fruits[1]);   # "banana"
print(fruits[-1]);  # "date" (last element)
print(fruits[-2]);  # "cherry"

# Modify elements
fruits[0] = "apricot";
print(fruits[0]);  # "apricot"

# Out of bounds returns null
print(fruits[100]);  # null

# Slicing
def slice(arr: Array, start: Int, end: Int) -> Array {
  result = [];
  actual_end = end;
  if (end > len(arr)) {
    actual_end = len(arr);
  }
  for (i in range(start, actual_end)) {
    push(result, arr[i]);
  }
  result
}

print(slice(fruits, 1, 3));  # ["banana", "cherry"]
```

#### Array Methods

```soli
numbers = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];

# map - transform each element
doubled = numbers.map(fn(x) x * 2);
print(doubled);  # [2, 4, 6, 8, 10, 12, 14, 16, 18, 20]

# filter - keep elements matching condition
evens = numbers.filter(fn(x) x % 2 == 0);
print(evens);  # [2, 4, 6, 8, 10]

# each - iterate with side effects
numbers.each(fn(x) print(x));  # Prints each number

# reduce - accumulate to single value
sum = numbers.reduce(fn(acc, x) acc + x, 0);  # 55
product = numbers.reduce(fn(acc, x) acc * x, 1);  # 3628800

# find - first matching element
first_even = numbers.find(fn(x) x % 2 == 0);  # 2

# find_index - index of first matching element
idx = numbers.find_index(fn(x) x > 5);  # 5

# every - check if all elements match
all_positive = numbers.every(fn(x) x > 0);  # true

# some - check if any element matches
has_large = numbers.some(fn(x) x > 8);  # true

# dig - safe nested access (returns null on any miss, no errors)
data = [
  { "user": { "name": "Alice", "posts": [ { "title": "Hello" } ] } },
  { "user": { "name": "Bob" } }
]
print(data.dig(0, "user", "name"));           # "Alice"
print(data.dig(0, "user", "posts", 0, "title")); # "Hello"
print(data.dig(1, "user", "posts", 0));       # null (safe, no crash)
print([10, 20, 30].dig(-1));                  # 30 (negative index supported)

# pluck - extract one or more fields (very useful on arrays of hashes)
posts = [
  { "id": 1, "title": "Hello" },
  { "id": 2, "title": "World" }
]
print(posts.pluck("title"));           # ["Hello", "World"]          (single field → flat array)
print(posts.pluck("id", "title"));     # [[1, "Hello"], [2, "World"]] (multiple → array of arrays)

# pick - value(s) from the *first* element only (the “get one” companion to pluck)
print(posts.pick("title"));      # "Hello"
print(posts.pick("id", "title")); # [1, "Hello"]

# chunk - split into chunks
def chunk(arr: Array, size: Int) -> Array[]
  result = [];
  current = [];
  for item in arr
    push(current, item);
    if len(current) >= size
      push(result, current);
      current = [];
    end
  end
  if len(current) > 0
    push(result, current);
  end
  result
end

print(chunk(numbers, 3));  # [[1,2,3], [4,5,6], [7,8,9], [10]]

# Chaining methods
result = numbers
  .filter(fn(x) x % 2 == 0)   # [2, 4, 6, 8, 10]
  .map(fn(x) x * x)           # [4, 16, 36, 64, 100]
  .filter(fn(x) x < 50);      # [4, 16, 36]

print(result);  # [4, 16, 36]
```

#### Array Functions

```soli
let arr = [1, 2, 3, 4, 5];

# Length (function or method — .len, .length, .size are all aliases)
print(len(arr));      # 5
print(arr.len);       # 5
print(arr.length);    # 5
print(arr.size);      # 5

# Push - add element to end
push(arr, 6);
print(arr);  # [1, 2, 3, 4, 5, 6]

# Pop - remove and return last element
let last = pop(arr);
print(last);  # 6
print(arr);   # [1, 2, 3, 4, 5]

# Unshift - add element to beginning
let new_arr = unshift(arr, 0);
print(new_arr);  # [0, 1, 2, 3, 4, 5]

# Shift - remove and return first element
let first = shift(arr);
print(first);  # 0

# Insert at index
def insert(arr: Array, index: Int, value: Any) -> Array
  let result = [];
  for i in range(0, len(arr))
    if i == index
      push(result, value);
    end
    push(result, arr[i]);
  end
  if index >= len(arr)
    push(result, value);
  end
  result
end

# Reverse
let reversed = reverse(arr);
print(reversed);  # [5, 4, 3, 2, 1]

# Sort
let unsorted = [3, 1, 4, 1, 5, 9, 2, 6];
let sorted = sort(unsorted);
print(sorted);  # [1, 1, 2, 3, 4, 5, 6, 9]

# Sort by hash key
let people = [{"name": "Alice", "age": 30}, {"name": "Bob", "age": 25}, {"name": "Charlie", "age": 35}];
let by_age = people.sort_by("age");
print(by_age);  # Sorted by age: Bob, Alice, Charlie

# Sort by function
let by_name = people.sort_by(fn(p) p.get("name"));
print(by_name);  # Sorted by name: Alice, Bob, Charlie
```

### Hashes

#### Creating Hashes

```soli
# Basic hash creation
person = {
  "name": "Alice",
  "age": 30,
  "city": "New York"
};

# Alternative syntax with =>
scores = {"Alice" => 95, "Bob" => 87, "Charlie" => 92};

# Nested hashes
user = {
  "id": 1,
  "profile": {
    "name": "Alice",
    "email": "alice@example.com"
  },
  "settings": {
    "theme": "dark",
    "notifications": true
  }
};

# Empty hash
empty = {};

# Type-annotated hash
let config: Hash = {
  "host": "localhost",
  "port": 8080,
  "ssl": false
};
```

#### Hash Access and Modification

```soli
person = {"name": "Alice", "age": 30, "city": "Paris"};

# Access values
print(person["name"]);   # "Alice"
print(person["age"]);    # 30
print(person["email"]);  # null (key doesn't exist)

# Modify values
person["age"] = 31;
person["country"] = "France";  # Add new key

print(person);  # {name: Alice, age: 31, city: Paris, country: France}

# Delete key
deleted = delete(person, "city");
print(deleted);  # Paris
print(person);   # {name: Alice, age: 31, country: France}
```

#### Hash Methods

Hash methods support two function signatures for iteration:

```soli
# Two parameters: key and value (recommended)
h.map(fn(key, value) [key, value * 2])

# One parameter: [key, value] pair array
h.map(fn(pair) [pair[0], pair[1] * 2])
```

```soli
scores = {"Alice": 90, "Bob": 85, "Charlie": 95, "Diana": 88};

# map - transform entries
# Returns a new hash. The function MUST return [key, value] (exactly 2 elements).
# Returning fewer or more elements skips that entry.
curved = scores.map(fn(k, v) [k, v + 5]);
print(curved);  # {Alice: 95, Bob: 90, Charlie: 100, Diana: 93}

# Transform only values (keep key unchanged)
doubled = scores.map(fn(k, v) [k, v * 2]);

# Transform keys (prefix with "user_")
prefixed = scores.map(fn(k, v) ["user_" + k, v]);

# filter - keep entries matching condition
# Function receives (key, value) or [key, value] pair
# Returns boolean or truthy/falsy value
high_scores = scores.filter(fn(k, v) v >= 90);
print(high_scores);  # {Alice: 90, Charlie: 95}

# each - iterate with side effects
# Function receives (key, value) or [key, value] pair
# Returns original hash for chaining (return value is discarded)
scores.each(fn(k, v) print(k + ": " + str(v)));
```

**Additional iteration methods:**

- `.each_key(fn)` — iterate over keys only
- `.each_value(fn)` — iterate over values only
- `.keep_if(fn)` / `.delete_if(fn)` — filter variants of select/reject
- `.all?(fn)` / `.any?(fn)` — predicate tests on all/any entries

**Important: map return value**

Hash `.map()` expects your function to return exactly `[key, value]` with 2 elements:

```soli
h = {"a": 1, "b": 2};

# ✓ Returns [key, value] - works correctly
h.map(fn(k, v) [k, v * 10]);  # {a: 10, b: 20}

# ✗ Returns [value] only - entry is skipped (not 2 elements!)
h.map(fn(k, v) [v * 10]);      # {} (empty!)

# ✗ Returns single value - entry is skipped
h.map(fn(k, v) v * 10);         # {} (empty!)
```

**Getting transformed values as an array**

If you only need transformed values (not a new hash):

```soli
h = {"a": 1, "b": 2};

# Get values first, then map to array
doubled = h.values |> map(fn(v) v * 10);
print(doubled);  # [10, 20]
```

**Lookup and conversion methods:**

- `.shift` — remove and return first [key, value] pair (mutates)
- `.flatten` — convert to array of [key, value] sub-arrays
- `.values_at(*keys)` — array of values for given keys (null for missing)
- `.fetch_values(*keys)` — array of values (raises if any missing)
- `.key(value)` — inverse lookup: first key for a given value
- `.has_value?(value)` / `.value?(value)` — check if value exists
- `.assoc(key)` — returns [key, value] pair or null
- `.rassoc(value)` — returns [key, value] for matching value or null
- `.to_h` — returns self (identity for hashes)
- `.update(other)` — alias for merge

#### Hash Functions

```soli
person = {"name": "Alice", "age": 30, "city": "Paris", "country": "France"};

# Get length (also available as .len, .length, .size methods)
print(len(person));    # 4
print(person.len);     # 4

# Get all keys
keys_list = keys(person);
print(keys_list);  # [name, age, city, country]

# Get all values
values_list = values(person);
print(values_list);  # [Alice, 30, Paris, France]

# Check if key exists
print(has_key(person, "name"));      # true
print(has_key(person, "email"));     # false

# Get entries as [key, value] pairs
entries_list = entries(person);
print(entries_list);  # [[name, Alice], [age, 30], [city, Paris], [country, France]]

# Merge hashes
defaults = {"age": 0, "country": "Unknown", "active": true};
merged = person.merge(defaults);
print(merged);  # {name: Alice, age: 30, city: Paris, country: France, active: true}

# Clear hash
clear(person);
print(person);  # {}
```

#### Iterating Over Hashes

```soli
prices = {"apple": 1.50, "banana": 0.75, "orange": 2.00, "grape": 3.00};

# Iterate entries
for pair in entries(prices)
  item = pair[0];
  price = pair[1];
  print(item + " costs $" + str(price));
end

# Iterate keys
for item in keys(prices)
  print(item + ": " + str(prices[item]));
end

# Iterate values and calculate total
total = 0;
for price in values(prices)
  total = total + price;
end
print("Total: $" + str(total));  # Total: $7.25

# Filter and transform
expensive = prices
  .filter(fn(k, v) v > 1.00)
  .map(fn(k, v) [k, v * 1.1]);  # 10% tax

print(expensive);  # {apple: 1.65, orange: 2.2, grape: 3.3}
```

### Common Collection Patterns

```soli
# Slicing
def slice(arr: Array, start: Int, end: Int) -> Array
  result = [];
  actual_end = end;
  if end > len(arr)
    actual_end = len(arr);
  end
  for i in range(start, actual_end)
    push(result, arr[i]);
  end
  result
end

print(slice(fruits, 1, 3));  # ["banana", "cherry"]
```

---

## Classes & OOP

### Basic Class Definition

```soli
class Person
  # Instance fields
  name: String;
  age: Int;
  email: String;

  # Constructor
  new(name: String, age: Int, email: String = null)
    this.name = name;
    this.age = age;
    this.email = email ?? "";
  end

  # Instance methods
  def greet() -> String
    "Hello, I'm " + this.name
  end

  def introduce() -> String
    intro = "Hi, I'm " + this.name + " and I'm " + str(this.age) + " years old";
    if this.email != ""
      intro = intro + ". You can reach me at " + this.email;
    end
    intro
  end

  def have_birthday()
    this.age = this.age + 1;
  end
end

# Creating instances
alice = new Person("Alice", 30);
bob = new Person("Bob", 25, "bob@example.com");

# Using instances
print(alice.greet());      # "Hello, I'm Alice"
print(bob.introduce());    # "Hi, I'm Bob and I'm 25 years old. You can reach me at bob@example.com"

alice.have_birthday();
print(alice.age);          # 31
```

#### `this` and `self`

`self` is an alias for `this` — they refer to the same instance and are interchangeable everywhere (instance methods, constructors, `instance_eval` blocks). Pick whichever reads better; Ruby refugees can keep typing `self`.

```soli
class User
  new(name)
    self.name = name        # same as this.name = name
  end

  def say_hello
    println("Hello, " + self.name)
  end
end
```

### The `@` Sigil — Shorthand for `this.`

Inside any instance method, `@name` is sugar for `this.name`. Ruby developers will feel at home, and it drops the noise in constructor/field-heavy code.

```soli
class Counter
  n: Int;

  new()
    @n = 0;            # same as this.n = 0
  end

  def bump()
    @n += 1;           # read + write via sugar
  end

  def double_it()
    @n = @n * 2;
  end

  def value() -> Int
    @n                 # bare read, same as this.n
  end
end
```

**What works:**

- Reads: `@foo`
- Writes: `@foo = x`
- Compound assignment: `@foo += 1`, `@foo *= 2`, etc.
- Method calls: `@greet()` (calls `this.greet()`)
- Chained access: `@inner.label`, `@items[0]`
- Postfix ops: `@foo++`, `@foo--`
- Inheritance: `@foo` resolves to fields set by a parent class's constructor

**What doesn't:**

- `@@foo` (Ruby-style class variables) — rejected at parse time. Use a `static` field instead.
- `@foo` outside a class method — fails the same way a literal `this.foo` would, because `this` isn't in scope.

### Controllers: Instance Fields Auto-Exposed to Views

In MVC controllers, fields set on the controller (via `@foo = ...` or `this.foo = ...`) inside an action are automatically exposed as view locals in the template that action renders — no data hash needed.

```soli
class PostsController < Controller
  def show
    @post = Post.find(params["id"]);
    @related = Post.where({"category_id": @post.category_id}).limit(5);
    render("posts/show")    # no data hash — view sees `post` and `related`
  end
end
```

```erb
<%# app/views/posts/show.html.erb %>
<h1><%= post.title %></h1>
<%= post.body %>

<h2>Related</h2>
<% for p in related %>
  <%= link_to(p.title, "/posts/" + str(p.id)) %>
<% end %>
```

- Explicit `render("view", {...})` data **always wins** over the auto-exposed fields.
- The framework-injected fields `req`, `params`, `session`, and `headers` are never re-exposed this way — those flow through their usual channels.
- Auto-exposure is scoped to the action currently running. No cross-action, cross-controller, or cross-request leakage.
- Partials (`render_partial`, `partial`) are unaffected — pass data to them explicitly. Inside the partial, read the data back via bare identifiers or the `locals` hash (`locals["class"]`) for keys whose names collide with reserved words or builtins. See [Views → The `locals` hash](./views.md#the-locals-hash) for the full semantics.

### Constructors and Factory Methods

```soli
class Rectangle
  width: Float;
  height: Float;

  new(width: Float, height: Float)
    this.width = width;
    this.height = height;
  end

  def area() -> Float
    this.width * this.height
  end

  def perimeter() -> Float
    2 * (this.width + this.height)
  end

  # Static factory method
  static def square(side: Float) -> Rectangle
    new Rectangle(side, side)
  end

  # Another factory method
  static def from_area(area: Float, aspect_ratio: Float = 1.0) -> Rectangle
    width = sqrt(area / aspect_ratio);
    height = width * aspect_ratio;
    new Rectangle(width, height)
  end
end

rect = new Rectangle(10.0, 5.0);
print(rect.area());  # 50.0

square = Rectangle.square(7.0);
print(square.area());  # 49.0

from_area = Rectangle.from_area(24.0, 2.0);  # 2:1 aspect ratio
print(from_area.width);   # ~3.464
print(from_area.height);  # ~6.928
```

### Inheritance

> **Note:** You can use `<` as an alias for `extends` (e.g., `class Dog < Animal`).

```soli
# Base class
class Animal
  name: String;
  age: Int;

  new(name: String, age: Int)
    this.name = name;
    this.age = age;
  end

  def speak() -> String
    this.name + " makes a sound"
  end

  def get_info() -> String
    this.name + " is " + str(this.age) + " years old"
  end
end

# Subclass
class Dog < Animal
  breed: String;

  new(name: String, age: Int, breed: String)
    # Call parent constructor
    super(name, age);
    this.breed = breed;
  end

  # Override method
  def speak() -> String
    this.name + " barks!"
  end

  # Subclass-specific method
  def fetch() -> String
    this.name + " fetches the ball!"
  end
end

# Another subclass
class Cat < Animal
  new(name: String, age: Int)
    super(name, age);
  end

  def speak() -> String
    this.name + " meows!"
  end

  def purr() -> String
    this.name + " purrs contentedly"
  end
end

# Using inheritance
dog = new Dog("Buddy", 3, "Golden Retriever");
print(dog.speak());        # "Buddy barks!"
print(dog.get_info());     # "Buddy is 3 years old"
print(dog.fetch());        # "Buddy fetches the ball!"
print(dog.breed);          # "Golden Retriever"

cat = new Cat("Whiskers", 5);
print(cat.speak());        # "Whiskers meows!"
print(cat.purr());         # "Whiskers purrs contentedly"

# Polymorphism
animals = [
  new Dog("Rex", 2, "German Shepherd"),
  new Cat("Mittens", 4),
  new Dog("Spot", 1, "Beagle")
];

for animal in animals
  print(animal.speak());  # Each calls the appropriate speak() method
end
```

### Multi-level Inheritance

Classes can extend other user-defined classes, forming deep inheritance chains. Methods, fields, and constructors are inherited through the full chain. Use `super` to call the parent's version at each level.

```soli
class Controller
  def action() -> String
    "base"
  end
end

class BaseController < Controller
  def before() -> String
    "authenticated"
  end
end

class HomeController < BaseController
  def action() -> String
    super.action() + " -> home"
  end
end

c = new HomeController()
print(c.action())   # "base -> home"
print(c.before())   # "authenticated" (inherited from BaseController)
```

### Interfaces

```soli
# Define an interface
interface Drawable {
  def draw() -> String;
  def get_color() -> String;
}

# Another interface
interface Resizable {
  def resize(width: Float, height: Float);
  def get_dimensions() -> {width: Float, height: Float};
}

# Class implementing multiple interfaces
class Circle implements Drawable, Resizable {
  radius: Float;
  color: String;

  new(radius: Float, color: String) {
    this.radius = radius;
    this.color = color;
  }

  def draw() -> String {
    "Circle with radius " + str(this.radius) + " and color " + this.color
  }

  def get_color() -> String {
    this.color
  }

  def resize(width: Float, height: Float) {
    this.radius = width / 2;
  }

  def get_dimensions() -> {width: Float, height: Float} {
    {"width": this.radius * 2, "height": this.radius * 2}
  }
}

class Rectangle implements Drawable, Resizable {
  width: Float;
  height: Float;
  color: String;

  new(width: Float, height: Float, color: String) {
    this.width = width;
    this.height = height;
    this.color = color;
  }

  def draw() -> String {
    "Rectangle " + str(this.width) + "x" + str(this.height) + " in " + this.color
  }

  def get_color() -> String {
    this.color
  }

  def resize(width: Float, height: Float) {
    this.width = width;
    this.height = height;
  }

  def get_dimensions() -> {width: Float, height: Float} {
    {"width": this.width, "height": this.height}
  }
}

# Using interfaces
shapes = [
  new Circle(5.0, "red"),
  new Rectangle(10.0, 6.0, "blue")
];

for shape in shapes {
  print(shape.draw());
}
```

#### `~` shorthand for `implements`

`~` is an alias for `implements` in class headers, in the same spirit as `<` aliases `extends`. It composes with both forms — use whichever reads best:

```soli
# Shorthand on its own
class Circle ~ Drawable, Resizable
  # ...
end

# `extends` + `~`
class Dog < Animal ~ Greetable
  def greet() "woof" end
end

# `<` + `~`
class Cat < Animal ~ Greetable
  def greet() "meow" end
end
```

The full `implements` keyword still works; pick whichever matches the style of the surrounding code.

### Visibility Modifiers

```soli
class BankAccount
  # Fields with visibility
  public account_number: String;
  private balance: Float;
  protected account_holder: String;
  public status: String;

  new(account_number: String, initial_deposit: Float)
    this.account_number = account_number;
    this.balance = initial_deposit;
    this.account_holder = "";
    this.status = "active";
  end

  # Public method
  public def deposit(amount: Float) -> Bool
    if this.validate_amount(amount)
      this.balance = this.balance + amount;
      this.log_transaction("Deposit", amount);
      return true;
    end
    false
  end

  # Public method
  public def withdraw(amount: Float) -> Bool
    if this.validate_amount(amount) && this.has_sufficient_funds(amount)
      this.balance = this.balance - amount;
      this.log_transaction("Withdrawal", -amount);
      return true;
    end
    false
  end

  # Public getter
  public def get_balance() -> Float
    this.balance
  end

  # Private method - internal helper
  private def validate_amount(amount: Float) -> Bool
    amount > 0
  end

  # Private method
  private def has_sufficient_funds(amount: Float) -> Bool
    this.balance >= amount
  end

  # Private method
  private def log_transaction(type: String, amount: Float)
    # Internal logging logic
  end

  # Protected method - for subclasses
  protected def update_status(new_status: String)
    this.status = new_status;
  end
end

# Using the class
account = new BankAccount("123456789", 1000.0);

account.deposit(500.0);           # Works - public method
print(account.get_balance());     # 1500.0
print(account.account_number);    # "123456789" - public field

# account.balance;              # Error - private field
# account.validate_amount(100); # Error - private method
```

### Static Members

```soli
class MathUtils
  # Static constants
  static PI: Float = 3.14159265359;
  static E: Float = 2.71828182846;
  static GOLDEN_RATIO: Float = 1.61803398875;

  # Static field
  static calculation_count: Int = 0;

  # Static methods
  static def square(x: Float) -> Float
    MathUtils.calculation_count = MathUtils.calculation_count + 1;
    x * x
  end

  static def cube(x: Float) -> Float
    MathUtils.calculation_count = MathUtils.calculation_count + 1;
    x * x * x
  end

  static def max(a: Float, b: Float) -> Float
    a > b ? a : b
  end

  static def min(a: Float, b: Float) -> Float
    a < b ? a : b
  end

  static def clamp(value: Float, min_val: Float, max_val: Float) -> Float
    if value < min_val
      return min_val;
    end
    if value > max_val
      return max_val;
    end
    value
  end
end

# Using static members
print(MathUtils.PI);           # 3.14159265359
print(MathUtils.square(4.0));  # 16.0
print(MathUtils.cube(3.0));    # 27.0

result = MathUtils.clamp(150, 0, 100);
print(result);  # 100

print(MathUtils.calculation_count);  # 3
```

#### Ruby-style `def self.method_name`

As an alternative to the `static` modifier, prefix the method name with `self.` — the `self.` prefix marks the method as static. This reads naturally for users coming from Ruby and skips the surrounding `class << self ... end` block when you only have a method or two.

```soli
class MathUtils
  def self.square(x: Float) -> Float
    x * x
  end

  def self.cube(x: Float) -> Float
    x * x * x
  end
end

print(MathUtils.square(4.0));  # 16.0
print(MathUtils.cube(3.0));    # 27.0
```

`fn self.foo` works the same way (`def` and `fn` are interchangeable). Combining both — `static def self.foo` — is allowed and stays static.

#### Grouping static methods with `class << self`

When a class has several class methods, repeating the `static` modifier on each one is noisy. Soli supports the Ruby-style singleton-class form: declare a `class << self ... end` block inside the class body and every method inside is treated as static. `def` and `fn` are interchangeable.

```soli
class MathUtils
  class << self
    def square(x: Float) -> Float
      x * x
    end

    def cube(x: Float) -> Float
      x * x * x
    end

    def max(a: Float, b: Float) -> Float
      a > b ? a : b
    end
  end
end

print(MathUtils.square(4.0));  # 16.0
print(MathUtils.cube(3.0));    # 27.0
print(MathUtils.max(2.0, 7.0)); # 7.0
```

The block can sit anywhere in the class body and coexists with regular instance methods, top-level `static fn` declarations, and `static` fields. Only method declarations are allowed inside the block — fields and constants stay at the class top level (`static foo: Type = ...`). For just a single class method, the lighter `def self.foo` form (above) is usually clearer than wrapping a one-method block.

### Complete Class Example: A Product Inventory System

```soli
# Base product class
class Product
  id: String;
  name: String;
  price: Float;
  quantity: Int;

  new(id: String, name: String, price: Float, quantity: Int)
    this.id = id;
    this.name = name;
    this.price = price;
    this.quantity = quantity;
  end

  def get_total_value() -> Float
    this.price * this.quantity
  end

  def is_in_stock() -> Bool
    this.quantity > 0
  end

  def reduce_quantity(amount: Int) -> Bool
    if this.quantity >= amount
      this.quantity = this.quantity - amount;
      return true;
    end
    false
  }

  def to_string() -> String {
    this.name + " ($" + str(this.price) + ") - " + str(this.quantity) + " in stock"
  }
}

# Electronics subclass with additional warranty info
class Electronics < Product
  warranty_months: Int;
  brand: String;

  new(id: String, name: String, price: Float, quantity: Int, brand: String, warranty_months: Int)
    super(id, name, price, quantity);
    this.brand = brand;
    this.warranty_months = warranty_months;
  end

  def is_under_warranty() -> Bool
    this.warranty_months > 12
  end

  def to_string() -> String
    this.brand + " " + this.name + " - $" + str(this.price) + " (" + str(this.warranty_months) + " month warranty)"
  end
end

# Inventory class to manage products
class Inventory
  products: Product[];

  new()
    this.products = [];
  end

  def add_product(product: Product)
    push(this.products, product);
  end

  def remove_product(product_id: String) -> Product?
    for product, i in this.products
      if product.id == product_id
        return splice(this.products, i, 1)[0];
      end
    end
    null
  end

  def find_product(id: String) -> Product?
    for product in this.products
      if product.id == id
        return product;
      end
    end
    null
  end

  def get_total_inventory_value() -> Float
    total = 0.0;
    for product in this.products
      total = total + product.get_total_value();
    end
    total
  end

  def get_out_of_stock_products() -> Product[]
    this.products.filter(fn(p) !p.is_in_stock())
  end

  def list_all()
    for product in this.products
      print(product.to_string);
    end
  end
end

# Using the inventory system
inventory = new Inventory();

# Add products
inventory.add_product(new Product("P001", "Laptop", 999.99, 10));
inventory.add_product(new Electronics("E001", "Headphones", 149.99, 50, "AudioTech", 24));
inventory.add_product(new Product("P002", "Mouse", 29.99, 100));

# Work with inventory
print("Total inventory value: $" + str(inventory.get_total_inventory_value()));

laptop = inventory.find_product("P001");
if laptop != null
  print("Found: " + laptop.to_string);
end

inventory.list_all();
```

### Nested Classes

Soli supports nested classes - classes defined within other classes. This feature is useful for organizing related classes, implementing design patterns, and creating clean namespaces.

```soli
class Organization
  class Department
    def get_name()
      "Engineering"
    end

    def get_budget()
      1000000
    end
  end

  class Team
    def get_name()
      "Backend Team"
    end
  end
end
```

#### Accessing Nested Classes

Use the `::` (scope resolution operator) to access nested classes:

```soli
dept = new Organization::Department();
print("Department: " + dept.get_name());  # "Department: Engineering"
print("Budget: $" + str(dept.get_budget()));  # "Budget: $1000000"

team = new Organization::Team();
print("Team: " + team.get_name());  # "Team: Backend Team"
```

#### Use Cases

**1. Design Patterns**

Nested classes are perfect for implementing design patterns:

```soli
# State Pattern
class TrafficLight
  class RedState
    def next()
      "green"
    end

    def get_duration()
      30
    end
  end

  class GreenState
    def next()
      "yellow"
    end

    def get_duration()
      20
    end
  end

  class YellowState
    def next()
      "red"
    end

    def get_duration()
      5
    end
  end
end

red = new TrafficLight::RedState();
print("Red light duration: " + str(red.get_duration()) + "s");  # "Red light duration: 30s"
print("Next state: " + red.next);  # "Next state: green"
```

**2. Organization and Encapsulation**

Group related classes together:

```soli
class Database
  class Connection
    def connect()
      "Connected to database"
    end
  end

  class QueryBuilder
    def select(table: String)
      "SELECT * FROM " + table
    end
  end

  class Transaction
    def begin()
      "Transaction started"
    end
  end
end

conn = new Database::Connection();
query = new Database::QueryBuilder();
tx = new Database::Transaction();

print(conn.connect());  # "Connected to database"
print(query.select("users"));  # "SELECT * FROM users"
print(tx.begin());  # "Transaction started"
```

**3. Configuration Objects**

Create hierarchical configuration structures:

```soli
class Server
  class SSLConfig
    def is_enabled()
      true
    end

    def get_protocol()
      "TLS 1.3"
    end
  end

  class LoggingConfig
    def get_level()
      "INFO"
    end
  end

  def start()
    "Server starting with SSL: " + str(new Server::SSLConfig().is_enabled())
  end
end

ssl = new Server::SSLConfig();
print("Protocol: " + ssl.get_protocol());  # "Protocol: TLS 1.3"
```

#### Multiple Nested Classes

You can define multiple nested classes at the same level:

```soli
class Service
  class Database
    def connect()
      "DB connected"
    end
  end

  class Cache
    def get(key: String)
      "cached:" + key
    end
  end

  class Logger
    def log(msg: String)
      "[LOG] " + msg
    end
  end
end

db = new Service::Database();
cache = new Service::Cache();
logger = new Service::Logger();

print(db.connect());  # "DB connected"
print(cache.get("test"));  # "cached:test"
print(logger.log("test message"));  # "[LOG] test message"
```

---

## Pattern Matching

### Basic Pattern Matching

```soli
# Simple value matching
x = 42;
result = match x {
  42 => "the answer to everything",
  0 => "zero",
  _ => "something else",
};
print(result);  # "the answer to everything"

# String matching
status = "active";
status_message = match status {
  "active" => "User is active and can access the system",
  "pending" => "Awaiting approval from administrator",
  "suspended" => "Account is temporarily disabled",
  "banned" => "Access permanently denied",
  _ => "Unknown status",
};
print(status_message);  # "User is active and can access the system"
```

### Guard Clauses

```soli
# Guard clauses with conditions
n = 5;
category = match n {
  n if n < 0 => "negative",
  0 => "zero",
  n if n > 0 && n < 10 => "single digit positive",
  n if n >= 10 && n < 100 => "two digit positive",
  _ => "large number",
};
print(category);  # "single digit positive"

# Practical example: HTTP status handling
def handle_status(code: Int) -> String {
  match code {
    code if code >= 200 && code < 300 => "Success: " + str(code),
    code if code >= 300 && code < 400 => "Redirect: " + str(code),
    404 => "Not Found",
    403 => "Forbidden",
    500 => "Internal Server Error",
    code if code >= 500 => "Server Error: " + str(code),
    _ => "Client Error: " + str(code),
  }
}

print(handle_status(200));  # "Success: 200"
print(handle_status(404));  # "Not Found"
print(handle_status(503));  # "Server Error: 503"
```

### Array Patterns

```soli
numbers = [1, 2, 3];

# Match array length
description = match numbers {
  [] => "empty array",
  [_] => "single element array",
  [_, _] => "two element array",
  [_, _, _] => "three element array",
  _ => "array with more than 3 elements",
};

# Destructuring arrays
result = match numbers {
  [first] => "First element is: " + str(first),
  [first, second] => "First: " + str(first) + ", Second: " + str(second),
  [first, second, third] => "Three elements: " + str(first) + ", " + str(second) + ", " + str(third),
  [first, ...rest] => "First: " + str(first) + ", Rest has " + str(len(rest)) + " elements",
};

# Rest pattern
arr = [1, 2, 3, 4, 5];
match arr {
  [first, second, ...rest] => {
    print("First two: " + str(first) + ", " + str(second));
    print("Remaining: " + str(rest));  # [3, 4, 5]
  },
};
```

### Hash Patterns

```soli
user = {"name": "Alice", "age": 30, "city": "Paris"};

# Match hash structure
match user {
  {} => "empty object",
  {name: n} => "name is: " + n,
  {name: n, age: a} => n + " is " + str(a) + " years old",
  {name: n, age: a, city: c} => n + " is " + str(a) + " years old and lives in " + c,
  _ => "unknown structure",
};

# Nested hash matching
data = {
  "user": {"name": "Alice", "email": "alice@example.com"},
  "posts": [{"title": "Post 1"}, {"title": "Post 2"}]
};

match data {
  {user: {name: n}, posts: posts} => {
    print(n + " wrote " + str(len(posts)) + " posts");
  },
  _ => "no match",
};
```

### Type-Based Matching

```soli
# Type patterns with Any values
let value: Any = get_some_value();

def describe_value(val: Any) -> String {
  match val {
    s: String => "String with " + str(len(s)) + " characters",
    n: Int => "Integer: " + str(n),
    f: Float => "Float: " + str(f),
    b: Bool => "Boolean: " + str(b),
    arr: Array => "Array with " + str(len(arr)) + " elements",
    h: Hash => "Hash with " + str(len(h)) + " keys",
    null => "Null value",
    _ => "Unknown type: " + type(val),
  }
}

# Practical example: JSON value handler
def handle_json_value(value: Any) -> String {
  match value {
    null => "null",
    s: String => "\"" + s + "\"",
    n: Int => str(n),
    n: Float => str(n),
    true => "true",
    false => "false",
    arr: Array => "[" + join(arr.map(fn(x) handle_json_value(x)), ", ") + "]",
    h: Hash => "{" + join(h.entries.map(fn(pair) {
      k = pair[0];
      v = pair[1];
      "\"" + k + "\": " + handle_json_value(v)
    }), ", ") + "}",
    _ => "\"unknown\"",
  }
}
```

### Advanced Pattern Matching Examples

```soli
# Command pattern matching
def execute_command(command: Hash) -> String {
  match command {
    {"action": "create", "type": "user", "data": data} => {
      "Creating user: " + data["name"];
    },
    {"action": "update", "type": "user", "id": id, "data": data} => {
      "Updating user " + str(id) + ": " + data["name"];
    },
    {"action": "delete", "type": "user", "id": id} => {
      "Deleting user " + str(id);
    },
    {"action": action} => {
      "Unknown action: " + action;
    },
    _ => "Invalid command format",
  }
}

# Tree traversal with pattern matching
def process_tree(node: Any) {
  match node {
    {"type": "leaf", "value": value} => {
      {"value": value * 2};
    },
    {"type": "node", "left": left, "right": right} => {
      {
        "type": "node",
        "left": process_tree(left),
        "right": process_tree(right),
      };
    },
    {"type": "leaf"} => {"value": 0},
    _ => null,
  }
}
```

---

## Enums

An enum is a type-safe set of named **variants**. Some variants are plain
("unit"); others carry a **payload** of named fields. Reach for an enum instead
of a stringly-typed state like `status = "pending"` — you get autocompletion,
typo protection, and exhaustiveness-checked `match`.

### Declaring an enum

```soli
enum Status
  Active,
  Archived,
  Pending(reason: String)   # payload field type is optional (used by `soli check`)
end
```

Both brace and `end` forms parse; `soli fmt` normalizes to the `end` form.
Variant names are `PascalCase`, like classes, and trailing commas are optional.

### Constructing values

```soli
a = Status.Active                    # a unit variant

p = Status.Pending(reason: "kyc")    # named argument
p = Status.Pending("kyc")            # positional — same value

p.variant()                          # "Pending" — the variant tag as a String
```

### Matching variants

Match a variant with `EnumName.Variant`. A payload variant binds its fields
positionally, in declaration order:

```soli
label = match status
  Status.Active     => "Live",
  Status.Archived   => "Archived",
  Status.Pending(r) => "Waiting: " + r,   # binds `reason` to `r`
end

enum Shape
  Circle(radius: Float),
  Rect(w: Float, h: Float)
end

area = match shape
  Shape.Circle(radius) => 3.14159 * radius * radius,
  Shape.Rect(w, h)     => w * h,
end
```

### Methods on enums

An enum can carry behaviour. Inside a method, `self` is the value and
`match self` dispatches on its variant:

```soli
enum Status
  Active,
  Archived,
  Pending(reason: String)

  def label -> String
    match self
      Status.Active     => "Live",
      Status.Pending(r) => "Waiting: " + r,
      _                 => "Archived",
    end
  end
end

Status.Pending(reason: "kyc").label()   # "Waiting: kyc"
```

### Equality and introspection

Enum values compare **structurally** — same variant, equal payloads:

```soli
Status.Active == Status.Active                                # true
Status.Active == Status.Archived                              # false
Status.Pending(reason: "x") == Status.Pending(reason: "x")    # true  (structural)
Status.Pending(reason: "x") == Status.Pending(reason: "y")    # false
```

`variant()` returns the variant name as a String — useful for serializing to a
database column or JSON.

### Exhaustiveness checking

When a value's type is a known enum, `soli check` warns if a `match` misses a
variant and has no `_` catch-all. It's a **non-blocking** warning — the code
still runs:

```soli
def describe(s: Status) -> String
  match s
    Status.Active     => "live",
    Status.Pending(r) => "waiting: " + r,
  end                  # missing `Archived`, no `_`
end
```

```
$ soli check app/
warning: match on enum 'Status' is not exhaustive — missing: Archived (add them, or a `_ =>` arm)
```

### Persisting enums in models

Declare `enum_field :name, EnumType` on a model and the column round-trips
automatically: a unit variant is stored as its tag string, a payload variant as
a tagged object, and reads rebuild the enum value. Define the enum **above** the
model so it loads first.

```soli
# app/models/order.sl
enum Status
  Pending(reason: String),
  Paid,
  Shipped,
  Cancelled(reason: String)
end

class Order < Model
  enum_field :status, Status

  def can_ship -> Bool
    match this.status
      Status.Paid => true,
      _           => false,
    end
  end
end
```

A real flow — create, read, transition, and branch on the status:

```soli
# app/controllers/orders_controller.sl
def create(req)
  # The enum value is stored as its tag / tagged object automatically.
  order = Order.create({ status: Status.Pending(reason: "awaiting payment") })
  redirect("/orders/" + order._key)
end

def pay(req)
  order = Order.find(req["params"]["id"])
  order.status = Status.Paid              # stored as "Paid"
  order.save()
  render_json({ "can_ship": order.can_ship() })   # true
end

def show(req)
  order = Order.find(req["params"]["id"])
  # `order.status` comes back as a Status value, not a raw string:
  label = match order.status
    Status.Pending(r)   => "Pending: " + r,
    Status.Paid         => "Paid",
    Status.Shipped      => "Shipped",
    Status.Cancelled(r) => "Cancelled: " + r,
  end
  render("orders/show", { "order": order, "label": label })
end
```

What lands in the database column:

```soli
# unit variant    → a plain string
"Paid"

# payload variant → a tagged object
{ "variant": "Pending", "reason": "awaiting payment" }
```

To rebuild an enum from a stored value by hand (e.g. a webhook payload), use
`Status.parse(value)` — it accepts either form. (The factory is `parse`, not
`from`, because `from` is a reserved word.)

---

## Pipeline Operator

### Basic Pipeline Usage

The pipeline operator `|>` passes the left value as the first argument to the right function:

```soli
def double(x: Int) -> Int { x * 2 }
def add_one(x: Int) -> Int { x + 1 }
def square(x: Int) -> Int { x * x }

# Without pipeline (nested calls)
result1 = square(add_one(double(5)));  # Hard to read

# With pipeline (left to right)
result2 = 5 |> double() |> add_one() |> square();
print(result2);  # (5 * 2 + 1)^2 = 121
```

### Pipeline with Multiple Arguments

```soli
def add(a: Int, b: Int) -> Int { a + b }
def multiply(a: Int, b: Int) -> Int { a * b }

# 5 |> add(3) means add(5, 3)
result = 5 |> add(3) |> multiply(2);
print(result);  # (5 + 3) * 2 = 16

# More complex chaining
def subtract(a: Int, b: Int) -> Int { a - b }
def divide(a: Int, b: Int) -> Int { int(a / b) }

calc = 100
  |> |x| { subtract(x, 10) }()
  |> |x| { divide(x, 3) }()
  |> |x| { multiply(x, 4) }();
print(calc);  # ((100 - 10) / 3) * 4 = 120
```

### Pipeline with Collection Methods

Iteration over arrays uses method chaining (`.map`, `.filter`, `.reduce`, `.each`). Lambdas are most concise in pipe form — `|x| x + 1` — but `fn(x) x + 1` works too.

```soli
numbers = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];

# Method chaining with pipe lambdas
evens_squared = numbers
  .filter(|x| x % 2 == 0)
  .map(|x| x * x);
print(evens_squared);  # [4, 16, 36, 64, 100]

# Reduce with two parameters
sum_of_evens = numbers
  .filter(|x| x % 2 == 0)
  .reduce(|acc, x| acc + x, 0);
print(sum_of_evens);  # 30

# `.each` for side effects
numbers.filter(|x| x > 5).each(|x| print(x));
```

### Real-World Pipeline Examples

```soli
# Data processing pipeline
def process_user_data(raw_data: Hash) -> Hash {
  raw_data
    |> |d| { d["sanitized_email"] = d["email"].lower().trim(); d }()
    |> validate_user()
    |> enrich_profile()
    |> calculate_metrics()
}

# HTTP request pipeline
def fetch_and_process(url: String) -> Hash {
  url
    |> HTTP.get_json()
    |> transform_response()
    |> validate_data()
    |> format_output()
}

# String transformation pipeline
def format_filename(filename: String) -> String {
  filename
    |> |s| { s.lower() }()
    |> |s| { s.replace(" ", "_") }()
    |> |s| { s.replace("-", "_") }()
    |> |s| { s + ".txt" }()
}

print(format_filename("My Document"));  # "my_document.txt"
```

### Inline Transformations

```soli
# Inline transformations on arrays use method chaining + pipe lambdas
numbers = [1, 2, 3, 4, 5];

result = numbers
  .map(|x| x * 2)
  .filter(|x| x > 4)
  .reduce(|acc, x| acc + x, 0);

print(result);  # 18 (6+8+10)

# Fetch and process user with async pipeline
def get_user_data(user_id: Int) -> Hash {
  user_id
    |> |id| { {"id": id} }()
    |> fetch_from_db()
    |> enrich_with_permissions()
    |> cache_result()
}

# Complex data pipeline
sales_data = [
  {"product": "A", "quantity": 10, "price": 100},
  {"product": "B", "quantity": 5, "price": 200},
  {"product": "C", "quantity": 15, "price": 50},
];

total_revenue = sales_data
  .map(|sale| sale["quantity"] * sale["price"])
  .reduce(|acc, rev| acc + rev, 0);

print("Total Revenue: $" + str(total_revenue));  # $2750
```

---

## Modules

### Creating Modules

```soli
# math.sl - A module exporting utility functions

# Private function (not exported)
def validate_number(n: Int) -> Bool {
  n >= 0
}

# Exported functions
export def add(a: Int, b: Int) -> Int {
  a + b
}

export def subtract(a: Int, b: Int) -> Int {
  a - b
}

export def multiply(a: Int, b: Int) -> Int {
  a * b
}

export def divide(a: Int, b: Int) -> Float {
  if (b == 0) {
    panic("Division by zero");
  }
  float(a) / float(b)
}

export def factorial(n: Int) -> Int {
  if (n <= 1) {
    return 1;
  }
  n * factorial(n - 1)
}

export def fibonacci(n: Int) -> Int {
  if (n <= 1) {
    return n;
  }
  fibonacci(n - 1) + fibonacci(n - 2)
}
```

### Importing Modules

```soli
# Import all exports from a module
import "./math.sl";

print(add(2, 3));        # 5
print(factorial(5));     # 120
print(fibonacci(10));    # 55

# Named imports - only import specific functions
import { add, multiply } from "./math.sl";

sum = add(1, 2);          # 3
product = multiply(3, 4); # 12

# Aliased imports - import with different names
import { add as sum, multiply as times } from "./math.sl";

result = sum(10, 20);  # 30
doubled = times(5, 6); # 30

# Import everything with a namespace
import "./utils.sl" as utils;

formatted = utils.format_date(DateTime.utc());
cleaned = utils.sanitize_input(user_input);
```

### Module Structure Example

```
my-project/
├── soli.toml
├── src/
│   ├── main.sl
│   ├── config.sl
│   └── utils/
│       ├── mod.sl
│       ├── string.sl
│       ├── array.sl
│       └── datetime.sl
└── lib/
    └── math/
        ├── mod.sl
        ├── basic.sl
        └── advanced.sl
```

```soli
# src/main.sl
import "./config.sl";
import "./utils/mod.sl" as utils;
import "../lib/math/mod.sl" as math;

def main() {
  config = load_config();
  processed = utils.process_data(config);
  result = math.calculate(processed);
  print(result);
}
```

### Package Configuration

```toml
# soli.toml
[package]
name = "my-app"
version = "1.0.0"
description = "My awesome Soli application"
main = "src/main.sl"
soli_version = "1.16.0"   # minimum Soli version required to run this project
authors = ["Author Name <author@example.com>"]

[dependencies]
# Local dependency
utils = { path = "./lib/utils" }

# Git dependency (future)
# math = { git = "https://github.com/user/math.sl" }

[dev-dependencies]
test-utils = { path = "./tests/test-utils" }

[features]
# Optional features
debug = []
web = ["some-web-lib"]

[scripts]
dev = "soli serve"
build = "soli build --release"
test = "soli test"
```

The optional `soli_version` field declares the **minimum** Soli interpreter
version the project needs — the same idea as Cargo's `rust-version` (MSRV). When
it is set, `soli serve`, `soli test`, and running a script inside the project
refuse to start on an older `soli` and print an upgrade message:

```
Error: this project requires soli >= 1.20.0,
but you are running soli 1.16.0.
Upgrade with: soli update
```

It is a bare minimum: a running version equal to or newer than the declared one
passes. Omit the field to accept any Soli version.

---

## Built-in Functions

### I/O Functions

```soli
# Print to stdout
print("Hello, World!");
print(42);
print([1, 2, 3]);

# Print multiple values
print("Name:", "Alice", "Age:", 30);

# Read input from stdin
name = input("Enter your name: ");
print("Hello, " + name + "!");

# Input with default
age_str = input("Enter age: ", "18");
age = int(age_str);
```

### Type Conversion

```soli
num_str = "42";
float_str = "3.14";

# String conversions
num = int(num_str);        # 42
f = float(float_str);      # 3.14
s = str(123);              # "123"
s2 = str(3.14);            # "3.14"
s3 = str([1, 2, 3]);       # "[1, 2, 3]"

# Type checking
let value: Any = "hello";
print(type(value));  # "String"
print(type(123));    # "Int"
print(type(3.14));   # "Float"
print(type(true));   # "Bool"
print(type(null));   # "Null"
```

### Array Functions

```soli
let arr = [1, 2, 3, 4, 5];

# Length (also available as .len, .length, .size methods)
print(len(arr));  # 5
print(arr.len);   # 5

# Range
let nums = range(1, 6);  # [1, 2, 3, 4, 5]

# Add and remove
push(arr, 6);   # [1, 2, 3, 4, 5, 6]
let last = pop(arr);  # 6, arr is now [1, 2, 3, 4, 5]

# Sort
let unsorted = [3, 1, 4, 1, 5, 9, 2, 6];
let sorted = unsorted.sort();  # [1, 1, 2, 3, 4, 5, 6, 9]

# Sort by key
let items = [{"name": "Bob"}, {"name": "Alice"}];
let sorted_items = items.sort_by("name");  # [{name: Alice}, {name: Bob}]

# Reverse
let reversed = reverse([1, 2, 3]);  # [3, 2, 1]

# Join
let joined = join(["a", "b", "c"], ", ");  # "a, b, c"
```

### Math Functions

```soli
# Number methods
print((-5).abs);       # 5
print(16.sqrt);        # 4.0
print(2.pow(10));      # 1024
print([3, 7].min);     # 3
print([3, 7].max);     # 7

# Math class
print(Math.sin(0));         # 0.0
print(Math.cos(0));         # 1.0
print(Math.tan(0.785398));  # ~1.0 (45 degrees in radians)

# Rounding (number methods)
print(3.7.floor);      # 3
print(3.2.ceil);       # 4
print(3.5.round);      # 4
print(38.995.round(2)); # 39.0 — rounds the decimal value, not the binary float

# Random
random_num = Math.random();   # 0.0 to 1.0

# Time
print(clock());  # Current time in seconds since epoch
```

### Hash Functions

```soli
person = {"name": "Alice", "age": 30, "city": "Paris"};

# Keys and values
keys_list = keys(person);   # ["name", "age", "city"]
values_list = values(person);  # ["Alice", 30, "Paris"]

# Check existence
print(has_key(person, "name"));   # true
print(has_key(person, "email"));  # false

# Merge
additional = {"country": "France", "email": "alice@example.com"};
merged = person.merge(additional);

# Delete
deleted = delete(person, "age");
print(deleted);  # 30
print(person);   # {name: Alice, city: Paris}

# Entries
entries_list = entries(person);  # [["name", "Alice"], ["age", 30], ["city", "Paris"]]

# Clear
clear(person);
print(person);  # {}
```

### File I/O

```soli
# Write to file
let content = "Hello, World!\nLine 2";
barf("output.txt", content);

# Read from file
let data = slurp("input.txt");
print(data);

# Binary mode
barf("data.bin", bytes_data, true);  # true for binary
let binary_data = slurp("data.bin", true);
```

### HTTP Functions

```soli
# GET request (async)
response = HTTP.get("https://api.example.com/data");
print(response["status"]);  # 200
print(response["body"]);    # Response body

# GET JSON and parse automatically
json_data = HTTP.get_json("https://api.example.com/users");
print(json_data[0]["name"]);

# POST request
post_response = HTTP.post("https://api.example.com/submit", {"key": "value"});
print(post_response["body"]);

# Generic request with options
custom_request = HTTP.request("DELETE", "https://api.example.com/resource/123", {
  "headers": {"Authorization": "Bearer token123"},
  "timeout": 30,
});
```

### JSON Functions

```soli
# Parse JSON string
json_str = '{"name": "Alice", "age": 30, "scores": [95, 87, 92]}';
parsed = json_parse(json_str);

print(parsed["name"]);   # "Alice"
print(parsed["scores"]); # [95, 87, 92]

# Convert to JSON
data = {"users": [{"name": "Alice"}, {"name": "Bob"}], "count": 2};
json_string = json_stringify(data);
# '{"users":[{"name":"Alice"},{"name":"Bob"}],"count":2}'
```

### Regex Class

```soli
text = "The quick brown fox jumps over the lazy dog";

# Match check
has_fox = Regex.matches("fox", text);  # true

# Find matches
first_word = Regex.find("\\w+", text);  # "The"
all_words = Regex.find_all("\\w+", text);  # ["The", "quick", "brown", "fox", ...]

# Replace
replaced = Regex.replace("fox", text, "cat");  # "The quick brown cat..."
all_replaced = Regex.replace_all("\\s+", text, "-");  # "The-quick-brown-fox-..."

# Split
words = Regex.split("\\s+", text);  # ["The", "quick", "brown", "fox", ...]

# Capture groups
date = "2024-01-15";
captures = Regex.capture("(\\d{4})-(\\d{2})-(\\d{2})", date);
print(captures["match"]);  # "2024-01-15"
print(captures[0]);  # "2024" (when using numbered groups)
print(captures[1]);  # "01"
print(captures[2]);  # "15"

# Escape special characters
escaped = Regex.escape("file.txt (1).pdf");
# "file\\.txt\\ \\(1\\)\\.pdf"
```

### Cryptographic Functions

```soli
# Password hashing with Argon2
let password = "secure_password_123";
let hash = argon2_hash(password);
print(hash);  # Long hash string

# Verify password
let is_valid = argon2_verify(password, hash);  # true
let is_wrong = argon2_verify("wrong_password", hash);  # false

# X25519 key exchange for secure communication
let alice_keys = x25519_keypair();
let alice_private = alice_keys["private"];
let alice_public = alice_keys["public"];

let bob_keys = x25519_keypair();
let bob_private = bob_keys["private"];
let bob_public = bob_keys["public"];

# Compute shared secrets
let alice_shared = x25519_shared_secret(alice_private, bob_public);
let bob_shared = x25519_shared_secret(bob_private, alice_public);

print(alice_shared == bob_shared);  # true - same shared secret!

# Derive public key from private
let derived_public = x25519_public_key(alice_private);

# Ed25519 digital signatures
let ed_keys = ed25519_keypair();
let private_key = ed_keys["private"];
let public_key = ed_keys["public"];

# Sign a message
let message = "Hello, this message is signed";
let signature = ed25519_sign(message, private_key);

# Verify signature
let is_valid = ed25519_verify(message, signature, public_key);
print(is_valid);  # true
```

### HTML Functions

```soli
# Escape HTML special characters
user_input = "<script>alert('xss')</script>";
escaped = html_escape(user_input);
# "&lt;script&gt;alert(&#39;xss&#39;)&lt;/script&gt;"

# Unescape HTML entities
html = "&lt;div&gt;Content&lt;/div&gt;";
unescaped = html_unescape(html);  # "<div>Content</div>"

# Sanitize HTML (remove dangerous tags)
raw_html = "<div><script>evil()</script><p>Safe content</p></div>";
safe = sanitize_html(raw_html);
# "<div><p>Safe content</p></div>"
```

---

## DateTime & Duration

### Creating DateTime Instances

```soli
# Current local time
now = DateTime.utc();
print(now.to_string);  # "2024-01-15 10:30:00"

# Parse from ISO 8601 string
parsed = DateTime.parse("2024-01-15T10:30:00");
print(parsed.year());    # 2024
print(parsed.month());   # 1
print(parsed.day());     # 15

# Parse with timezone
with_tz = DateTime.parse("2024-01-15T10:30:00+05:00");

# Create from Unix timestamp
from_timestamp = DateTime.from_unix(1705315800);
```

### DateTime Methods

```soli
dt = DateTime.parse("2024-01-15T10:30:45");

# Component accessors
print(dt.year());       # 2024
print(dt.month());      # 1
print(dt.day());        # 15
print(dt.hour());       # 10
print(dt.minute());     # 30
print(dt.second());     # 45

# Weekday (0 = Monday, 6 = Sunday)
print(dt.weekday());    # "monday" (or 0)

# Conversion
unix_ts = dt.to_unix();           # 1705315845
iso_str = dt.to_iso();            # "2024-01-15T10:30:45"
human = dt.to_string;           # "2024-01-15 10:30:45"

# Formatting
custom = dt.format("%Y-%m-%d");     # "2024-01-15"
time_only = dt.format("%H:%M:%S");  # "10:30:45"

# Formatting with locale (I18n)
fr = dt.format("%A %d %B %Y", "fr");  # "lundi 15 janvier 2024"
es = dt.format("%A %d %B %Y", "es");  # "lunes 15 enero 2024"
```

### DateTime Arithmetic

```soli
let now = DateTime.utc();

# Add durations
let tomorrow = now.add(Duration.days(1));
let next_week = now.add(Duration.weeks(1));
let in_30_minutes = now.add(Duration.minutes(30));
let next_month = now.add(Duration.days(30));

# Subtract durations
let yesterday = now.sub(Duration.days(1));
let last_week = now.sub(Duration.weeks(1));

# Difference between DateTimes
let start = DateTime.parse("2024-01-01T00:00:00");
let end = DateTime.parse("2024-01-15T12:30:00");
let diff = end.sub(start);
print(diff.total_days());   # 14.5
print(diff.total_hours());  # 348.0
```

### Duration Class

```soli
# Create duration from components
dur1 = Duration.days(7);           # 7 days
dur2 = Duration.hours(24);         # 24 hours (same as 1 day)
dur3 = Duration.minutes(90);       # 90 minutes
dur4 = Duration.seconds(3600);     # 3600 seconds (1 hour)

# Duration between two DateTimes
dt1 = DateTime.parse("2024-01-01T00:00:00");
dt2 = DateTime.parse("2024-01-02T12:00:00");
between = Duration.between(dt1, dt2);

# Duration methods
print(between.total_seconds());  # 108000.0
print(between.total_minutes());  # 1800.0
print(between.total_hours());    # 30.0
print(between.total_days());     # 1.25
print(between.to_string);      # "1 day, 6 hours"
```

### Practical DateTime Examples

```soli
# Calculate age from birthdate
def calculate_age(birthdate: DateTime) -> Int {
  now = DateTime.utc();
  age = now.year() - birthdate.year();
  if (now.month() < birthdate.month() ||
    (now.month() == birthdate.month() && now.day() < birthdate.day())) {
    age = age - 1;
  }
  age
}

birthdate = DateTime.parse("1990-05-15");
print(calculate_age(birthdate));  # e.g., 33

# Format relative time
def relative_time(dt: DateTime) -> String {
  now = DateTime.utc();
  diff = now.sub(dt);

  seconds = diff.total_seconds();
  if (seconds < 60) {
    return "just now";
  }
  if (seconds < 3600) {
    mins = int(seconds / 60);
    return str(mins) + (mins == 1 ? " minute ago" : " minutes ago");
  }
  if (seconds < 86400) {
    hours = int(seconds / 3600);
    return str(hours) + (hours == 1 ? " hour ago" : " hours ago");
  }
  if (seconds < 604800) {
    days = int(seconds / 86400);
    return str(days) + (days == 1 ? " day ago" : " days ago");
  }
  dt.to_string
}

# Check if date is in the past
def is_past(dt: DateTime) -> Bool {
  now = DateTime.utc();
  dt.to_unix() < now.to_unix()
}

# Get start of day
def start_of_day(dt: DateTime) -> DateTime {
  DateTime.parse(
    str(dt.year()) + "-" +
    str(dt.month()) + "-" +
    str(dt.day()) + "T00:00:00"
  )
}

# Get business days between two dates
def business_days(start: DateTime, end: DateTime) -> Int {
  count = 0;
  current = start;
  while (current.to_unix() <= end.to_unix()) {
    weekday = current.weekday();
    if (weekday != "saturday" && weekday != "sunday") {
      count = count + 1;
    }
    current = current.add(Duration.days(1));
  }
  count
}
```

---

## Linting

Soli includes a built-in linter (`soli lint`) that catches style issues and code smells without executing your code.

### Usage

```bash
soli lint              # all .sl files in current directory (recursive)
soli lint src/         # lint a directory
soli lint app/main.sl  # lint a single file
```

Exit code: `0` = clean, `1` = issues found.

### Output Format

```
app/main.sl:12:5 - [naming/snake-case] variable 'myVar' should use snake_case
app/main.sl:30:9 - [smell/unreachable-code] unreachable code after return statement

2 issue(s) found in 1 file(s)
```

### Rules

| Rule | Description |
|---|---|
| `naming/snake-case` | Variables, functions, methods, and parameters should use `snake_case` |
| `naming/pascal-case` | Classes and interfaces should use `PascalCase` |
| `style/empty-block` | Blocks should not be empty |
| `style/line-length` | Lines should not exceed 120 characters |
| `smell/unreachable-code` | Code after a `return` statement is unreachable |
| `smell/empty-catch` | Catch blocks should not be empty (silently swallowing errors) |
| `smell/deep-nesting` | Nesting depth should not exceed 4 levels |
| `smell/duplicate-methods` | A class should not have two methods with the same name |
| `smell/dangerous-server-builtin` | Calls to `db_query_raw`, `Trusted.*`, `System.shell` / `System.shell_sync`, or backtick command substitution from `app/controllers/`, `app/middleware/`, or `app/views/`. Suggests the safe alternative: parameterised `@sdbql{ ... #{value} ... }`, the jailed `File.*` API, or `System.run([...])` with an argv array. Models, migrations, and tests are out of scope. |
| `style/redundant-model-import` | No `import "../models/*.sl"` inside `app/controllers/` — models are auto-loaded |
| `idiom/nil-comparison` | Prefer `.nil?` / `.present?` over `== null` / `!= null` |
| `idiom/prefer-blank` | Prefer `.blank?` / `.present?` over comparing to an empty string (`.blank?` also covers nil) |
| `idiom/prefer-includes` | Replace a chain of 3+ same-value `==`/`!=` comparisons with `.includes?` |
| `idiom/manual-find-guard` | Drop the nil-check after `Model.find` — it raises on a miss (handled as a 404); use `find_by`/`first_by` for "or nil" |
| `component/props` | A component's `props(...)` declaration must use string-literal names with no duplicates |

### Suppressing Warnings

When a warning is a known false-positive or an intentional exception, suppress it inline with a directive comment.

**Single-line forms.** `disable-next-line` covers the line below; `disable-line` covers the same line.

```soli
# soli-lint-disable-next-line smell/dangerous-server-builtin
if Trusted.is_dir(wt_path)
  ...
end

Trusted.read(p)  # soli-lint-disable-line smell/dangerous-server-builtin
```

**Block forms.** `disable` / `enable` toggle a rule for a region of code. Useful when several adjacent lines are intentional exceptions:

```soli
# soli-lint-disable smell/dangerous-server-builtin
exists = Trusted.is_dir(path)
data   = Trusted.read(path)
# soli-lint-enable smell/dangerous-server-builtin
```

- Omit the rule name to suppress every rule (e.g. `# soli-lint-disable`). Pass a comma-separated list to scope to multiple rules.
- An `enable` for a specific rule re-enables only that rule, even if the prior `disable` was a blanket one.
- A block `disable` with no matching `enable` runs to the end of the file.
- Prefer naming the exact rule so unrelated warnings still surface.

### Editor Integration

The VS Code / Cursor extension (`editors/vscode/`) provides full Language Server Protocol (LSP) support for Soli, including:

- **Real-time linting** - warnings and errors displayed inline as you type
- **Hover information** - documentation for functions, classes, and builtins
- **Autocomplete** - suggestions for keywords, types, and symbols
- **Go to definition** - jump to symbol definitions
- **Find references** - locate all uses of a symbol
- **Document symbols** - outline view of classes, functions, and methods
- **Code folding** - fold code blocks and classes
- **Inlay hints** - type annotations displayed inline

#### Installation

**From VSIX (recommended):**

```bash
cd editors/vscode
vsce package
# Install the generated .vsix file in Cursor/VS Code
```

**From source:**

Copy the extension folder to your editor's extensions directory:

- **Cursor/VS Code (Linux):** `~/.cursor/extensions/` or `~/.vscode/extensions/`
- **Cursor/VS Code (macOS):** `~/.cursor/extensions/` or `~/.vscode/extensions/`
- **Cursor/VS Code (Windows):** `%USERPROFILE%\.cursor\extensions\` or `%USERPROFILE%\.vscode\extensions\`

#### Configuration

The extension works with sensible defaults but supports customization:

```json
{
 "soli.lsp.enable": true,
 "soli.lsp.executablePath": "soli",
 "soli.lint.enable": true,
 "soli.lint.onSave": true
}
```

| Setting | Default | Description |
|---------|---------|-------------|
| `soli.lsp.enable` | `true` | Enable/disable LSP server |
| `soli.lsp.executablePath` | `"soli"` | Path to the `soli` binary |
| `soli.lint.enable` | `true` | Run linter on save |
| `soli.lint.onSave` | `true` | Lint file when saving |

#### Manual LSP Setup

For editors that support custom LSP servers directly (Neovim, Emacs, etc.), configure:

```lua
-- Neovim with lspconfig
require('lspconfig').soli.setup({
 cmd = {"soli", "lsp"},
 filetypes = {"soli"},
 root_dir = lspconfig.util.root_pattern("soli.toml", ".git"),
})
```

```json
// Generic JSON config for LSP-compatible editors
{
 "name": "soli",
 "command": "soli lsp",
 "filetypes": ["soli"],
 "rootPatterns": ["soli.toml"],
 "languageId": "soli"
}
```

#### Available LSP Features

| Feature | Description |
|---------|-------------|
| `textDocument/completion` | Keywords, types, builtins, and local symbols |
| `textDocument/hover` | Documentation for symbols and builtins |
| `textDocument/definition` | Jump to symbol definitions |
| `textDocument/references` | Find all references to a symbol |
| `textDocument/documentSymbol` | Hierarchical symbol tree |
| `textDocument/foldingRange` | Fold classes, functions, and blocks |
| `textDocument/inlayHint` | Type annotations for variables |
| `textDocument/codeAction` | Quick fixes for lint violations |
| `textDocument/formatting` | Format document |

---

## Best Practices

### Variables & Types

```soli
# Good: Use type inference when obvious
count = 10;
name = "Alice";

# Good: Add annotations for public API or complex types
pub fn process_user(user_id: Int) -> User {
  # ...
}

# Good: Use meaningful names
items_per_page = 25;
max_retry_attempts = 3;

# Avoid: Single-letter names except for loop variables
c = 10;           # Bad
item_count = 10;  # Good
```

### Functions

```soli
# Good: Single responsibility
def validate_email(email: String) -> Bool {
  email.contains("@") && email.contains(".")
}

# Good: Descriptive names
def calculate_total_with_tax() -> Float {
  # ...
}

# Good: Limit parameters
def create_user(info: Hash) -> User {
  # Instead of: def create_user(name, email, age, address, phone)
}

# Good: Early returns for validation
def process_order(order: Hash) -> Result {
  if (!has_key(order, "items")) {
    return {"error": "Missing items"};
  }
  if (!has_key(order, "customer")) {
    return {"error": "Missing customer"};
  }
  # Main logic...
}
```

### Collections

```soli
# Good: Initialize with known values
weekdays = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday"];

# Good: Check bounds
def safe_get(arr: Array, index: Int) -> Any? {
  if (index >= 0 && index < len(arr)) {
    return arr[index];
  }
  null
}

# Good: Use functional methods for transformations
doubled = numbers.map(fn(x) x * 2);
evens = numbers.filter(fn(x) x % 2 == 0);
```

### Classes

```soli
# Good: Single responsibility
class User {}
class UserRepository {}
class UserService {}

# Good: Program to interfaces
interface Repository {
  def find(id: Int) -> Any?;
  def save(entity: Any);
}

# Good: Use private fields for encapsulation
class BankAccount {
  private balance: Float;

  public def deposit(amount: Float) {
    # ...
  }

  public def get_balance() -> Float {
    this.balance
  }
}
```

### Control Flow

```soli
# Good: Avoid deep nesting
def process(data: Hash) -> Result {
  if (data == null) {
    return {"error": "No data"};
  }

  if (!has_key(data, "required")) {
    return {"error": "Missing required field"};
  }

  # Main logic at less indentation
  do_processing(data)
}

# Good: Use pattern matching for complex conditions
def handle_event(event: Hash) -> String {
  match event {
    {"type": "click", "element": "button"} => "Button clicked",
    {"type": "submit", "form": form} => "Form submitted",
    {"type": "error", "message": msg} => "Error: " + msg,
    _ => "Unknown event",
  }
}
```

---

## Quick Reference

### Hello World
```soli
print("Hello, World!");
```

### Variables
```soli
name = "Alice";
let age: Int = 30;
const PI = 3.14159;
```

### Functions
```soli
def add(a: Int, b: Int) -> Int {
  a + b
}

# `def` is an alias for `fn`
def greet(name: String)
  print("Hello, " + name + "!");
end
```

### Classes
```soli
class Person {
  name: String;
  new(name: String) {
    this.name = name;
  }
  def greet() -> String {
    "Hello, " + this.name
  }
}
```

### Control Flow
```soli
if (condition) {
  # code
} elsif (other) {
  # code
} else {
  # code
}

for (item in collection) {
  # code
}

while (condition) {
  # code
}

match value {
  pattern => result,
  _ => default,
}
```

### Error Handling
```soli
try
  risky_operation();
catch e
  print("Error: " + str(e));
finally
  cleanup();
end
```

### Collections
```soli
arr = [1, 2, 3];
hash = {"key": "value"};

arr.map(fn(x) x * 2);
hash.filter(fn(pair) pair[1] > 0);
```

### Pipeline
```soli
value |> fn1() |> fn2();
```

### Module
```soli
# math.sl
export def add(a: Int, b: Int) -> Int {
  a + b
}

# main.sl
import "./math.sl";
print(add(2, 3));
```

---

## Next Steps

- [Installation Guide](/docs/installation/) - Set up your development environment
- [Quickstart](/docs/quickstart/) - Build your first application
- [MVC Framework](/docs/introduction/) - Learn the web framework
- [Routing](/docs/routing/) - Define API routes
- [Controllers](/docs/controllers/) - Handle HTTP requests
- [Views](/docs/views/) - Render responses
- [Middleware](/docs/middleware/) - Add request/response processing
- [Testing](/docs/testing/) - Write tests for your code
