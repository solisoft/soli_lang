---
title: Control Flow
description: Conditionals and loops in Soli
---

# Control Flow

Soli provides familiar control flow constructs for conditional execution and iteration.

## If/Else

### Basic If

```rust
let age = 18;

if (age >= 18) {
    print("Adult");
}
```

### If/Else

```rust
let score = 75;

if (score >= 60) {
    print("Pass");
} else {
    print("Fail");
}
```

### If/Else If/Else

```rust
let grade = 85;

if (grade >= 90) {
    print("A");
} else if (grade >= 80) {
    print("B");
} else if (grade >= 70) {
    print("C");
} else if (grade >= 60) {
    print("D");
} else {
    print("F");
}
```

### Nested If

```rust
let x = 10;
let y = 20;

if (x > 0) {
    if (y > 0) {
        print("Both positive");
    }
}
```

## Comparison Operators

| Operator | Meaning |
|----------|---------|
| `==` | Equal to |
| `!=` | Not equal to |
| `<` | Less than |
| `<=` | Less than or equal |
| `>` | Greater than |
| `>=` | Greater than or equal |

## Logical Operators

### And (&&)

Both conditions must be true:

```rust
let age = 25;
let hasLicense = true;

if (age >= 18 && hasLicense) {
    print("Can drive");
}
```

### Or (||)

At least one condition must be true:

```rust
let isWeekend = true;
let isHoliday = false;

if (isWeekend || isHoliday) {
    print("Day off!");
}
```

### Not (!)

Inverts a boolean:

```rust
let isRaining = false;

if (!isRaining) {
    print("No umbrella needed");
}
```

### Combining Operators

```rust
let age = 25;
let hasTicket = true;
let isVIP = false;

if ((age >= 18 && hasTicket) || isVIP) {
    print("Entry allowed");
}
```

## Pattern Matching

Pattern matching provides a powerful way to destructure and match against values. It's an expression that evaluates one of several arms based on the input value.

### Basic Match

```rust
let x = 42;
let result = match x {
    42 => "the answer",
    _ => "something else",
};
print(result);  // "the answer"
```

### Variable Binding

Patterns can bind values to variables:

```rust
let value = 10;
match value {
    n => "got: " + str(n),
};
```

### Literal Patterns

Match against specific literal values:

```rust
let status = "active";
match status {
    "active" => "User is active",
    "pending" => "Awaiting approval",
    "banned" => "Access denied",
    _ => "Unknown status",
};
```

### Guard Clauses

Add conditions to patterns with `if`:

```rust
let n = 5;
match n {
    n if n > 0 => "positive",
    n if n < 0 => "negative",
    0 => "zero",
};
```

### Array Patterns

Destructure arrays with patterns:

```rust
let numbers = [1, 2, 3];
match numbers {
    [] => "empty array",
    [first] => "single element: " + str(first),
    [first, second] => "two elements: " + str(first) + " and " + str(second),
    [first, second, ...rest] => "first two: " + str(first) + ", " + str(second),
};
```

> **Note:** Array and hash destructuring patterns are currently supported in the interpreter. Full bytecode compilation support is planned for a future release.

### Hash Patterns

Destructure hashes with patterns:

```rust
let user = {"name": "Alice", "age": 30};
match user {
    {} => "empty object",
    {name: n} => "name is: " + n,
    {name: n, age: a} => n + " is " + str(a) + " years old",
};
```

### Nested Patterns

Combine patterns for nested data:

```rust
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

### Multiple Arms

Match against multiple conditions:

```rust
let httpCode = 404;
let message = match httpCode {
    200 => "OK",
    201 => "Created",
    400 => "Bad Request",
    401 => "Unauthorized",
    403 => "Forbidden",
    404 => "Not Found",
    500 => "Internal Server Error",
    _ => "Unknown status code",
};
```

### Use Cases

**Type-based dispatch:**

```rust
let value: Any = getSomeValue();
match value {
    s: String => "Got a string: " + s,
    n: Int => "Got an integer: " + str(n),
    b: Bool => "Got a boolean: " + str(b),
    _ => "Unknown type",
};
```

**Result processing:**

```rust
fn processResult(result: Result) -> String {
    return match result {
        {status: "success", data: d} => "Data: " + str(d),
        {status: "error", message: m} => "Error: " + m,
        _ => "Invalid result format",
    };
}
```

**State machines:**

```rust
let state = "authenticated";
let response = match state {
    "login" => "Showing login form",
    "authenticated" => "Welcome back!",
    "guest" => "Browse as guest",
    _ => "Unknown state",
};
```

## While Loop

Repeats while a condition is true:

```rust
let i = 0;
while (i < 5) {
    print(i);
    i = i + 1;
}
// Output: 0, 1, 2, 3, 4
```

### Infinite Loop (with break logic)

```rust
let count = 0;
while (true) {
    print(count);
    count = count + 1;
    if (count >= 3) {
        // In practice, you'd break here
        // For now, we use a condition
    }
}
```

### Loop with Complex Condition

```rust
let attempts = 0;
let maxAttempts = 3;
let success = false;

while (attempts < maxAttempts && !success) {
    print("Attempt " + str(attempts + 1));
    // Try something...
    attempts = attempts + 1;
}
```

## For Loop

Iterate over collections:

### Over Arrays

```rust
let fruits = ["apple", "banana", "cherry"];

for (fruit in fruits) {
    print(fruit);
}
```

### With Range

```rust
// Print 0 to 4
for (i in range(0, 5)) {
    print(i);
}

// Print 1 to 10
for (i in range(1, 11)) {
    print(i);
}
```

### Nested Loops

```rust
for (i in range(1, 4)) {
    for (j in range(1, 4)) {
        print(str(i) + " x " + str(j) + " = " + str(i * j));
    }
}
```

## Common Patterns

### Counting

```rust
let count = 0;
for (item in items) {
    if (condition(item)) {
        count = count + 1;
    }
}
```

### Accumulating

```rust
let sum = 0;
for (n in range(1, 101)) {
    sum = sum + n;
}
print(sum);  // 5050
```

### Finding

```rust
let numbers = [4, 8, 15, 16, 23, 42];
let found = false;
let target = 15;

let i = 0;
while (i < len(numbers) && !found) {
    if (numbers[i] == target) {
        found = true;
        print("Found at index " + str(i));
    }
    i = i + 1;
}
```

### FizzBuzz Example

```rust
let i = 1;
while (i <= 20) {
    if (i % 15 == 0) {
        print("FizzBuzz");
    } else if (i % 3 == 0) {
        print("Fizz");
    } else if (i % 5 == 0) {
        print("Buzz");
    } else {
        print(i);
    }
    i = i + 1;
}
```

## Truthiness

In conditions, these values are "falsy":

- `false`
- `null`
- `0` (integer zero)
- `""` (empty string)
- `[]` (empty array)

Everything else is "truthy":

```rust
if ("hello") {
    print("Non-empty string is truthy");
}

if ([1, 2, 3]) {
    print("Non-empty array is truthy");
}
```

## Best Practices

1. **Avoid deep nesting**: Extract complex conditions into functions
   ```rust
   // Instead of deeply nested if/else
   fn canProceed(age: Int, hasPermission: Bool) -> Bool {
       return age >= 18 && hasPermission;
   }

   if (canProceed(age, hasPermission)) {
       // ...
   }
   ```

2. **Use early returns**: Simplify logic flow
   ```rust
   fn processItem(item: Item) {
       if (!isValid(item)) {
           return;
       }
       // Main logic here
   }
   ```

3. **Prefer for-in over while** for iteration when possible
   ```rust
   // Clearer
   for (item in items) {
       process(item);
   }

   // More error-prone
   let i = 0;
   while (i < len(items)) {
       process(items[i]);
       i = i + 1;
   }
   ```

## Next Steps

- Learn about [Functions](/guides/functions/)
- Explore [Arrays](/guides/arrays/)
- Dive into [Classes & OOP](/guides/classes/)
