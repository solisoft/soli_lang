---
title: Functions
description: Defining and using functions in Soli
---

# Functions

Functions are the building blocks of Soli programs. They let you organize code into reusable units.

## Basic Syntax

```rust
fn functionName(param1: Type1, param2: Type2) -> ReturnType {
    // function body
    return value;
}
```

## Simple Examples

### No Parameters, No Return

```rust
fn sayHello() {
    print("Hello!");
}

sayHello();  // Hello!
```

### With Parameters

```rust
fn greet(name: String) {
    print("Hello, " + name + "!");
}

greet("Alice");  // Hello, Alice!
```

### With Return Value

```rust
fn add(a: Int, b: Int) -> Int {
    return a + b;
}

let sum = add(3, 4);
print(sum);  // 7
```

## Return Types

### Explicit Return

```rust
fn square(x: Int) -> Int {
    return x * x;
}
```

### Void Functions

Functions without a return type (or returning `Void`) don't return a value:

```rust
fn logMessage(msg: String) {
    print("[LOG] " + msg);
    // implicit return
}
```

### Early Return

```rust
fn absolute(x: Int) -> Int {
    if (x < 0) {
        return -x;
    }
    return x;
}
```

## Function Calls

### Basic Calls

```rust
let result = add(5, 3);
print(multiply(2, 4));
```

### Nested Calls

```rust
let value = add(multiply(2, 3), 4);  // (2 * 3) + 4 = 10
```

### With Pipeline Operator

```rust
fn double(x: Int) -> Int { return x * 2; }
fn addOne(x: Int) -> Int { return x + 1; }

let result = 5 |> double() |> addOne();  // 11
```

## Multiple Parameters

```rust
fn createGreeting(name: String, age: Int) -> String {
    return "Hello, " + name + "! You are " + str(age) + " years old.";
}

print(createGreeting("Bob", 25));
```

## Recursive Functions

Functions can call themselves:

```rust
fn factorial(n: Int) -> Int {
    if (n <= 1) {
        return 1;
    }
    return n * factorial(n - 1);
}

print(factorial(5));  // 120
```

```rust
fn fibonacci(n: Int) -> Int {
    if (n <= 1) {
        return n;
    }
    return fibonacci(n - 1) + fibonacci(n - 2);
}

print(fibonacci(10));  // 55
```

## Higher-Order Functions

Functions can be passed to other functions and stored in variables:

```rust
fn apply(x: Int, f: (Int) -> Int) -> Int {
    return f(x);
}

fn double(x: Int) -> Int {
    return x * 2;
}

let result = apply(5, double);  // 10
```

## Functions with Multiple Arguments for Pipeline

The pipeline operator passes the left value as the first argument:

```rust
fn add(a: Int, b: Int) -> Int {
    return a + b;
}

fn multiply(a: Int, b: Int) -> Int {
    return a * b;
}

// 5 |> add(3) means add(5, 3)
let result = 5 |> add(3) |> multiply(2);  // (5 + 3) * 2 = 16
```

## Built-in Functions

Soli provides several built-in functions:

| Function | Description |
|----------|-------------|
| `print(...)` | Print values to stdout |
| `len(x)` | Get length of array or string |
| `str(x)` | Convert to string |
| `int(x)` | Convert to integer |
| `float(x)` | Convert to float |
| `type(x)` | Get type name as string |
| `push(arr, val)` | Add element to array |
| `pop(arr)` | Remove and return last element |
| `range(start, end)` | Create array from start to end-1 |
| `abs(x)` | Absolute value |
| `min(a, b)` | Minimum of two values |
| `max(a, b)` | Maximum of two values |
| `sqrt(x)` | Square root |
| `pow(base, exp)` | Exponentiation |
| `clock()` | Current time in seconds |

## Best Practices

1. **Single responsibility**: Each function should do one thing well
   ```rust
   // Good
   fn calculateTax(amount: Float) -> Float { ... }
   fn formatCurrency(amount: Float) -> String { ... }

   // Less good
   fn calculateAndFormatTax(amount: Float) -> String { ... }
   ```

2. **Descriptive names**: Use verbs for actions
   ```rust
   fn getUserName() -> String { ... }
   fn validateEmail(email: String) -> Bool { ... }
   ```

3. **Limit parameters**: Consider using objects for many parameters
   ```rust
   // Instead of many parameters
   fn createUser(name: String, age: Int, email: String, ...) { ... }

   // Consider a class
   class UserData { ... }
   fn createUser(data: UserData) { ... }
   ```

## Next Steps

- Learn about [Control Flow](/guides/control-flow/)
- Master the [Pipeline Operator](/guides/pipeline/)
- Explore [Classes & OOP](/guides/classes/)
