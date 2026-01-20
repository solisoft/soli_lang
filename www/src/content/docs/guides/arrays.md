---
title: Arrays
description: Working with arrays in Soli
---

# Arrays

Arrays in Soli store ordered collections of values of the same type.

## Creating Arrays

### Array Literals

```rust
let numbers = [1, 2, 3, 4, 5];
let names = ["Alice", "Bob", "Charlie"];
let flags = [true, false, true];
```

### With Type Annotation

```rust
let scores: Int[] = [95, 87, 92];
let words: String[] = [];  // Empty array
```

### Empty Arrays

```rust
let items: Int[] = [];
let messages: String[] = [];
```

## Accessing Elements

### By Index

Arrays are zero-indexed:

```rust
let fruits = ["apple", "banana", "cherry"];

print(fruits[0]);  // apple
print(fruits[1]);  // banana
print(fruits[2]);  // cherry
```

### Negative Indices

Access from the end:

```rust
let numbers = [10, 20, 30, 40, 50];

print(numbers[-1]);  // 50 (last)
print(numbers[-2]);  // 40 (second to last)
```

## Modifying Arrays

### Update Elements

```rust
let colors = ["red", "green", "blue"];
colors[1] = "yellow";
print(colors);  // [red, yellow, blue]
```

### Add Elements

```rust
let stack = [1, 2, 3];
push(stack, 4);
push(stack, 5);
print(stack);  // [1, 2, 3, 4, 5]
```

### Remove Elements

```rust
let queue = [1, 2, 3, 4];
let last = pop(queue);
print(last);   // 4
print(queue);  // [1, 2, 3]
```

## Array Properties

### Length

```rust
let items = [10, 20, 30, 40];
print(len(items));  // 4
```

### Empty Check

```rust
let items: Int[] = [];
if (len(items) == 0) {
    print("Array is empty");
}
```

## Iterating Arrays

### For Loop

```rust
let numbers = [1, 2, 3, 4, 5];

for (n in numbers) {
    print(n);
}
```

### With Index

```rust
let fruits = ["apple", "banana", "cherry"];

let i = 0;
while (i < len(fruits)) {
    print(str(i) + ": " + fruits[i]);
    i = i + 1;
}
// Output:
// 0: apple
// 1: banana
// 2: cherry
```

## Iteration Methods

Arrays have built-in methods for functional-style iteration: `map`, `filter`, and `each`.

### `map` - Transform Elements

Transform each element and return a new array:

```rust
let numbers = [1, 2, 3, 4, 5];

// Expression body (implicit return)
let doubled = numbers.map(fn(x) x * 2);
print(doubled);  // [2, 4, 6, 8, 10]

// Block body (explicit return)
let squares = numbers.map(fn(x) {
    return x * x;
});
print(squares);  // [1, 4, 9, 16, 25]
```

### `filter` - Select Elements

Keep elements where the function returns truthy:

```rust
let numbers = [1, 2, 3, 4, 5];

// Expression syntax
let evens = numbers.filter(fn(x) x % 2 == 0);
print(evens);  // [2, 4]

// Block syntax
let large = numbers.filter(fn(n) {
    return n > 3;
});
print(large);  // [4, 5]
```

### `each` - Side Effects

Execute a function for each element, returns the original array:

```rust
let numbers = [1, 2, 3];

numbers.each(fn(x) print(x));
// Prints: 1, 2, 3
// Returns: [1, 2, 3]
```

### Chaining Methods

Chain multiple operations together:

```rust
let numbers = [1, 2, 3, 4, 5];

let result = numbers
    .map(fn(x) x * 2)
    .filter(fn(x) x > 5);

print(result);  // [6, 8, 10]
```

## Common Operations

### Sum

```rust
fn sum(numbers: Int[]) -> Int {
    let total = 0;
    for (n in numbers) {
        total = total + n;
    }
    return total;
}

print(sum([1, 2, 3, 4, 5]));  // 15
```

### Find Maximum

```rust
fn maximum(numbers: Int[]) -> Int {
    let maxVal = numbers[0];
    for (n in numbers) {
        if (n > maxVal) {
            maxVal = n;
        }
    }
    return maxVal;
}

print(maximum([3, 1, 4, 1, 5, 9]));  // 9
```

### Filter

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

let nums = [-1, 2, -3, 4, -5, 6];
print(filterPositive(nums));  // [2, 4, 6]
```

### Map

```rust
fn doubleAll(numbers: Int[]) -> Int[] {
    let result: Int[] = [];
    for (n in numbers) {
        push(result, n * 2);
    }
    return result;
}

print(doubleAll([1, 2, 3]));  // [2, 4, 6]
```

### Contains

```rust
fn contains(arr: Int[], target: Int) -> Bool {
    for (item in arr) {
        if (item == target) {
            return true;
        }
    }
    return false;
}

let nums = [1, 2, 3, 4, 5];
print(contains(nums, 3));  // true
print(contains(nums, 9));  // false
```

## Using Range

Create arrays of sequential numbers:

```rust
let nums = range(0, 5);   // [0, 1, 2, 3, 4]
let nums2 = range(1, 6);  // [1, 2, 3, 4, 5]

// Useful for iteration
for (i in range(0, 10)) {
    print(i);
}
```

## Nested Arrays

Arrays of arrays:

```rust
let matrix = [
    [1, 2, 3],
    [4, 5, 6],
    [7, 8, 9]
];

print(matrix[0][0]);  // 1
print(matrix[1][2]);  // 6

// Iterate nested arrays
for (row in matrix) {
    for (val in row) {
        print(val);
    }
}
```

## Array Algorithms

### Reverse

```rust
fn reverse(arr: Int[]) -> Int[] {
    let result: Int[] = [];
    let i = len(arr) - 1;
    while (i >= 0) {
        push(result, arr[i]);
        i = i - 1;
    }
    return result;
}

print(reverse([1, 2, 3, 4, 5]));  // [5, 4, 3, 2, 1]
```

### Count Occurrences

```rust
fn count(arr: Int[], target: Int) -> Int {
    let total = 0;
    for (item in arr) {
        if (item == target) {
            total = total + 1;
        }
    }
    return total;
}

let votes = [1, 2, 1, 1, 3, 2, 1];
print(count(votes, 1));  // 4
```

### Remove Duplicates

```rust
fn unique(arr: Int[]) -> Int[] {
    let result: Int[] = [];
    for (item in arr) {
        let found = false;
        for (r in result) {
            if (r == item) {
                found = true;
            }
        }
        if (!found) {
            push(result, item);
        }
    }
    return result;
}

print(unique([1, 2, 2, 3, 1, 4]));  // [1, 2, 3, 4]
```

## Pipeline with Arrays

Arrays work great with pipelines:

```rust
fn double(n: Int) -> Int { return n * 2; }

fn processArray(arr: Int[]) -> Int[] {
    let result: Int[] = [];
    for (n in arr) {
        push(result, n |> double());
    }
    return result;
}

fn sumArray(arr: Int[]) -> Int {
    let total = 0;
    for (n in arr) {
        total = total + n;
    }
    return total;
}

let result = [1, 2, 3, 4, 5] |> processArray() |> sumArray();
print(result);  // 30
```

## Array Iteration Methods Summary

| Method | Callback | Returns | Description |
|--------|----------|---------|-------------|
| `arr.map(fn)` | `fn(element)` | Array | Transform each element |
| `arr.filter(fn)` | `fn(element)` | Array | Keep elements where callback returns truthy |
| `arr.each(fn)` | `fn(element)` | Array | Execute for side effects |

### Expression vs Block Syntax

Both syntaxes work for all iteration methods:

```rust
// Expression body (implicit return)
arr.map(fn(x) x * 2)

// Block body (explicit return)
arr.map(fn(x) {
    return x * 2;
})
```

## Best Practices

1. **Initialize with known values** when possible
2. **Check bounds** before accessing by index
3. **Use `for-in`** for simple iteration
4. **Use `while` with index** when you need the position
5. **Prefer immutable patterns** - create new arrays rather than modifying

## Next Steps

- Learn about [Classes & OOP](/guides/classes/)
- Master the [Pipeline Operator](/guides/pipeline/)
- See [Built-in Functions](/reference/builtins/)
