---
title: Introduction
description: Learn about Soli and its key features
---

# Introduction to Soli

Soli is a modern, statically-typed programming language designed for clarity and expressiveness. It combines object-oriented programming with functional concepts like the pipeline operator.

## Key Features

### Static Typing with Inference

Soli is statically typed, meaning type errors are caught at compile time. However, you don't always need to write types explicitly—the compiler can infer them:

```solilang
let x = 42;           // Type inferred as Int
let name = "Alice";   // Type inferred as String
let pi: Float = 3.14; // Explicit type annotation
```

### Pipeline Operator

The pipeline operator `|>` is one of Soli's most distinctive features. It lets you chain function calls in a readable, left-to-right manner:

```solilang
// Without pipeline
let result = addOne(double(5));

// With pipeline - much clearer!
let result = 5 |> double() |> addOne();
```

The left side of `|>` becomes the first argument of the function on the right.

### Object-Oriented Programming

Soli supports full OOP with classes, inheritance, and interfaces:

```solilang
interface Drawable {
    fn draw() -> String;
}

class Shape {
    x: Float;
    y: Float;

    new(x: Float, y: Float) {
        this.x = x;
        this.y = y;
    }
}

class Circle extends Shape implements Drawable {
    radius: Float;

    new(x: Float, y: Float, radius: Float) {
        this.x = x;
        this.y = y;
        this.radius = radius;
    }

    fn draw() -> String {
        return "Circle at (" + str(this.x) + ", " + str(this.y) + ")";
    }
}
```

## Design Philosophy

Soli follows these principles:

1. **Clarity over cleverness**: Code should be easy to read and understand
2. **Safety without verbosity**: Static typing shouldn't mean excessive boilerplate
3. **Familiar syntax**: Draw from popular languages to minimize learning curve
4. **Practical features**: Include features that solve real problems (like pipelines)

## Use Cases

Soli is great for:

- Learning programming concepts
- Scripting and automation
- Data transformation pipelines
- Teaching OOP and type systems
- Prototyping ideas quickly
- Building internationalized applications
- Full-stack web development with Soli MVC
- Real-time applications with WebSocket support

## Standard Library Features

Soli includes practical features for real-world development:

- **Date & Time**: Comprehensive datetime support with `DateTime` and `Duration` classes
- **Internationalization**: Translation, pluralization, and locale-aware formatting
- **HTTP Client & Server**: Built-in HTTP functions and web server capabilities
- **WebSockets**: Real-time bidirectional communication for chat, notifications, and live updates
- **Database**: SoliDB integration for document storage
- **File I/O**: Read/write files with text and binary support
- **Cryptography**: Secure password hashing with Argon2

## Next Steps

Ready to get started? Head to the [Installation](/guides/installation/) guide to set up Soli on your machine.

Want to learn specific features? Check out:
- [Date & Time](/guides/datetime/) - Working with dates and times
- [Internationalization](/guides/internationalization/) - Building multi-language applications
- [WebSockets](/guides/websocket/) - Real-time bidirectional communication
