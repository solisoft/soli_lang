---
title: Pipeline Operator
description: Master Soli's distinctive pipeline operator
---

# Pipeline Operator

The pipeline operator `|>` is one of Soli's most powerful features. It transforms how you write and read function calls.

## Basic Concept

The pipeline operator takes the value on its left and passes it as the **first argument** to the function on its right.

```rust
// These are equivalent:
let result1 = double(5);
let result2 = 5 |> double();
```

## Why Use Pipelines?

### Without Pipeline

Traditional nested function calls read inside-out:

```rust
// What's the order of operations here?
let result = addOne(double(square(5)));

// You have to read from inside out:
// 1. square(5) = 25
// 2. double(25) = 50
// 3. addOne(50) = 51
```

### With Pipeline

Pipelines read left-to-right, like natural language:

```rust
// Much clearer! Read left to right:
let result = 5 |> square() |> double() |> addOne();

// 5 → square → double → addOne
// 5 →   25   →   50   →   51
```

## Function Definitions

Define functions that work well with pipelines:

```rust
fn double(x: Int) -> Int {
    return x * 2;
}

fn addOne(x: Int) -> Int {
    return x + 1;
}

fn square(x: Int) -> Int {
    return x * x;
}

// Use them in a pipeline
let result = 10 |> double() |> addOne() |> square();
print(result);  // ((10 * 2) + 1)² = 441
```

## Multiple Arguments

When a function has multiple parameters, the piped value becomes the **first** argument:

```rust
fn add(a: Int, b: Int) -> Int {
    return a + b;
}

fn multiply(a: Int, b: Int) -> Int {
    return a * b;
}

// 5 |> add(3) means add(5, 3)
let result = 5 |> add(3) |> multiply(2);
// Step 1: add(5, 3) = 8
// Step 2: multiply(8, 2) = 16
print(result);  // 16
```

## Chaining Operations

Build complex transformations step by step:

```rust
fn increment(x: Int) -> Int { return x + 1; }
fn double(x: Int) -> Int { return x * 2; }
fn negate(x: Int) -> Int { return -x; }

// Chain multiple operations
let result = 3
    |> increment()    // 4
    |> double()       // 8
    |> increment()    // 9
    |> negate();      // -9

print(result);  // -9
```

## String Pipelines

Works great with string operations:

```rust
fn greet(name: String) -> String {
    return "Hello, " + name + "!";
}

fn exclaim(text: String) -> String {
    return text + "!!";
}

fn uppercase(text: String) -> String {
    // In practice, you'd have a built-in for this
    return text;  // Placeholder
}

let message = "World" |> greet() |> exclaim();
print(message);  // Hello, World!!!
```

## Data Processing

Pipelines excel at data transformation:

```rust
fn filterPositive(numbers: Int[]) -> Int[] {
    let result: Int[] = [];
    for (n in numbers) {
        if (n > 0) {
            push(result, n);
        }
    }
    return result;
}

fn doubleAll(numbers: Int[]) -> Int[] {
    let result: Int[] = [];
    for (n in numbers) {
        push(result, n * 2);
    }
    return result;
}

fn sumAll(numbers: Int[]) -> Int {
    let total = 0;
    for (n in numbers) {
        total = total + n;
    }
    return total;
}

// Process data through a pipeline
let data = [-1, 2, -3, 4, 5];
let result = data |> filterPositive() |> doubleAll() |> sumAll();
print(result);  // (2 + 4 + 5) * 2 = 22
```

## Combining with Methods

Pipelines work with method calls too:

```rust
class Calculator {
    value: Int;

    new(initial: Int) {
        this.value = initial;
    }

    fn add(x: Int) -> Calculator {
        this.value = this.value + x;
        return this;
    }

    fn multiply(x: Int) -> Calculator {
        this.value = this.value * x;
        return this;
    }
}

fn createCalculator(initial: Int) -> Calculator {
    return new Calculator(initial);
}

fn getValue(calc: Calculator) -> Int {
    return calc.value;
}

// Mix functions and method chains
let result = 5
    |> createCalculator()
    |> getValue();
```

## Real-World Example

Here's a more complex example processing user data:

```rust
class User {
    name: String;
    age: Int;
    active: Bool;

    new(name: String, age: Int, active: Bool) {
        this.name = name;
        this.age = age;
        this.active = active;
    }
}

fn filterActive(users: User[]) -> User[] {
    let result: User[] = [];
    for (user in users) {
        if (user.active) {
            push(result, user);
        }
    }
    return result;
}

fn filterAdults(users: User[]) -> User[] {
    let result: User[] = [];
    for (user in users) {
        if (user.age >= 18) {
            push(result, user);
        }
    }
    return result;
}

fn getNames(users: User[]) -> String[] {
    let result: String[] = [];
    for (user in users) {
        push(result, user.name);
    }
    return result;
}

// Find names of active adult users
let users = [
    new User("Alice", 25, true),
    new User("Bob", 17, true),
    new User("Charlie", 30, false),
    new User("Diana", 22, true)
];

let names = users |> filterActive() |> filterAdults() |> getNames();
// names = ["Alice", "Diana"]
```

## Best Practices

### 1. Design Pipeline-Friendly Functions

Put the "data" parameter first:

```rust
// Good - data first
fn transform(data: Int, factor: Int) -> Int {
    return data * factor;
}

5 |> transform(2);  // Works naturally

// Less ideal - data last
fn transformAlt(factor: Int, data: Int) -> Int {
    return data * factor;
}
// Can't use pipeline easily
```

### 2. Keep Pipelines Readable

Break long pipelines across lines:

```rust
let result = data
    |> step1()
    |> step2()
    |> step3()
    |> step4();
```

### 3. Use Meaningful Names

Pipeline steps should read like a sentence:

```rust
let activeUserCount = users
    |> filterByStatus("active")
    |> filterByRole("admin")
    |> count();
```

## When to Use Pipelines

**Good use cases:**
- Sequential data transformations
- Function composition
- Replacing deeply nested calls
- Making data flow explicit

**Consider alternatives when:**
- Operations aren't sequential
- Side effects are involved
- The transformation is trivial

## Next Steps

- Practice with [Arrays](/guides/arrays/)
- See [Built-in Functions](/reference/builtins/)
- Explore [Classes & OOP](/guides/classes/)
