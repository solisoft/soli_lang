---
title: Classes & OOP
description: Object-oriented programming in Soli
---

# Classes & OOP

Soli supports full object-oriented programming with classes, inheritance, and interfaces.

## Defining Classes

### Basic Class

```rust
class Person {
    name: String;
    age: Int;

    new(name: String, age: Int) {
        this.name = name;
        this.age = age;
    }

    fn greet() -> String {
        return "Hello, I'm " + this.name;
    }
}
```

### Creating Instances

```rust
let person = new Person("Alice", 30);
print(person.greet());  // Hello, I'm Alice
print(person.name);     // Alice
print(person.age);      // 30
```

## Constructors

The `new` keyword defines a constructor:

```rust
class Rectangle {
    width: Float;
    height: Float;

    new(w: Float, h: Float) {
        this.width = w;
        this.height = h;
    }
}

let rect = new Rectangle(10.0, 5.0);
```

### Default Values

Set defaults in the constructor:

```rust
class Counter {
    value: Int;

    new() {
        this.value = 0;
    }

    fn increment() {
        this.value = this.value + 1;
    }
}
```

## Methods

### Instance Methods

```rust
class Circle {
    radius: Float;

    new(r: Float) {
        this.radius = r;
    }

    fn area() -> Float {
        return 3.14159 * this.radius * this.radius;
    }

    fn circumference() -> Float {
        return 2.0 * 3.14159 * this.radius;
    }
}

let circle = new Circle(5.0);
print(circle.area());          // 78.53975
print(circle.circumference()); // 31.4159
```

### Using `this`

Inside methods, `this` refers to the current instance:

```rust
class BankAccount {
    balance: Float;

    new(initial: Float) {
        this.balance = initial;
    }

    fn deposit(amount: Float) {
        this.balance = this.balance + amount;
    }

    fn withdraw(amount: Float) -> Bool {
        if (amount <= this.balance) {
            this.balance = this.balance - amount;
            return true;
        }
        return false;
    }
}
```

## Inheritance

Use `extends` to inherit from a parent class:

```rust
class Animal {
    name: String;

    new(name: String) {
        this.name = name;
    }

    fn speak() -> String {
        return this.name + " makes a sound";
    }
}

class Dog extends Animal {
    breed: String;

    new(name: String, breed: String) {
        this.name = name;
        this.breed = breed;
    }

    fn speak() -> String {
        return this.name + " barks!";
    }

    fn fetch() -> String {
        return this.name + " fetches the ball";
    }
}

let dog = new Dog("Buddy", "Golden Retriever");
print(dog.speak());  // Buddy barks!
print(dog.fetch());  // Buddy fetches the ball
```

## Interfaces

Interfaces define contracts that classes must fulfill. They specify **what** methods a class must have, without providing **how** they work.

### Why Use Interfaces?

- **Contracts**: Guarantee that classes provide required functionality
- **Polymorphism**: Treat different classes uniformly through a common interface
- **Decoupling**: Code depends on abstractions, not concrete implementations
- **Documentation**: Clearly communicate expected behavior

### Defining Interfaces

Use the `interface` keyword to declare method signatures:

```rust
interface Drawable {
    fn draw() -> String;
    fn getColor() -> String;
}

interface Resizable {
    fn resize(factor: Float);
    fn getSize() -> Float;
}
```

**Key points:**
- Interfaces contain only method signatures (no implementations)
- Interfaces cannot have fields
- Methods don't need visibility modifiers (they're always public)

### Implementing Interfaces

A class uses `implements` to declare it fulfills an interface contract:

```rust
interface Drawable {
    fn draw() -> String;
}

class Circle implements Drawable {
    radius: Float;

    new(r: Float) {
        this.radius = r;
    }

    // Required by Drawable interface
    fn draw() -> String {
        return "Circle with radius " + str(this.radius);
    }
}

let shape = new Circle(5.0);
print(shape.draw());  // Circle with radius 5
```

The type checker verifies that:
1. Every method in the interface is implemented
2. Method signatures match exactly (same parameters and return type)

### Multiple Interfaces

A class can implement multiple interfaces by separating them with commas:

```rust
interface Drawable {
    fn draw() -> String;
}

interface Resizable {
    fn resize(factor: Float);
}

interface Movable {
    fn move(dx: Float, dy: Float);
}

class Rectangle implements Drawable, Resizable, Movable {
    x: Float;
    y: Float;
    width: Float;
    height: Float;

    new(x: Float, y: Float, w: Float, h: Float) {
        this.x = x;
        this.y = y;
        this.width = w;
        this.height = h;
    }

    // From Drawable
    fn draw() -> String {
        return "Rectangle at (" + str(this.x) + "," + str(this.y) + ")";
    }

    // From Resizable
    fn resize(factor: Float) {
        this.width = this.width * factor;
        this.height = this.height * factor;
    }

    // From Movable
    fn move(dx: Float, dy: Float) {
        this.x = this.x + dx;
        this.y = this.y + dy;
    }
}
```

### Combining Inheritance and Interfaces

A class can extend a parent class AND implement interfaces:

```rust
class Shape {
    x: Float;
    y: Float;

    new(x: Float, y: Float) {
        this.x = x;
        this.y = y;
    }

    fn getPosition() -> String {
        return "(" + str(this.x) + ", " + str(this.y) + ")";
    }
}

class Circle extends Shape implements Drawable, Resizable {
    radius: Float;

    new(x: Float, y: Float, radius: Float) {
        this.x = x;
        this.y = y;
        this.radius = radius;
    }

    fn draw() -> String {
        return "Circle at " + this.getPosition();  // Uses inherited method
    }

    fn resize(factor: Float) {
        this.radius = this.radius * factor;
    }
}
```

### Interface Design Patterns

#### Small, Focused Interfaces

Prefer many small interfaces over one large one:

```rust
// Good: Small, focused interfaces
interface Readable {
    fn read() -> String;
}

interface Writable {
    fn write(data: String);
}

interface Closable {
    fn close();
}

// A file implements all three
class File implements Readable, Writable, Closable {
    // ...
}

// A read-only stream only implements Readable
class InputStream implements Readable {
    // ...
}
```

#### Common Interface Patterns

```rust
// Comparable - for sorting/ordering
interface Comparable {
    fn compareTo(other: Comparable) -> Int;
}

// Serializable - for converting to/from strings
interface Serializable {
    fn serialize() -> String;
    fn deserialize(data: String);
}

// Iterator - for traversing collections
interface Iterator {
    fn hasNext() -> Bool;
    fn next() -> Any;
}

// Observer - for event handling
interface Observer {
    fn update(event: String);
}
```

### Compile-Time Checking

If you forget to implement a method, you'll get a compile-time error:

```rust
interface Drawable {
    fn draw() -> String;
    fn getColor() -> String;
}

class Square implements Drawable {
    size: Float;

    new(s: Float) {
        this.size = s;
    }

    fn draw() -> String {
        return "Square";
    }

    // ERROR: Missing getColor() method!
    // The type checker will report:
    // "class 'Square' does not implement method 'getColor' from interface 'Drawable'"
}
```

### Interface Summary

| Feature | Description |
|---------|-------------|
| Declaration | `interface Name { fn method() -> Type; }` |
| Implementation | `class X implements InterfaceName { ... }` |
| Multiple | `class X implements A, B, C { ... }` |
| With inheritance | `class X extends Parent implements A { ... }` |
| Contents | Method signatures only (no fields, no implementations) |
| Checking | Compile-time verification of all methods |

## Visibility Modifiers

Control access to fields and methods:

```rust
class User {
    public name: String;
    private password: String;
    protected email: String;

    new(name: String, password: String, email: String) {
        this.name = name;
        this.password = password;
        this.email = email;
    }

    public fn getName() -> String {
        return this.name;
    }

    private fn hashPassword() -> String {
        // Internal use only
        return "hashed:" + this.password;
    }
}
```

| Modifier | Access |
|----------|--------|
| `public` | Accessible from anywhere |
| `private` | Only within the class |
| `protected` | Within class and subclasses |

## Static Members

Class-level fields and methods:

```rust
class MathUtils {
    static PI: Float = 3.14159;

    static fn square(x: Float) -> Float {
        return x * x;
    }

    static fn cube(x: Float) -> Float {
        return x * x * x;
    }
}

print(MathUtils.PI);           // 3.14159
print(MathUtils.square(4.0));  // 16
```

## Complete Example

```rust
interface Printable {
    fn toString() -> String;
}

class Vehicle {
    brand: String;
    year: Int;

    new(brand: String, year: Int) {
        this.brand = brand;
        this.year = year;
    }

    fn getAge() -> Int {
        return 2024 - this.year;
    }
}

class Car extends Vehicle implements Printable {
    model: String;
    mileage: Float;

    new(brand: String, year: Int, model: String) {
        this.brand = brand;
        this.year = year;
        this.model = model;
        this.mileage = 0.0;
    }

    fn drive(miles: Float) {
        this.mileage = this.mileage + miles;
    }

    fn toString() -> String {
        return this.year + " " + this.brand + " " + this.model +
               " (" + str(this.mileage) + " miles)";
    }
}

let car = new Car("Toyota", 2020, "Camry");
car.drive(150.5);
car.drive(75.0);
print(car.toString());  // 2020 Toyota Camry (225.5 miles)
print("Age: " + str(car.getAge()) + " years");
```

## Best Practices

1. **Single Responsibility**: Each class should have one purpose
2. **Prefer Composition**: Use composition over deep inheritance hierarchies
3. **Program to Interfaces**: Depend on abstractions, not concrete classes
4. **Encapsulate State**: Use private fields with public methods

## Next Steps

- Learn about the [Pipeline Operator](/guides/pipeline/)
- Explore [Arrays](/guides/arrays/)
- See the [Built-in Functions Reference](/reference/builtins/)
