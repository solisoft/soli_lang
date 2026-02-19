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
10. [Pipeline Operator](#pipeline-operator)
11. [Modules](#modules)
12. [Built-in Functions](#built-in-functions)
13. [DateTime & Duration](#datetime--duration)
14. [Linting](#linting)

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
fn calculate_area(radius: Float) -> Float
    3.14159 * radius * radius
end

let radius = 5.0;
let area = calculate_area(radius);
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
let name = "Alice";           # String
let age = 30;                 # Int
let temperature = 98.6;       # Float
let is_active = true;         # Bool
let nothing = null;           # Null
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
let count = 42;
let negative = -100;
let large = 9_000_000;  # Underscores for readability

# Float - 64-bit floating-point
let pi = 3.14159;
let small = 0.001;
let scientific = 2.5e10;  # 25000000000.0

# String - UTF-8 text
let greeting = "Hello, World!";
let multiline = "Line 1\nLine 2\tTabbed";
let raw = r"Path: C:\Users\name";  # Raw string (no escape processing)

# Multiline strings
let poem = """The fog comes
on little cat feet.""";

let story = [[Once upon
a time in
the wild west.]];

# Command substitution - execute shell commands
let files = `ls *.sl`;        # Returns Future<{stdout, stderr, exit_code}>
let output = files.stdout;     # Auto-resolves when accessed
let code = files.exit_code;    # Exit code (0 = success)

# Bool - Boolean values
let is_valid = true;
let is_complete = false;

# Null - Absence of value
let missing = null;
```

### Type Inference

Soli automatically infers types when not explicitly specified:

```soli
# Type inference examples
let x = 5;              # Inferred as Int
let y = 3.14;           # Inferred as Float
let z = "hello";        # Inferred as String
let flag = true;        # Inferred as Bool
let nums = [1, 2, 3];   # Inferred as Int[]
let person = {"name": "Alice"};  # Inferred as Hash

# You can always add annotations even with inference
let id = 123;  # Int - inferred
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
let x = 1;

if true
    let y = 2;      # y is only visible in this block
    let x = 3;      # This shadows the outer x
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
let message = "outer";

if true
    let message = "inner";
    print(message);  # "inner"
end

print(message);      # "outer"

# Common use case: transforming data
let data = get_data();
if data != null
    let data = process(data);  # Transform while keeping same name
    print(data);
end
```

---

## Operators

### Arithmetic Operators

```soli
let a = 10;
let b = 3;

# Basic arithmetic
print(a + b);   # 13  (addition)
print(a - b);   # 7   (subtraction)
print(a * b);   # 30  (multiplication)
print(a / b);   # 3.3333333333333335  (division - always float!)
print(a % b);   # 1   (modulo)

# Integer division requires special handling
let int_result = int(a / b);  # 3
let remainder = a % b;        # 1

# Compound assignment
let counter = 0;
counter = counter + 1;  # 1
counter += 1;           # 2 (shorthand)
counter *= 2;           # 4
```

### Comparison Operators

```soli
let x = 5;
let y = 10;

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
let age = 25;
let has_license = true;
let is_weekend = false;

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
let score = 85;
if score >= 90 && attendance >= 80
    print("Grade: A");
elsif score >= 80 || extra_credit > 10
    print("Grade: B");
end
```

### String Operations

```soli
# Concatenation
let greeting = "Hello, " + "World!";    # "Hello, World!"
let message = "Value: " + 42;           # "Value: 42" (auto-conversion)
let path = "/home/" + "user";           # "/home/user"

# String methods
let text = "  Hello, World!  ";
print(text.trim());        # "Hello, World!" (removes whitespace)
print(text.upper());       # "  HELLO, WORLD!  "
print(text.lower());       # "  hello, world!  "
print(text.len());         # 18

# Substring operations
let s = "Hello, World!";
print(s.sub(0, 5));        # "Hello" (from index 0, length 5)
print(s.find("World"));    # 7 (index of first occurrence)
print(s.contains("Hello"));  # true
print(s.starts_with("Hell"));  # true
print(s.ends_with("!"));      # true

# String transformation
let snake_case = "HelloWorld".snake_case();  # "hello_world"
let camel_case = "hello_world".camel_case(); # "helloWorld"

# String interpolation
let name = "World";
let greeting = "Hello #{name}!";           # "Hello World!"
let a = 2;
let b = 3;
let result = "Sum is #{a + b}";             # "Sum is 5"
let first = "John";
let last = "Doe";
let full = "#{first} #{last}";              # "John Doe"
let text = "hello";
let upper = "Upper: #{text.upper()}";        # "Upper: HELLO"
let items = ["Alice", "Bob"];
let first_item = "First: #{items[0]}";       # "First: Alice"
let person = {"name": "Charlie"};
let person_name = "Name: #{person["name"]}"; # "Name: Charlie"
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
let user = {"name": "Alice", "email": null};

# Traditional null check
let email = user["email"];
if email == null
    email = "unknown";
end

# Null coalescing operator
let display_email = user["email"] ?? "unknown";

# Chaining with null values
let city = user["address"]["city"] ?? "Unknown City";
# If any key in the chain is null/missing, returns "Unknown City"

# Safe navigation operator (&.)
# Access properties or call methods on values that might be null
let user = get_user()  # might return null

let name = user&.name              # null if user is null, otherwise user.name
let city = user&.address&.city     # chain for nested access
let greeting = user&.greet()       # null if user is null, otherwise calls greet()
let display = user&.name ?? "Anon" # combine with ?? for defaults
```

---

## Control Flow

### If/Else Statements

```soli
let age = 18;

# Simple if
if age >= 18
    print("Adult");
end

# If-else
let score = 75;
if score >= 60
    print("Pass");
else
    print("Fail");
end

# Else-if chain
let grade = 85;
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
let is_weekend = true;
let is_holiday = false;
let has_plans = true;

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

### While Loops

```soli
# Basic while loop
let i = 0;
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
let data = [1, 2, 3, 4, 5];
let sum = 0;
let idx = 0;
while idx < len(data) && data[idx] < 4
    sum = sum + data[idx];
    idx = idx + 1;
end
print("Sum: " + str(sum));  # 6 (1+2+3)

# Do-while equivalent (using break)
let count = 0;
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

# Iterate with index
for i, fruit in fruits
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

# Break and continue
let numbers = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
let sum = 0;
for n in numbers
    if n % 2 == 0
        continue;  # Skip even numbers
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
let x = 10;
print("big") if (x > 5);

let y = 3;
print("small") unless (y > 5);

# More examples
let status = "active";
print("Welcome!") if (status == "active");
print("Account locked") unless (status != "banned");

let items = [];
print("Empty") if (len(items) == 0);
```

### Ternary Operator

```soli
# Basic ternary
let x = 10;
let size = x > 5 ? "large" : "small";
print(size);  # "large"

# Nested ternary
let grade = 85;
let letter = grade >= 90 ? "A"
             : grade >= 80 ? "B"
             : grade >= 70 ? "C"
             : grade >= 60 ? "D"
             : "F";
print(letter);  # "B"

# In assignments
let max_val = a > b ? a : b;
let status = is_valid ? "valid" : "invalid";
```

### Match Expression

Soli's powerful pattern matching (covered in detail in the Pattern Matching section):

```soli
# Basic match
let x = 42;
let result = match x {
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
let falsy_values = [false, null, 0, 0.0, "", []];

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
let config = get_config();
if config
    print("Config loaded: " + str(config));
end

let items = get_items() ?? [];
if len(items) > 0
    print("Found " + str(len(items)) + " items");
end
```

---

## Error Handling

### Try / Catch / Finally

Soli provides `try`/`catch`/`finally` for exception handling, using `end`-delimited blocks (just like `if`, `while`, and `for`).

```soli
# Basic try/catch
try
    let result = 10 / 0;
catch e
    print("Error: " + str(e));
end

# With finally (always runs)
try
    let data = read_file("config.sl");
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
fn divide(a: Int, b: Int) -> Int
    if b == 0
        throw "Division by zero";
    end
    a / b
end

try
    let result = divide(10, 0);
catch e
    print("Caught: " + str(e));  # "Caught: Division by zero"
end
```

### Brace Syntax

Try/catch also supports brace-delimited blocks:

```soli
try {
    let result = risky_operation();
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
fn say_hello
    print("Hello!");
end

# Equivalent with explicit empty parentheses
fn say_hello()
    print("Hello!");
end

# `def` works exactly like `fn`
def greet(name: String)
    print("Hello, " + name + "!");
end

# With return value
fn add(a: Int, b: Int) -> Int
    a + b
end

# Void function (explicit)
def log_message(msg: String)
    print("[LOG] " + msg);
end

# With type annotations on return
fn multiply(a: Float, b: Float) -> Float
    a * b
end
```

### Function Examples

```soli
# Calculate factorial
fn factorial(n: Int) -> Int
    if n <= 1
        return 1;
    end
    n * factorial(n - 1)
end

print(factorial(5));  # 120

# Calculate Fibonacci
fn fibonacci(n: Int) -> Int
    if n <= 1
        return n;
    end
    fibonacci(n - 1) + fibonacci(n - 2)
end

print(fibonacci(10));  # 55

# Check if a number is prime
fn is_prime(n: Int) -> Bool
    if n < 2
        return false;
    end
    if n == 2
        return true;
    end
    if n % 2 == 0
        return false;
    end
    let i = 3;
    while i * i <= n
        if n % i == 0
            return false;
        end
        i = i + 2;
    end
    true
end

# Find maximum in array
fn find_max(arr: Int[]) -> Int
    if len(arr) == 0
        return 0;  # or panic for empty array
    end
    let max = arr[0];
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
fn process_user(user: Hash) -> Hash
    # Validate required fields
    if !has_key(user, "name")
        return {"error": "Name is required"};
    end
    if !has_key(user, "email")
        return {"error": "Email is required"};
    end

    # Validate email format
    let email = user["email"];
    if !email.contains("@")
        return {"error": "Invalid email format"};
    end

    # Process user data
    let processed = user;
    processed["status"] = "active";
    processed["created_at"] = DateTime.utc().to_iso();

    processed
end
```

### Higher-Order Functions

Functions can accept other functions as parameters and return functions:

```soli
# Function as parameter
fn apply(x: Int, f: (Int) -> Int) -> Int
    f(x)
end

fn double(x: Int) -> Int
    x * 2
end

fn square(x: Int) -> Int
    x * x
end

let result = apply(5, double);   # 10
let squared = apply(5, square);  # 25

# Passing anonymous functions
fn transform_array(arr: Int[], transformer: (Int) -> Int) -> Int[]
    let result = [];
    for item in arr
        push(result, transformer(item));
    end
    result
end

let numbers = [1, 2, 3, 4, 5];
let doubled = transform_array(numbers, fn(x) x * 2);  # [2, 4, 6, 8, 10]

# Function that returns a function
fn multiplier(factor: Int) -> (Int) -> Int
    fn closure(x: Int) -> Int
        x * factor
    end
    closure
end

let times_two = multiplier(2);
print(times_two(5));   # 10
print(times_two(10));  # 20

let times_three = multiplier(3);
print(times_three(5));  # 15
```

### Closures

```soli
# Counter using closure
fn make_counter() -> () -> Int
    let count = 0;
    fn counter() -> Int
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
fn make_greeter(greeting: String) -> (String) -> String
    fn greet(name: String) -> String
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
fn greet(name: String, greeting: String = "Hello") -> String
    greeting + ", " + name + "!"
end

print(greet("Alice"));              # "Hello, Alice!"
print(greet("Bob", "Hi"));          # "Hi, Bob!"
print(greet("Charlie", "Welcome")); # "Welcome, Charlie!"

# Optional parameters
fn create_user(name: String, email: String = null, role: String = "user") -> Hash
    let user = {"name": name, "role": role};
    if email != null
        user["email"] = email;
    end
    user
end

let user1 = create_user("Alice");
let user2 = create_user("Bob", "bob@example.com");
let user3 = create_user("Charlie", "charlie@example.com", "admin");
```

### Named Parameters

You can call functions using named parameters with the colon syntax:

```soli
fn configure(host: String = "localhost", port: Int = 8080, debug: Bool = false) -> Void
    print("Connecting to \(host):\(port) with debug=\(debug)");
end

configure();                              # Using all defaults
configure(host: "example.com");           # Only specify host
configure(port: 3000, debug: true);       # Named parameters in any order
configure("example.com", port: 443);      # Mixed: positional then named
configure(host: "api.example.com", port: 443, debug: true);  # All named
```

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
fn http_request(
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
fn sum(numbers: Int[]) -> Int
    let total = 0;
    for n in numbers
        total = total + n;
    end
    total
end

print(sum([1, 2, 3, 4, 5]));  # 15

# Using spread operator
let nums = [1, 2, 3];
print(sum(nums));             # 6
print(sum([...nums, 4, 5]));  # 15

# Variadic-like with array
fn format_list(items: String[], separator: String = ", ", final_separator: String = "and") -> String
    let len = len(items);
    if len == 0
        return "";
    end
    if len == 1
        return items[0];
    end
    if len == 2
        return items[0] + " " + final_separator + " " + items[1];
    end
    let result = "";
    for i in range(0, len - 1)
        result = result + items[i] + separator;
    end
    result + final_separator + " " + items[len - 1]
end

print(format_list(["apple", "banana", "cherry"]));  # "apple, banana and cherry"
print(format_list(["one", "two"]));                 # "one and two"
```

---

## Collections

### Arrays

#### Creating Arrays

```soli
# Basic array creation
let numbers = [1, 2, 3, 4, 5];
let names = ["Alice", "Bob", "Charlie"];
let mixed = [1, "two", 3.0, true];

# Type-annotated arrays
let scores: Int[] = [95, 87, 92, 88, 90];
let words: String[] = [];  # Empty array

# Array from range
let range_arr = range(1, 10);  # [1, 2, 3, 4, 5, 6, 7, 8, 9]
let step_arr = range(0, 10, 2);  # [0, 2, 4, 6, 8]

# Initialize with default value
let zeros = [];
for _ in range(0, 5)
    push(zeros, 0);
end  # [0, 0, 0, 0, 0]
```

#### Array Access and Modification

```soli
let fruits = ["apple", "banana", "cherry", "date"];

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
fn slice(arr: Array, start: Int, end: Int) -> Array {
    let result = [];
    let actual_end = end;
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
let numbers = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];

# map - transform each element
let doubled = numbers.map(fn(x) x * 2);
print(doubled);  # [2, 4, 6, 8, 10, 12, 14, 16, 18, 20]

# filter - keep elements matching condition
let evens = numbers.filter(fn(x) x % 2 == 0);
print(evens);  # [2, 4, 6, 8, 10]

# each - iterate with side effects
numbers.each(fn(x) print(x));  # Prints each number

# reduce - accumulate to single value
let sum = numbers.reduce(fn(acc, x) acc + x, 0);  # 55
let product = numbers.reduce(fn(acc, x) acc * x, 1);  # 3628800

# find - first matching element
let first_even = numbers.find(fn(x) x % 2 == 0);  # 2

# find_index - index of first matching element
let idx = numbers.find_index(fn(x) x > 5);  # 5

# every - check if all elements match
let all_positive = numbers.every(fn(x) x > 0);  # true

# some - check if any element matches
let has_large = numbers.some(fn(x) x > 8);  # true

# chunk - split into chunks
fn chunk(arr: Array, size: Int) -> Array[]
    let result = [];
    let current = [];
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
let result = numbers
    .filter(fn(x) x % 2 == 0)   # [2, 4, 6, 8, 10]
    .map(fn(x) x * x)           # [4, 16, 36, 64, 100]
    .filter(fn(x) x < 50);      # [4, 16, 36]

print(result);  # [4, 16, 36]
```

#### Array Functions

```soli
let arr = [1, 2, 3, 4, 5];

# Length
print(len(arr));  # 5

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
fn insert(arr: Array, index: Int, value: Any) -> Array
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
let person = {
    "name": "Alice",
    "age": 30,
    "city": "New York"
};

# Alternative syntax with =>
let scores = {"Alice" => 95, "Bob" => 87, "Charlie" => 92};

# Nested hashes
let user = {
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
let empty = {};

# Type-annotated hash
let config: Hash = {
    "host": "localhost",
    "port": 8080,
    "ssl": false
};
```

#### Hash Access and Modification

```soli
let person = {"name": "Alice", "age": 30, "city": "Paris"};

# Access values
print(person["name"]);   # "Alice"
print(person["age"]);    # 30
print(person["email"]);  # null (key doesn't exist)

# Modify values
person["age"] = 31;
person["country"] = "France";  # Add new key

print(person);  # {name: Alice, age: 31, city: Paris, country: France}

# Delete key
let deleted = delete(person, "city");
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
let scores = {"Alice": 90, "Bob": 85, "Charlie": 95, "Diana": 88};

# map - transform entries
# Returns a new hash. The function MUST return [key, value] (exactly 2 elements).
# Returning fewer or more elements skips that entry.
let curved = scores.map(fn(k, v) [k, v + 5]);
print(curved);  # {Alice: 95, Bob: 90, Charlie: 100, Diana: 93}

# Transform only values (keep key unchanged)
let doubled = scores.map(fn(k, v) [k, v * 2]);

# Transform keys (prefix with "user_")
let prefixed = scores.map(fn(k, v) ["user_" + k, v]);

# filter - keep entries matching condition
# Function receives (key, value) or [key, value] pair
# Returns boolean or truthy/falsy value
let high_scores = scores.filter(fn(k, v) v >= 90);
print(high_scores);  # {Alice: 90, Charlie: 95}

# each - iterate with side effects
# Function receives (key, value) or [key, value] pair
# Returns original hash for chaining (return value is discarded)
scores.each(fn(k, v) print(k + ": " + str(v)));
```

**Important: map return value**

Hash `.map()` expects your function to return exactly `[key, value]` with 2 elements:

```soli
let h = {"a": 1, "b": 2};

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
let h = {"a": 1, "b": 2};

# Get values first, then map to array
let doubled = h.values() |> map(fn(v) v * 10);
print(doubled);  # [10, 20]
```

#### Hash Functions

```soli
let person = {"name": "Alice", "age": 30, "city": "Paris", "country": "France"};

# Get length
print(len(person));  # 4

# Get all keys
let keys_list = keys(person);
print(keys_list);  # [name, age, city, country]

# Get all values
let values_list = values(person);
print(values_list);  # [Alice, 30, Paris, France]

# Check if key exists
print(has_key(person, "name"));      # true
print(has_key(person, "email"));     # false

# Get entries as [key, value] pairs
let entries_list = entries(person);
print(entries_list);  # [[name, Alice], [age, 30], [city, Paris], [country, France]]

# Merge hashes
let defaults = {"age": 0, "country": "Unknown", "active": true};
let merged = merge(person, defaults);
print(merged);  # {name: Alice, age: 30, city: Paris, country: France, active: true}

# Clear hash
clear(person);
print(person);  # {}
```

#### Iterating Over Hashes

```soli
let prices = {"apple": 1.50, "banana": 0.75, "orange": 2.00, "grape": 3.00};

# Iterate entries
for pair in entries(prices)
    let item = pair[0];
    let price = pair[1];
    print(item + " costs $" + str(price));
end

# Iterate keys
for item in keys(prices)
    print(item + ": " + str(prices[item]));
end

# Iterate values and calculate total
let total = 0;
for price in values(prices)
    total = total + price;
end
print("Total: $" + str(total));  # Total: $7.25

# Filter and transform
let expensive = prices
    .filter(fn(k, v) v > 1.00)
    .map(fn(k, v) [k, v * 1.1]);  # 10% tax

print(expensive);  # {apple: 1.65, orange: 2.2, grape: 3.3}
```

### Common Collection Patterns

```soli
# Slicing
fn slice(arr: Array, start: Int, end: Int) -> Array
    let result = [];
    let actual_end = end;
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
    fn greet() -> String
        "Hello, I'm " + this.name
    end

    fn introduce() -> String
        let intro = "Hi, I'm " + this.name + " and I'm " + str(this.age) + " years old";
        if this.email != ""
            intro = intro + ". You can reach me at " + this.email;
        end
        intro
    end

    fn have_birthday()
        this.age = this.age + 1;
    end
end

# Creating instances
let alice = new Person("Alice", 30);
let bob = new Person("Bob", 25, "bob@example.com");

# Using instances
print(alice.greet());      # "Hello, I'm Alice"
print(bob.introduce());    # "Hi, I'm Bob and I'm 25 years old. You can reach me at bob@example.com"

alice.have_birthday();
print(alice.age);          # 31
```

### Constructors and Factory Methods

```soli
class Rectangle
    width: Float;
    height: Float;

    new(width: Float, height: Float)
        this.width = width;
        this.height = height;
    end

    fn area() -> Float
        this.width * this.height
    end

    fn perimeter() -> Float
        2 * (this.width + this.height)
    end

    # Static factory method
    static fn square(side: Float) -> Rectangle
        new Rectangle(side, side)
    end

    # Another factory method
    static fn from_area(area: Float, aspect_ratio: Float = 1.0) -> Rectangle
        let width = sqrt(area / aspect_ratio);
        let height = width * aspect_ratio;
        new Rectangle(width, height)
    end
end

let rect = new Rectangle(10.0, 5.0);
print(rect.area());  # 50.0

let square = Rectangle.square(7.0);
print(square.area());  # 49.0

let from_area = Rectangle.from_area(24.0, 2.0);  # 2:1 aspect ratio
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

    fn speak() -> String
        this.name + " makes a sound"
    end

    fn get_info() -> String
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
    fn speak() -> String
        this.name + " barks!"
    end

    # Subclass-specific method
    fn fetch() -> String
        this.name + " fetches the ball!"
    end
end

# Another subclass
class Cat < Animal
    new(name: String, age: Int)
        super(name, age);
    end

    fn speak() -> String
        this.name + " meows!"
    end

    fn purr() -> String
        this.name + " purrs contentedly"
    end
end

# Using inheritance
let dog = new Dog("Buddy", 3, "Golden Retriever");
print(dog.speak());        # "Buddy barks!"
print(dog.get_info());     # "Buddy is 3 years old"
print(dog.fetch());        # "Buddy fetches the ball!"
print(dog.breed);          # "Golden Retriever"

let cat = new Cat("Whiskers", 5);
print(cat.speak());        # "Whiskers meows!"
print(cat.purr());         # "Whiskers purrs contentedly"

# Polymorphism
let animals = [
    new Dog("Rex", 2, "German Shepherd"),
    new Cat("Mittens", 4),
    new Dog("Spot", 1, "Beagle")
];

for animal in animals
    print(animal.speak());  # Each calls the appropriate speak() method
end
```

### Interfaces

```soli
# Define an interface
interface Drawable
    fn draw() -> String;
    fn get_color() -> String;
end

# Another interface
interface Resizable
    fn resize(width: Float, height: Float);
    fn get_dimensions() -> {width: Float, height: Float};
end

# Class implementing multiple interfaces
class Circle implements Drawable, Resizable
    radius: Float;
    color: String;

    new(radius: Float, color: String)
        this.radius = radius;
        this.color = color;
    end

    fn draw() -> String
        "Circle with radius " + str(this.radius) + " and color " + this.color
    end

    fn get_color() -> String
        this.color
    end

    fn resize(width: Float, height: Float)
        # For a circle, we use the average or just one dimension
        this.radius = width / 2;
    end

    fn get_dimensions() -> {width: Float, height: Float}
        {"width": this.radius * 2, "height": this.radius * 2}
    end
end

class Rectangle implements Drawable, Resizable
    width: Float;
    height: Float;
    color: String;

    new(width: Float, height: Float, color: String)
        this.width = width;
        this.height = height;
        this.color = color;
    end

    fn draw() -> String
        "Rectangle " + str(this.width) + "x" + str(this.height) + " in " + this.color
    end

    fn get_color() -> String
        this.color
    end

    fn resize(width: Float, height: Float)
        this.width = width;
        this.height = height;
    end

    fn get_dimensions() -> {width: Float, height: Float}
        {"width": this.width, "height": this.height}
    end
end

# Using interfaces
let shapes: Drawable[] = [
    new Circle(5.0, "red"),
    new Rectangle(10.0, 6.0, "blue")
];

for shape in shapes
    print(shape.draw());  # Polymorphic call
end
```

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
    public fn deposit(amount: Float) -> Bool
        if this.validate_amount(amount)
            this.balance = this.balance + amount;
            this.log_transaction("Deposit", amount);
            return true;
        end
        false
    end

    # Public method
    public fn withdraw(amount: Float) -> Bool
        if this.validate_amount(amount) && this.has_sufficient_funds(amount)
            this.balance = this.balance - amount;
            this.log_transaction("Withdrawal", -amount);
            return true;
        end
        false
    end

    # Public getter
    public fn get_balance() -> Float
        this.balance
    end

    # Private method - internal helper
    private fn validate_amount(amount: Float) -> Bool
        amount > 0
    end

    # Private method
    private fn has_sufficient_funds(amount: Float) -> Bool
        this.balance >= amount
    end

    # Private method
    private fn log_transaction(type: String, amount: Float)
        # Internal logging logic
    end

    # Protected method - for subclasses
    protected fn update_status(new_status: String)
        this.status = new_status;
    end
end

# Using the class
let account = new BankAccount("123456789", 1000.0);

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
    static fn square(x: Float) -> Float
        MathUtils.calculation_count = MathUtils.calculation_count + 1;
        x * x
    end

    static fn cube(x: Float) -> Float
        MathUtils.calculation_count = MathUtils.calculation_count + 1;
        x * x * x
    end

    static fn max(a: Float, b: Float) -> Float
        a > b ? a : b
    end

    static fn min(a: Float, b: Float) -> Float
        a < b ? a : b
    end

    static fn clamp(value: Float, min_val: Float, max_val: Float) -> Float
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

let result = MathUtils.clamp(150, 0, 100);
print(result);  # 100

print(MathUtils.calculation_count);  # 3
```

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

    fn get_total_value() -> Float
        this.price * this.quantity
    end

    fn is_in_stock() -> Bool
        this.quantity > 0
    end

    fn reduce_quantity(amount: Int) -> Bool
        if this.quantity >= amount
            this.quantity = this.quantity - amount;
            return true;
        end
        false
    }

    fn to_string() -> String {
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

    fn is_under_warranty() -> Bool
        this.warranty_months > 12
    end

    fn to_string() -> String
        this.brand + " " + this.name + " - $" + str(this.price) + " (" + str(this.warranty_months) + " month warranty)"
    end
end

# Inventory class to manage products
class Inventory
    products: Product[];

    new()
        this.products = [];
    end

    fn add_product(product: Product)
        push(this.products, product);
    end

    fn remove_product(product_id: String) -> Product?
        for i, product in this.products
            if product.id == product_id
                return splice(this.products, i, 1)[0];
            end
        end
        null
    end

    fn find_product(id: String) -> Product?
        for product in this.products
            if product.id == id
                return product;
            end
        end
        null
    end

    fn get_total_inventory_value() -> Float
        let total = 0.0;
        for product in this.products
            total = total + product.get_total_value();
        end
        total
    end

    fn get_out_of_stock_products() -> Product[]
        this.products.filter(fn(p) !p.is_in_stock())
    end

    fn list_all()
        for product in this.products
            print(product.to_string());
        end
    end
end

# Using the inventory system
let inventory = new Inventory();

# Add products
inventory.add_product(new Product("P001", "Laptop", 999.99, 10));
inventory.add_product(new Electronics("E001", "Headphones", 149.99, 50, "AudioTech", 24));
inventory.add_product(new Product("P002", "Mouse", 29.99, 100));

# Work with inventory
print("Total inventory value: $" + str(inventory.get_total_inventory_value()));

let laptop = inventory.find_product("P001");
if laptop != null
    print("Found: " + laptop.to_string());
end

inventory.list_all();
```

### Nested Classes

Soli supports nested classes - classes defined within other classes. This feature is useful for organizing related classes, implementing design patterns, and creating clean namespaces.

```soli
class Organization
    class Department
        fn get_name()
            "Engineering"
        end

        fn get_budget()
            1000000
        end
    end

    class Team
        fn get_name()
            "Backend Team"
        end
    end
end
```

#### Accessing Nested Classes

Use the `::` (scope resolution operator) to access nested classes:

```soli
let dept = new Organization::Department();
print("Department: " + dept.get_name());  # "Department: Engineering"
print("Budget: $" + str(dept.get_budget()));  # "Budget: $1000000"

let team = new Organization::Team();
print("Team: " + team.get_name());  # "Team: Backend Team"
```

#### Use Cases

**1. Design Patterns**

Nested classes are perfect for implementing design patterns:

```soli
# State Pattern
class TrafficLight
    class RedState
        fn next()
            "green"
        end

        fn get_duration()
            30
        end
    end

    class GreenState
        fn next()
            "yellow"
        end

        fn get_duration()
            20
        end
    end

    class YellowState
        fn next()
            "red"
        end

        fn get_duration()
            5
        end
    end
end

let red = new TrafficLight::RedState();
print("Red light duration: " + str(red.get_duration()) + "s");  # "Red light duration: 30s"
print("Next state: " + red.next());  # "Next state: green"
```

**2. Organization and Encapsulation**

Group related classes together:

```soli
class Database
    class Connection
        fn connect()
            "Connected to database"
        end
    end

    class QueryBuilder
        fn select(table: String)
            "SELECT * FROM " + table
        end
    end

    class Transaction
        fn begin()
            "Transaction started"
        end
    end
end

let conn = new Database::Connection();
let query = new Database::QueryBuilder();
let tx = new Database::Transaction();

print(conn.connect());  # "Connected to database"
print(query.select("users"));  # "SELECT * FROM users"
print(tx.begin());  # "Transaction started"
```

**3. Configuration Objects**

Create hierarchical configuration structures:

```soli
class Server
    class SSLConfig
        fn is_enabled()
            true
        end

        fn get_protocol()
            "TLS 1.3"
        end
    end

    class LoggingConfig
        fn get_level()
            "INFO"
        end
    end

    fn start()
        "Server starting with SSL: " + str(new Server::SSLConfig().is_enabled())
    end
end

let ssl = new Server::SSLConfig();
print("Protocol: " + ssl.get_protocol());  # "Protocol: TLS 1.3"
```

#### Multiple Nested Classes

You can define multiple nested classes at the same level:

```soli
class Service
    class Database
        fn connect()
            "DB connected"
        end
    end

    class Cache
        fn get(key: String)
            "cached:" + key
        end
    end

    class Logger
        fn log(msg: String)
            "[LOG] " + msg
        end
    end
end

let db = new Service::Database();
let cache = new Service::Cache();
let logger = new Service::Logger();

print(db.connect());  # "DB connected"
print(cache.get("test"));  # "cached:test"
print(logger.log("test message"));  # "[LOG] test message"
```

---

## Pattern Matching

### Basic Pattern Matching

```soli
# Simple value matching
let x = 42;
let result = match x {
    42 => "the answer to everything",
    0 => "zero",
    _ => "something else",
};
print(result);  # "the answer to everything"

# String matching
let status = "active";
let status_message = match status {
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
let n = 5;
let category = match n {
    n if n < 0 => "negative",
    0 => "zero",
    n if n > 0 && n < 10 => "single digit positive",
    n if n >= 10 && n < 100 => "two digit positive",
    _ => "large number",
};
print(category);  # "single digit positive"

# Practical example: HTTP status handling
fn handle_status(code: Int) -> String {
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
let numbers = [1, 2, 3];

# Match array length
let description = match numbers {
    [] => "empty array",
    [_] => "single element array",
    [_, _] => "two element array",
    [_, _, _] => "three element array",
    _ => "array with more than 3 elements",
};

# Destructuring arrays
let result = match numbers {
    [first] => "First element is: " + str(first),
    [first, second] => "First: " + str(first) + ", Second: " + str(second),
    [first, second, third] => "Three elements: " + str(first) + ", " + str(second) + ", " + str(third),
    [first, ...rest] => "First: " + str(first) + ", Rest has " + str(len(rest)) + " elements",
};

# Rest pattern
let arr = [1, 2, 3, 4, 5];
match arr {
    [first, second, ...rest] => {
        print("First two: " + str(first) + ", " + str(second));
        print("Remaining: " + str(rest));  # [3, 4, 5]
    },
};
```

### Hash Patterns

```soli
let user = {"name": "Alice", "age": 30, "city": "Paris"};

# Match hash structure
match user {
    {} => "empty object",
    {name: n} => "name is: " + n,
    {name: n, age: a} => n + " is " + str(a) + " years old",
    {name: n, age: a, city: c} => n + " is " + str(a) + " years old and lives in " + c,
    _ => "unknown structure",
};

# Nested hash matching
let data = {
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

fn describe_value(val: Any) -> String {
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
fn handle_json_value(value: Any) -> String {
    match value {
        null => "null",
        s: String => "\"" + s + "\"",
        n: Int => str(n),
        n: Float => str(n),
        true => "true",
        false => "false",
        arr: Array => "[" + join(arr.map(fn(x) handle_json_value(x)), ", ") + "]",
        h: Hash => "{" + join(h.entries().map(fn(pair) {
            let k = pair[0];
            let v = pair[1];
            "\"" + k + "\": " + handle_json_value(v)
        }), ", ") + "}",
        _ => "\"unknown\"",
    }
}
```

### Advanced Pattern Matching Examples

```soli
# Command pattern matching
fn execute_command(command: Hash) -> String {
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
fn process_tree(node: Any) {
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

## Pipeline Operator

### Basic Pipeline Usage

The pipeline operator `|>` passes the left value as the first argument to the right function:

```soli
fn double(x: Int) -> Int { x * 2 }
fn add_one(x: Int) -> Int { x + 1 }
fn square(x: Int) -> Int { x * x }

# Without pipeline (nested calls)
let result1 = square(add_one(double(5)));  # Hard to read

# With pipeline (left to right)
let result2 = 5 |> double() |> add_one() |> square();
print(result2);  # (5 * 2 + 1)^2 = 121
```

### Pipeline with Multiple Arguments

```soli
fn add(a: Int, b: Int) -> Int { a + b }
fn multiply(a: Int, b: Int) -> Int { a * b }

# 5 |> add(3) means add(5, 3)
let result = 5 |> add(3) |> multiply(2);
print(result);  # (5 + 3) * 2 = 16

# More complex chaining
fn subtract(a: Int, b: Int) -> Int { a - b }
fn divide(a: Int, b: Int) -> Int { int(a / b) }

let calc = 100
    |> fn(x) { subtract(x, 10) }()
    |> fn(x) { divide(x, 3) }()
    |> fn(x) { multiply(x, 4) }();
print(calc);  # ((100 - 10) / 3) * 4 = 120
```

### Pipeline with Collection Methods

```soli
let numbers = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];

# Method chaining with pipeline
let result = numbers
    .filter(fn(x) x % 2 == 0)
    .map(fn(x) x * x)
    .each(fn(x) print(x));

# Equivalent using pipeline
let result2 = numbers
    |> filter(fn(x) x % 2 == 0)
    |> map(fn(x) x * x)
    |> each(fn(x) print(x));
```

### Real-World Pipeline Examples

```soli
# Data processing pipeline
fn process_user_data(raw_data: Hash) -> Hash {
    raw_data
        |> fn(d) { d["sanitized_email"] = d["email"].lower().trim(); d }()
        |> validate_user()
        |> enrich_profile()
        |> calculate_metrics()
}

# HTTP request pipeline
fn fetch_and_process(url: String) -> Hash {
    url
        |> http_get_json()
        |> transform_response()
        |> validate_data()
        |> format_output()
}

# String transformation pipeline
fn format_filename(filename: String) -> String {
    filename
        |> fn(s) { s.lower() }()
        |> fn(s) { s.replace(" ", "_") }()
        |> fn(s) { s.replace("-", "_") }()
        |> fn(s) { s + ".txt" }()
}

print(format_filename("My Document"));  # "my_document.txt"
```

### Inline Transformations

```soli
# Using inline functions for transformations
let numbers = [1, 2, 3, 4, 5];

let result = numbers
    |> map(fn(x) x * 2)
    |> filter(fn(x) x > 4)
    |> reduce(fn(acc, x) acc + x, 0);

print(result);  # 18 (6+8+10)

# Fetch and process user with async pipeline
fn get_user_data(user_id: Int) -> Hash {
    user_id
        |> fn(id) { {"id": id} }()
        |> fetch_from_db()
        |> enrich_with_permissions()
        |> cache_result()
}

# Complex data pipeline
let sales_data = [
    {"product": "A", "quantity": 10, "price": 100},
    {"product": "B", "quantity": 5, "price": 200},
    {"product": "C", "quantity": 15, "price": 50},
];

let total_revenue = sales_data
    |> map(fn(sale) sale["quantity"] * sale["price"])
    |> reduce(fn(acc, rev) acc + rev, 0);

print("Total Revenue: $" + str(total_revenue));  # $2750
```

---

## Modules

### Creating Modules

```soli
# math.sl - A module exporting utility functions

# Private function (not exported)
fn validate_number(n: Int) -> Bool {
    n >= 0
}

# Exported functions
export fn add(a: Int, b: Int) -> Int {
    a + b
}

export fn subtract(a: Int, b: Int) -> Int {
    a - b
}

export fn multiply(a: Int, b: Int) -> Int {
    a * b
}

export fn divide(a: Int, b: Int) -> Float {
    if (b == 0) {
        panic("Division by zero");
    }
    float(a) / float(b)
}

export fn factorial(n: Int) -> Int {
    if (n <= 1) {
        return 1;
    }
    n * factorial(n - 1)
}

export fn fibonacci(n: Int) -> Int {
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

let sum = add(1, 2);          # 3
let product = multiply(3, 4); # 12

# Aliased imports - import with different names
import { add as sum, multiply as times } from "./math.sl";

let result = sum(10, 20);  # 30
let doubled = times(5, 6); # 30

# Import everything with a namespace
import "./utils.sl" as utils;

let formatted = utils.format_date(DateTime.utc());
let cleaned = utils.sanitize_input(user_input);
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

fn main() {
    let config = load_config();
    let processed = utils.process_data(config);
    let result = math.calculate(processed);
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
let name = input("Enter your name: ");
print("Hello, " + name + "!");

# Input with default
let age_str = input("Enter age: ", "18");
let age = int(age_str);
```

### Type Conversion

```soli
let num_str = "42";
let float_str = "3.14";

# String conversions
let num = int(num_str);        # 42
let f = float(float_str);      # 3.14
let s = str(123);              # "123"
let s2 = str(3.14);            # "3.14"
let s3 = str([1, 2, 3]);       # "[1, 2, 3]"

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

# Length
print(len(arr));  # 5

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
# Basic math
print(abs(-5));        # 5
print(min(3, 7));      # 3
print(max(3, 7));      # 7

# Power and roots
print(sqrt(16));       # 4.0
print(pow(2, 10));     # 1024.0
print(pow(27, 1/3));   # 3.0 (cube root)

# Trigonometry
print(sin(0));         # 0.0
print(cos(0));         # 1.0
print(tan(0.785398));  # ~1.0 (45 degrees in radians)

# Rounding
print(floor(3.7));     # 3
print(ceil(3.2));      # 4
print(round(3.5));     # 4

# Random
let random_num = random();        # 0.0 to 1.0
let random_int = random_int(1, 100);  # 1 to 100

# Time
print(clock());  # Current time in seconds since epoch
```

### Hash Functions

```soli
let person = {"name": "Alice", "age": 30, "city": "Paris"};

# Keys and values
let keys_list = keys(person);   # ["name", "age", "city"]
let values_list = values(person);  # ["Alice", 30, "Paris"]

# Check existence
print(has_key(person, "name"));   # true
print(has_key(person, "email"));  # false

# Merge
let additional = {"country": "France", "email": "alice@example.com"};
let merged = merge(person, additional);

# Delete
let deleted = delete(person, "age");
print(deleted);  # 30
print(person);   # {name: Alice, city: Paris}

# Entries
let entries_list = entries(person);  # [["name", "Alice"], ["age", 30], ["city", "Paris"]]

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
let response = http_get("https://api.example.com/data");
print(response["status"]);  # 200
print(response["body"]);    # Response body

# GET JSON and parse automatically
let json_data = http_get_json("https://api.example.com/users");
print(json_data[0]["name"]);

# POST request
let post_response = http_post("https://api.example.com/submit", {"key": "value"});
print(post_response["body"]);

# Generic request with options
let custom_request = http_request("DELETE", "https://api.example.com/resource/123", null, {
    "headers": {"Authorization": "Bearer token123"},
    "timeout": 30,
});
```

### JSON Functions

```soli
# Parse JSON string
let json_str = '{"name": "Alice", "age": 30, "scores": [95, 87, 92]}';
let parsed = json_parse(json_str);

print(parsed["name"]);   # "Alice"
print(parsed["scores"]); # [95, 87, 92]

# Convert to JSON
let data = {"users": [{"name": "Alice"}, {"name": "Bob"}], "count": 2};
let json_string = json_stringify(data);
# '{"users":[{"name":"Alice"},{"name":"Bob"}],"count":2}'
```

### Regex Class

```soli
let text = "The quick brown fox jumps over the lazy dog";

# Match check
let has_fox = Regex.matches("fox", text);  # true

# Find matches
let first_word = Regex.find("\\w+", text);  # "The"
let all_words = Regex.find_all("\\w+", text);  # ["The", "quick", "brown", "fox", ...]

# Replace
let replaced = Regex.replace("fox", text, "cat");  # "The quick brown cat..."
let all_replaced = Regex.replace_all("\\s+", text, "-");  # "The-quick-brown-fox-..."

# Split
let words = Regex.split("\\s+", text);  # ["The", "quick", "brown", "fox", ...]

# Capture groups
let date = "2024-01-15";
let captures = Regex.capture("(\\d{4})-(\\d{2})-(\\d{2})", date);
print(captures["match"]);  # "2024-01-15"
print(captures[0]);  # "2024" (when using numbered groups)
print(captures[1]);  # "01"
print(captures[2]);  # "15"

# Escape special characters
let escaped = Regex.escape("file.txt (1).pdf");
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
let user_input = "<script>alert('xss')</script>";
let escaped = html_escape(user_input);
# "&lt;script&gt;alert(&#39;xss&#39;)&lt;/script&gt;"

# Unescape HTML entities
let html = "&lt;div&gt;Content&lt;/div&gt;";
let unescaped = html_unescape(html);  # "<div>Content</div>"

# Sanitize HTML (remove dangerous tags)
let raw_html = "<div><script>evil()</script><p>Safe content</p></div>";
let safe = sanitize_html(raw_html);
# "<div><p>Safe content</p></div>"
```

---

## DateTime & Duration

### Creating DateTime Instances

```soli
# Current local time
let now = DateTime.utc();
print(now.to_string());  # "2024-01-15 10:30:00"

# Parse from ISO 8601 string
let parsed = DateTime.parse("2024-01-15T10:30:00");
print(parsed.year());    # 2024
print(parsed.month());   # 1
print(parsed.day());     # 15

# Parse with timezone
let with_tz = DateTime.parse("2024-01-15T10:30:00+05:00");

# Create from Unix timestamp
let from_timestamp = DateTime.from_unix(1705315800);
```

### DateTime Methods

```soli
let dt = DateTime.parse("2024-01-15T10:30:45");

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
let unix_ts = dt.to_unix();           # 1705315845
let iso_str = dt.to_iso();            # "2024-01-15T10:30:45"
let human = dt.to_string();           # "2024-01-15 10:30:45"

# Formatting
let custom = dt.format("YYYY-MM-DD");  # "2024-01-15"
let time_only = dt.format("HH:mm:ss"); # "10:30:45"
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
let dur1 = Duration.days(7);           # 7 days
let dur2 = Duration.hours(24);         # 24 hours (same as 1 day)
let dur3 = Duration.minutes(90);       # 90 minutes
let dur4 = Duration.seconds(3600);     # 3600 seconds (1 hour)

# Duration between two DateTimes
let dt1 = DateTime.parse("2024-01-01T00:00:00");
let dt2 = DateTime.parse("2024-01-02T12:00:00");
let between = Duration.between(dt1, dt2);

# Duration methods
print(between.total_seconds());  # 108000.0
print(between.total_minutes());  # 1800.0
print(between.total_hours());    # 30.0
print(between.total_days());     # 1.25
print(between.to_string());      # "1 day, 6 hours"
```

### Practical DateTime Examples

```soli
# Calculate age from birthdate
fn calculate_age(birthdate: DateTime) -> Int {
    let now = DateTime.utc();
    let age = now.year() - birthdate.year();
    if (now.month() < birthdate.month() ||
        (now.month() == birthdate.month() && now.day() < birthdate.day())) {
        age = age - 1;
    }
    age
}

let birthdate = DateTime.parse("1990-05-15");
print(calculate_age(birthdate));  # e.g., 33

# Format relative time
fn relative_time(dt: DateTime) -> String {
    let now = DateTime.utc();
    let diff = now.sub(dt);

    let seconds = diff.total_seconds();
    if (seconds < 60) {
        return "just now";
    }
    if (seconds < 3600) {
        let mins = int(seconds / 60);
        return str(mins) + (mins == 1 ? " minute ago" : " minutes ago");
    }
    if (seconds < 86400) {
        let hours = int(seconds / 3600);
        return str(hours) + (hours == 1 ? " hour ago" : " hours ago");
    }
    if (seconds < 604800) {
        let days = int(seconds / 86400);
        return str(days) + (days == 1 ? " day ago" : " days ago");
    }
    dt.to_string()
}

# Check if date is in the past
fn is_past(dt: DateTime) -> Bool {
    let now = DateTime.utc();
    dt.to_unix() < now.to_unix()
}

# Get start of day
fn start_of_day(dt: DateTime) -> DateTime {
    DateTime.parse(
        str(dt.year()) + "-" +
        str(dt.month()) + "-" +
        str(dt.day()) + "T00:00:00"
    )
}

# Get business days between two dates
fn business_days(start: DateTime, end: DateTime) -> Int {
    let count = 0;
    let current = start;
    while (current.to_unix() <= end.to_unix()) {
        let weekday = current.weekday();
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

### Editor Integration

The VS Code / Cursor extension (`editors/vscode/`) runs `soli lint` automatically on save and displays warnings inline in the editor.

---

## Best Practices

### Variables & Types

```soli
# Good: Use type inference when obvious
let count = 10;
let name = "Alice";

# Good: Add annotations for public API or complex types
pub fn process_user(user_id: Int) -> User {
    # ...
}

# Good: Use meaningful names
let items_per_page = 25;
let max_retry_attempts = 3;

# Avoid: Single-letter names except for loop variables
let c = 10;           # Bad
let item_count = 10;  # Good
```

### Functions

```soli
# Good: Single responsibility
fn validate_email(email: String) -> Bool {
    email.contains("@") && email.contains(".")
}

# Good: Descriptive names
fn calculate_total_with_tax() -> Float {
    # ...
}

# Good: Limit parameters
fn create_user(info: Hash) -> User {
    # Instead of: fn create_user(name, email, age, address, phone)
}

# Good: Early returns for validation
fn process_order(order: Hash) -> Result {
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
let weekdays = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday"];

# Good: Check bounds
fn safe_get(arr: Array, index: Int) -> Any? {
    if (index >= 0 && index < len(arr)) {
        return arr[index];
    }
    null
}

# Good: Use functional methods for transformations
let doubled = numbers.map(fn(x) x * 2);
let evens = numbers.filter(fn(x) x % 2 == 0);
```

### Classes

```soli
# Good: Single responsibility
class User {}
class UserRepository {}
class UserService {}

# Good: Program to interfaces
interface Repository {
    fn find(id: Int) -> Any?;
    fn save(entity: Any);
}

# Good: Use private fields for encapsulation
class BankAccount {
    private balance: Float;

    public fn deposit(amount: Float) {
        # ...
    }

    public fn get_balance() -> Float {
        this.balance
    }
}
```

### Control Flow

```soli
# Good: Avoid deep nesting
fn process(data: Hash) -> Result {
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
fn handle_event(event: Hash) -> String {
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
let name = "Alice";
let age: Int = 30;
const PI = 3.14159;
```

### Functions
```soli
fn add(a: Int, b: Int) -> Int {
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
    fn greet() -> String {
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
let arr = [1, 2, 3];
let hash = {"key": "value"};

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
export fn add(a: Int, b: Int) -> Int {
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
