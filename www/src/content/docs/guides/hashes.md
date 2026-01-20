---
title: Hashes
description: Working with hashes (dictionaries) in Soli
---

# Hashes

Hashes in Soli are ordered key-value collections, similar to Ruby's Hash or Python's dict. They preserve insertion order and allow any hashable value as a key.

## Creating Hashes

### Hash Literals

Soli supports two syntaxes for hash literals:

**JSON-style colon syntax** (recommended for JSON-like data):

```rust
let person = {
    "name": "Alice",
    "age": 30,
    "city": "New York"
};

let config = {
    "host": "localhost",
    "port": 8080,
    "debug": true
};
```

**Ruby-style hash rocket syntax** (`=>`):

```rust
let scores = {
    "Alice" => 95,
    "Bob" => 87,
    "Charlie" => 92
};
```

:::tip[Choose Your Style]
Both syntaxes are equivalent. Use `:` for JSON-like data structures and `=>` when you prefer Ruby-style syntax. You can even mix them (though consistency is recommended).
:::

### Empty Hash

```rust
let empty = {};
print(len(empty));  // 0
```

### Numeric Keys

Keys can be any hashable type (Int, Float, String, Bool):

```rust
let lookup = {
    1 => "one",
    2 => "two",
    3 => "three"
};

print(lookup[2]);  // two
```

## Accessing Values

### By Key

```rust
let colors = {
    "red": "#FF0000",
    "green": "#00FF00",
    "blue": "#0000FF"
};

print(colors["red"]);    // #FF0000
print(colors["green"]);  // #00FF00
```

### Missing Keys

Accessing a missing key returns `null`:

```rust
let hash = {"a": 1};
print(hash["b"]);  // null
```

## Modifying Hashes

### Adding/Updating Values

```rust
let person = {"name": "Alice"};

// Add new key
person["age"] = 30;

// Update existing key
person["name"] = "Alicia";

print(person);  // {name: Alicia, age: 30}
```

### Deleting Keys

```rust
let scores = {"Alice" => 95, "Bob" => 87};
let removed = delete(scores, "Bob");

print(removed);  // 87
print(scores);   // {Alice => 95}
```

### Clearing

```rust
let data = {"x": 1, "y": 2};
clear(data);
print(data);  // {}
```

## Hash Functions

### len

Returns the number of key-value pairs:

```rust
let hash = {"a": 1, "b": 2, "c": 3};
print(len(hash));  // 3
```

### keys

Returns an array of all keys (in insertion order):

```rust
let person = {"name": "Alice", "age": 30};
print(keys(person));  // [name, age]
```

### values

Returns an array of all values (in insertion order):

```rust
let person = {"name": "Alice", "age": 30};
print(values(person));  // [Alice, 30]
```

### has_key

Check if a key exists:

```rust
let scores = {"Alice" => 95, "Bob" => 87};

print(has_key(scores, "Alice"));  // true
print(has_key(scores, "Carol"));  // false
```

### delete

Remove a key and return its value (or `null` if not found):

```rust
let hash = {"a": 1, "b": 2};
let val = delete(hash, "a");

print(val);   // 1
print(hash);  // {b: 2}
```

### merge

Combine two hashes (second hash's values win on conflicts):

```rust
let h1 = {"a": 1, "b": 2};
let h2 = {"b": 3, "c": 4};

let merged = merge(h1, h2);
print(merged);  // {a: 1, b: 3, c: 4}
```

### entries

Get an array of `[key, value]` pairs:

```rust
let colors = {"red": "#FF0000", "green": "#00FF00"};
print(entries(colors));  // [[red, #FF0000], [green, #00FF00]]
```

### clear

Remove all entries from a hash:

```rust
let data = {"x": 1, "y": 2};
clear(data);
print(data);  // {}
```

## Iterating Over Hashes

Use `entries()` with a for loop to iterate:

```rust
let prices = {
    "apple": 1.50,
    "banana": 0.75,
    "orange": 2.00
};

for (pair in entries(prices)) {
    let item = pair[0];
    let price = pair[1];
    print(item + " costs $" + str(price));
}
// Output:
// apple costs $1.5
// banana costs $0.75
// orange costs $2
```

### Iterating Keys

```rust
let scores = {"Alice" => 95, "Bob" => 87};

for (name in keys(scores)) {
    print(name + ": " + str(scores[name]));
}
```

### Iterating Values

```rust
let scores = {"Alice" => 95, "Bob" => 87};
let total = 0;

for (score in values(scores)) {
    total = total + score;
}

print("Total: " + str(total));  // Total: 182
```

## Iteration Methods

Hashes have built-in methods for functional-style iteration: `map`, `filter`, and `each`. These methods pass a `[key, value]` array to the callback function.

### `map` - Transform Entries

Transform each entry and return a new hash. The callback must return `[new_key, new_value]`:

```rust
let scores = {"Alice": 90, "Bob": 85, "Charlie": 95};

// Add 10 points to each score
let curved = scores.map(fn(pair) {
    let key = pair[0];
    let value = pair[1];
    return [key, value + 10];
});
print(curved);  // {Alice: 100, Bob: 95, Charlie: 105}

// Or more concisely:
let doubled = scores.map(fn(pair) [pair[0], pair[1] * 2]);
```

### `filter` - Select Entries

Keep entries where the callback returns truthy:

```rust
let scores = {"Alice": 90, "Bob": 85, "Charlie": 95};

// Keep only passing scores (>= 90)
let passing = scores.filter(fn(pair) {
    return pair[1] >= 90;
});
print(passing);  // {Alice: 90, Charlie: 95}
```

### `each` - Side Effects

Execute a function for each entry, returns the original hash:

```rust
let user = {"name": "Alice", "age": 30, "city": "Paris"};

user.each(fn(pair) {
    print(pair[0] + ": " + pair[1]);
});
// Prints:
// name: Alice
// age: 30
// city: Paris
```

### Chaining with Arrays

Combine hash methods with array methods:

```rust
let scores = {"Alice": 90, "Bob": 85, "Charlie": 95};

// Get names of people with score >= 90
let names = scores
    .filter(fn(pair) pair[1] >= 90)
    .map(fn(pair) pair[0]);

print(names);  // [Alice, Charlie]
```

## Common Patterns

### Default Values

```rust
fn get_or_default(hash: Any, key: Any, default: Any) -> Any {
    if (has_key(hash, key)) {
        return hash[key];
    }
    return default;
}

let config = {"timeout": 30};
print(get_or_default(config, "timeout", 60));  // 30
print(get_or_default(config, "retries", 3));   // 3
```

### Counting Occurrences

```rust
fn count_items(items: String[]) -> Any {
    let counts = {};
    for (item in items) {
        if (has_key(counts, item)) {
            counts[item] = counts[item] + 1;
        } else {
            counts[item] = 1;
        }
    }
    return counts;
}

let fruits = ["apple", "banana", "apple", "orange", "banana", "apple"];
print(count_items(fruits));  // {apple => 3, banana => 2, orange => 1}
```

### Grouping by Property

```rust
fn group_by_length(words: String[]) -> Any {
    let groups = {};
    for (word in words) {
        let length = len(word);
        if (!has_key(groups, length)) {
            groups[length] = [];
        }
        push(groups[length], word);
    }
    return groups;
}

let words = ["cat", "dog", "bird", "fish", "ant"];
print(group_by_length(words));
// {3 => [cat, dog, ant], 4 => [bird, fish]}
```

### Inverting a Hash

```rust
fn invert(hash: Any) -> Any {
    let result = {};
    for (pair in entries(hash)) {
        result[pair[1]] = pair[0];
    }
    return result;
}

let colors = {"red": "#FF0000", "green": "#00FF00"};
print(invert(colors));  // {#FF0000: red, #00FF00: green}
```

## Pipeline with Hashes

Hashes work well with the pipeline operator:

```rust
fn get_keys(h: Any) -> Any {
    return keys(h);
}

fn first(arr: Any) -> Any {
    return arr[0];
}

let data = {"first": 100, "second": 200};
let first_key = data |> get_keys() |> first();
print(first_key);  // first
```

## Hash Summary

| Function | Parameters | Returns | Description |
|----------|-----------|---------|-------------|
| `len(h)` | Hash | Int | Number of entries |
| `keys(h)` | Hash | Array | All keys |
| `values(h)` | Hash | Array | All values |
| `has_key(h, k)` | Hash, Any | Bool | Check if key exists |
| `delete(h, k)` | Hash, Any | Any | Remove and return value |
| `merge(h1, h2)` | Hash, Hash | Hash | Combine hashes |
| `entries(h)` | Hash | Array | Array of [key, value] pairs |
| `clear(h)` | Hash | Void | Remove all entries |
| `h.map(fn)` | Hash, Function | Hash | Transform entries |
| `h.filter(fn)` | Hash, Function | Hash | Select entries |
| `h.each(fn)` | Hash, Function | Hash | Iterate with side effects |

## Best Practices

1. **Use meaningful keys**: Choose descriptive string keys for readability
2. **Check before access**: Use `has_key()` when unsure if a key exists
3. **Preserve order**: Hashes maintain insertion order, use this for predictable iteration
4. **Prefer immutable patterns**: Use `merge()` to create new hashes rather than modifying in place when possible

## Next Steps

- Learn about [Arrays](/guides/arrays/)
- Master the [Pipeline Operator](/guides/pipeline/)
- See [Built-in Functions](/reference/builtins/)
