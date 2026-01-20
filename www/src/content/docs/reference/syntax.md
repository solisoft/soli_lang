---
title: Syntax Reference
description: Complete syntax reference for Soli
---

# Syntax Reference

This page provides a complete reference for Soli's syntax.

## Comments

```rust
// Single-line comment

/*
   Multi-line
   comment
*/

/* Nested /* comments */ are supported */
```

## Literals

### Numbers

```rust
42          // Integer
-17         // Negative integer
1_000_000   // Underscores for readability

3.14        // Float
-0.5        // Negative float
2.5e10      // Scientific notation
```

### Strings

```rust
"Hello, World!"
"Line 1\nLine 2"   // Newline
"Tab\there"        // Tab
"Quote: \"hi\""    // Escaped quote
"Path: C:\\Users"  // Escaped backslash
```

### Booleans

```rust
true
false
```

### Null

```rust
null
```

### Arrays

```rust
[1, 2, 3]
["a", "b", "c"]
[]  // Empty array
```

### Hashes

Both fat arrow (`=>`) and colon (`:`) syntax are supported:

```rust
// Fat arrow syntax
{"name" => "Alice", "age" => 30}
{1 => "one", 2 => "two"}

// JSON-style colon syntax
{"name": "Alice", "age": 30}
{"nested": {"key": "value"}, "array": [1, 2, 3]}

{}  // Empty hash
```

## Variables

```rust
// Declaration with inference
let name = "Alice";
let age = 30;

// With type annotation
let count: Int = 0;
let pi: Float = 3.14159;

// Assignment
name = "Bob";
count = count + 1;
```

## Operators

### Arithmetic

| Operator | Description |
|----------|-------------|
| `+` | Addition |
| `-` | Subtraction |
| `*` | Multiplication |
| `/` | Division |
| `%` | Modulo (remainder) |

### Comparison

| Operator | Description |
|----------|-------------|
| `==` | Equal |
| `!=` | Not equal |
| `<` | Less than |
| `<=` | Less than or equal |
| `>` | Greater than |
| `>=` | Greater than or equal |

### Logical

| Operator | Description |
|----------|-------------|
| `&&` | Logical AND |
| `\|\|` | Logical OR |
| `!` | Logical NOT |

### Pipeline

```rust
|>   // Pipeline operator
```

### Assignment

```rust
=    // Assignment
```

## Control Flow

### If/Else

```rust
if (condition) {
    // then branch
}

if (condition) {
    // then branch
} else {
    // else branch
}

if (condition1) {
    // ...
} else if (condition2) {
    // ...
} else {
    // ...
}
```

### While Loop

```rust
while (condition) {
    // body
}
```

### For Loop

```rust
for (item in collection) {
    // body
}

for (i in range(0, 10)) {
    // body
}
```

### Pattern Matching

```rust
match expression {
    pattern => expression,
    pattern if condition => expression,
    _ => expression,  // wildcard
}
```

**Patterns:**

```rust
// Literal patterns
match value {
    42 => "the answer",
    "hello" => "a greeting",
    true => "truthy",
    null => "nothing",
}

// Variable patterns
match value {
    n => str(n),  // binds n to value
}

// Wildcard pattern
match value {
    _ => "matches anything",
}

// Array patterns
match arr {
    [] => "empty",
    [x] => "single: " + str(x),
    [first, second, ...rest] => "first two: " + str(first) + ", " + str(second),
}

// Hash patterns
match obj {
    {} => "empty",
    {name: n} => "name: " + n,
    {name: n, age: a} => n + " is " + str(a),
}

// Guard clauses
match value {
    n if n > 0 => "positive",
    n if n < 0 => "negative",
    0 => "zero",
}
```

## Functions

### Declaration

```rust
fn functionName(param1: Type1, param2: Type2) -> ReturnType {
    // body
    return value;
}

// No return type (void)
fn noReturn(x: Int) {
    print(x);
}

// No parameters
fn greet() -> String {
    return "Hello!";
}
```

### Calling

```rust
functionName(arg1, arg2)
noReturn(42)
let result = greet()
```

### Pipeline Calls

```rust
value |> function()
value |> function(arg2, arg3)
```

## Classes

### Class Declaration

```rust
class ClassName {
    // Fields
    fieldName: Type;

    // Constructor
    new(param: Type) {
        this.fieldName = param;
    }

    // Methods
    fn methodName() -> ReturnType {
        return this.fieldName;
    }
}
```

### Inheritance

```rust
class Child extends Parent {
    // ...
}
```

### Interfaces

```rust
interface InterfaceName {
    fn methodSignature(param: Type) -> ReturnType;
}

class MyClass implements InterfaceName {
    fn methodSignature(param: Type) -> ReturnType {
        // implementation
    }
}

// Multiple interfaces
class MyClass implements Interface1, Interface2 {
    // ...
}
```

### Visibility

```rust
class Example {
    public field1: Int;
    private field2: String;
    protected field3: Float;

    public fn method1() { }
    private fn method2() { }
    protected fn method3() { }
}
```

### Static Members

```rust
class Utils {
    static value: Int = 42;

    static fn helper() -> Int {
        return Utils.value;
    }
}
```

### Instantiation

```rust
let obj = new ClassName(arg1, arg2);
```

### Member Access

```rust
obj.field
obj.method()
ClassName.staticMethod()
```

## Types

### Primitive Types

| Type | Description |
|------|-------------|
| `Int` | 64-bit integer |
| `Float` | 64-bit float |
| `String` | UTF-8 string |
| `Bool` | Boolean |
| `Void` | No value |
| `Future` | Async result (auto-resolves when used) |

### Array Types

```rust
Int[]      // Array of integers
String[]   // Array of strings
Type[]     // Array of Type
```

### Function Types

```rust
(Int) -> Int           // One param, returns Int
(Int, Int) -> Int      // Two params
() -> String           // No params
(String) -> Void       // No return
```

## Blocks

```rust
{
    // statements
}
```

Blocks create new scopes:

```rust
let x = 1;
{
    let x = 2;  // Different x
    print(x);   // 2
}
print(x);       // 1
```

## Statements

### Expression Statement

```rust
expression;
```

### Return Statement

```rust
return;
return value;
```

## Keywords

Reserved words that cannot be used as identifiers:

| | | | |
|---|---|---|---|
| `let` | `fn` | `return` | `if` |
| `else` | `while` | `for` | `in` |
| `match` | `class` | `extends` | `implements` |
| `interface` | `new` | `this` | `super` |
| `public` | `private` | `protected` | `static` |
| `true` | `false` | `null` | |

## Type Keywords

| | | | |
|---|---|---|---|
| `Int` | `Float` | `Bool` | `String` |
| `Void` | | | |

## Operator Precedence

From lowest to highest:

1. `=` (assignment)
2. `||` (logical or)
3. `&&` (logical and)
4. `==`, `!=` (equality)
5. `<`, `<=`, `>`, `>=` (comparison)
6. `|>` (pipeline)
7. `+`, `-` (addition, subtraction)
8. `*`, `/`, `%` (multiplication, division, modulo)
9. `!`, `-` (unary not, negate)
10. `.`, `()`, `[]` (member access, call, index)
