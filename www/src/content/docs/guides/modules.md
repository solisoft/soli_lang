---
title: Modules & Packages
description: Organize your code with modules and packages
---

# Modules & Packages

Soli supports a module system that allows you to organize your code across multiple files and reuse functionality.

## Export Declarations

Use `export` to make functions, classes, interfaces, or variables available to other modules:

```solilang
// math.soli

export fn add(a: Int, b: Int) -> Int {
    return a + b;
}

export fn multiply(a: Int, b: Int) -> Int {
    return a * b;
}

export class Calculator {
    fn calculate(x: Int) -> Int {
        return x * 2;
    }
}

// Private - not exported
fn internal_helper(x: Int) -> Int {
    return x + 1;
}
```

## Import Statements

### Import All Exports

Import everything exported from a module:

```solilang
import "./math.soli";

// Now add, multiply, and Calculator are available
let result = add(2, 3);
```

### Named Imports

Import specific items from a module:

```solilang
import { add, multiply } from "./math.soli";

let sum = add(1, 2);
let product = multiply(3, 4);
```

### Aliased Imports

Rename imports to avoid conflicts or for clarity:

```solilang
import { add as sum, multiply as mul } from "./math.soli";

let result = sum(1, 2);
```

### Namespace Imports

Import all exports under a namespace (note: currently behaves like import all):

```solilang
import * as math from "./math.soli";

// TODO: Namespace access not yet implemented
// let result = math.add(1, 2);
```

## Module Resolution

Soli resolves module paths in the following order:

1. **Relative paths** (starting with `.` or `..`):
   - `./module.soli` - Same directory
   - `../lib/utils.soli` - Parent directory
   - `./lib/index.soli` - Subdirectory

2. **Package dependencies** (from `soli.toml`):
   - Named dependencies defined in your package file

3. **Absolute paths** from base directory

### File Resolution

When resolving a path, Soli looks for:

1. Exact file path
2. Path with `.soli` extension added
3. Directory with `index.soli`
4. Directory with `mod.soli`

```solilang
// All equivalent if ./lib/math.soli exists
import "./lib/math.soli";
import "./lib/math";

// If ./lib/utils/index.soli exists
import "./lib/utils";
```

## Package Files

Create a `soli.toml` file in your project root to configure your package:

```toml
[package]
name = "my-app"
version = "1.0.0"
description = "My awesome Soli application"
main = "src/main.soli"

[dependencies]
utils = { path = "./lib/utils" }
math = { path = "../shared/math" }
```

### Package Fields

| Field | Description |
|-------|-------------|
| `name` | Package name (required) |
| `version` | Semantic version |
| `description` | Package description |
| `main` | Entry point file |

### Dependencies

Dependencies can be specified as:

- **Path dependencies**: Local file paths
  ```toml
  mylib = { path = "./lib/mylib" }
  mylib = "./lib/mylib"  # Shorthand
  ```

- **Version dependencies** (planned):
  ```toml
  http = "1.0.0"  # From package registry (future)
  ```

## Project Structure

A typical Soli project structure:

```
my-project/
├── soli.toml
├── src/
│   ├── main.soli      # Entry point
│   └── utils.soli     # Local utilities
└── lib/
    ├── math/
    │   ├── mod.soli   # Module entry point
    │   ├── basic.soli
    │   └── advanced.soli
    └── http/
        └── index.soli
```

## Example

### lib/math.soli
```solilang
export fn add(a: Int, b: Int) -> Int {
    return a + b;
}

export fn square(n: Int) -> Int {
    return n * n;
}
```

### lib/utils.soli
```solilang
export fn greet(name: String) -> String {
    return "Hello, " + name + "!";
}

export fn max(a: Int, b: Int) -> Int {
    if (a > b) { return a; }
    return b;
}
```

### main.soli
```solilang
import "./lib/math.soli";
import { greet, max } from "./lib/utils.soli";

// Use imported functions
print(greet("World"));         // Hello, World!
print("5 + 3 =", add(5, 3));   // 5 + 3 = 8
print("max(10, 7) =", max(10, 7)); // max(10, 7) = 10

// Combine them
let result = square(add(2, 3));
print("square(add(2, 3)) =", result);  // 25
```

## Circular Dependencies

Soli detects circular dependencies and reports an error:

```solilang
// a.soli
import "./b.soli";  // b imports a -> cycle!

// b.soli
import "./a.soli";
```

Error: `Circular dependency: a.soli -> b.soli -> a.soli`

## Best Practices

1. **Use named imports** for clarity and to avoid name conflicts
2. **Keep modules focused** - each module should have a single responsibility
3. **Use `index.soli` or `mod.soli`** as entry points for directories
4. **Document exports** with comments describing their purpose
5. **Avoid circular dependencies** by restructuring your code

## Limitations

Current module system limitations:

- Namespace imports (`import * as name`) don't create an actual namespace object
- No support for re-exports (`export { x } from "module"`)
- No dynamic imports
- Version-based dependencies not yet implemented
