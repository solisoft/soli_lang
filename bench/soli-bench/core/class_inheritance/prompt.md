# class_inheritance

Implement a small class hierarchy with `super` and a polymorphic method.

- `class Animal` has:
  - a `new(name: String)` constructor that stores `this.name`.
  - a `greet()` method that returns `"hi, I'm \(this.name)"`.

- `class Dog extends Animal` has:
  - a `new(name: String, breed: String)` constructor that calls `super(name)` and
    stores `this.breed`.
  - a `greet()` method that calls `super.greet()` and appends `" (a \(this.breed))"`.

```soli
let a = new Animal("Alex");
a.greet();             // => "hi, I'm Alex"

let d = new Dog("Rex", "lab");
d.greet();             // => "hi, I'm Rex (a lab)"
d.name;                // => "Rex"
d.breed;               // => "lab"
```

**Idiomatic touches we want to see**
- A `class X extends Y { ... }` (C-style braces), or `class X < Y ... end` (Ruby).
- `this.<field>` for instance state.
- `super(name)` to forward to the parent constructor.
- A `super.greet()` call to reuse the parent's behavior.
- Bare assignment in the constructor.

**Note on style**: the rest of the repo prefers Ruby-style for class bodies
(`class Dog < Animal ... end` + `def greet ... end`). Use that here so
your code reads like the rest of the codebase.
