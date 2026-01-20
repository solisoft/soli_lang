---
title: Variables & Types
description: Learn about Soli's type system and variable declarations
---

# Variables & Types

Soli is a statically-typed language with type inference. This means you get the safety of static types without always having to write them explicitly.

## Variable Declaration

Variables are declared using the `let` keyword:

```rust
let name = "Alice";
let age = 30;
let temperature = 98.6;
```

### With Type Annotations

You can explicitly specify types:

```rust
let name: String = "Alice";
let age: Int = 30;
let temperature: Float = 98.6;
let isActive: Bool = true;
```

## Primitive Types

### Int

64-bit signed integers:

```rust
let x = 42;
let negative = -100;
let big = 9_000_000;  // Underscores for readability
```

### Float

64-bit floating-point numbers:

```rust
let pi = 3.14159;
let small = 0.001;
let scientific = 2.5e10;
```

### String

UTF-8 strings:

```rust
let greeting = "Hello, World!";
let multiword = "This is a sentence.";

// Escape sequences
let newline = "Line 1\nLine 2";
let tab = "Column1\tColumn2";
let quote = "She said \"Hello\"";
```

### Bool

Boolean values:

```rust
let yes = true;
let no = false;
```

### Null

The absence of a value:

```rust
let nothing = null;
```

## Type Inference

Soli infers types when possible:

```rust
let x = 5;          // Int
let y = 3.14;       // Float
let z = "hello";    // String
let flag = true;    // Bool
let nums = [1, 2];  // Int[]
```

## Type Coercion

Some automatic conversions happen:

```rust
// Int to Float in mixed arithmetic
let result = 5 + 3.0;  // result is Float: 8.0

// Any type to String with concatenation
let msg = "Value: " + 42;  // "Value: 42"
```

## Arrays

Arrays hold multiple values of the same type:

```rust
let numbers: Int[] = [1, 2, 3, 4, 5];
let names = ["Alice", "Bob", "Charlie"];  // String[] inferred

// Access elements
print(numbers[0]);  // 1
print(names[2]);    // Charlie

// Modify elements
numbers[0] = 10;
```

## Assignment

Variables can be reassigned:

```rust
let x = 5;
x = 10;        // OK
x = x + 1;     // OK

// But types must match
let y: Int = 5;
// y = "hello";  // Error: type mismatch
```

## Scope

Variables are block-scoped:

```rust
let x = 1;

if (true) {
    let y = 2;      // y only visible in this block
    let x = 3;      // Shadows outer x
    print(x);       // 3
}

print(x);           // 1
// print(y);        // Error: y not in scope
```

## Constants

While Soli doesn't have a `const` keyword, you can use naming conventions:

```rust
let PI = 3.14159;
let MAX_SIZE = 100;
```

## Best Practices

1. **Use type inference** when the type is obvious:
   ```rust
   let count = 0;           // Clear it's an Int
   let name = "Alice";      // Clear it's a String
   ```

2. **Add annotations** when clarity helps:
   ```rust
   let result: Float = calculate();  // Return type unclear
   ```

3. **Use meaningful names**:
   ```rust
   let userAge = 25;        // Good
   let x = 25;              // Less clear
   ```

## Next Steps

- Learn about [Functions](/guides/functions/)
- Explore [Control Flow](/guides/control-flow/)
- See how types work with [Classes](/guides/classes/)
