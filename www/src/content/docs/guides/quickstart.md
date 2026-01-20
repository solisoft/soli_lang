---
title: Quick Start
description: Write your first Soli program
---

# Quick Start

Let's write your first Soli program! This guide will walk you through the basics.

## Hello World

Create a file called `hello.soli`:

```solilang
print("Hello, Soli!");
```

Run it:

```bash
soli hello.soli
```

Output:
```
Hello, Soli!
```

## Variables and Types

Soli has several built-in types:

```solilang
// Integers
let age = 25;
let count: Int = 100;

// Floating point numbers
let pi = 3.14159;
let temperature: Float = 98.6;

// Strings
let name = "Alice";
let greeting: String = "Hello";

// Booleans
let isReady = true;
let isDone: Bool = false;
```

## Basic Operations

```solilang
// Arithmetic
let sum = 10 + 5;      // 15
let diff = 10 - 5;     // 5
let product = 10 * 5;  // 50
let quotient = 10 / 5; // 2
let remainder = 10 % 3; // 1

// String concatenation
let fullName = "John" + " " + "Doe";
print(fullName);  // John Doe

// Comparisons
let isEqual = 5 == 5;     // true
let isGreater = 10 > 5;   // true
let isLessOrEq = 5 <= 5;  // true
```

## Functions

Define reusable functions:

```solilang
fn greet(name: String) -> String {
    return "Hello, " + name + "!";
}

fn add(a: Int, b: Int) -> Int {
    return a + b;
}

print(greet("World"));  // Hello, World!
print(add(3, 4));       // 7
```

## Pipeline Operator

The pipeline operator `|>` chains function calls elegantly:

```solilang
fn double(x: Int) -> Int {
    return x * 2;
}

fn addTen(x: Int) -> Int {
    return x + 10;
}

// Traditional way
let result1 = addTen(double(5));

// With pipeline - reads left to right!
let result2 = 5 |> double() |> addTen();

print(result1);  // 20
print(result2);  // 20
```

## Control Flow

### If/Else

```solilang
let score = 85;

if (score >= 90) {
    print("A");
} else if (score >= 80) {
    print("B");
} else {
    print("C");
}
```

### While Loop

```solilang
let i = 0;
while (i < 5) {
    print(i);
    i = i + 1;
}
```

### For Loop

```solilang
let numbers = [1, 2, 3, 4, 5];
for (n in numbers) {
    print(n);
}

// Using range
for (i in range(0, 5)) {
    print(i);
}
```

## Arrays

```solilang
let fruits = ["apple", "banana", "cherry"];

print(fruits[0]);      // apple
print(len(fruits));    // 3

fruits[1] = "blueberry";
push(fruits, "date");

for (fruit in fruits) {
    print(fruit);
}
```

## Classes

```solilang
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

    fn perimeter() -> Float {
        return 2.0 * (this.width + this.height);
    }
}

let rect = new Rectangle(10.0, 5.0);
print("Area:", rect.area());           // Area: 50
print("Perimeter:", rect.perimeter()); // Perimeter: 30
```

## Complete Example

Here's FizzBuzz in Soli:

```solilang
fn fizzbuzz(n: Int) -> String {
    if (n % 15 == 0) {
        return "FizzBuzz";
    }
    if (n % 3 == 0) {
        return "Fizz";
    }
    if (n % 5 == 0) {
        return "Buzz";
    }
    return str(n);
}

let i = 1;
while (i <= 20) {
    print(fizzbuzz(i));
    i = i + 1;
}
```

## Next Steps

- Learn more about [Variables & Types](/guides/variables/)
- Explore [Functions](/guides/functions/) in depth
- Master the [Pipeline Operator](/guides/pipeline/)
- Dive into [Classes & OOP](/guides/classes/)
