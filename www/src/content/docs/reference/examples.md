---
title: Example Programs
description: Complete example programs in Soli
---

# Example Programs

Here are several complete programs demonstrating Soli's features.

## Hello World

The simplest program:

```rust
print("Hello, World!");
```

## FizzBuzz

The classic programming challenge:

```rust
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
while (i <= 100) {
    print(fizzbuzz(i));
    i = i + 1;
}
```

## Fibonacci

Recursive Fibonacci sequence:

```rust
fn fibonacci(n: Int) -> Int {
    if (n <= 1) {
        return n;
    }
    return fibonacci(n - 1) + fibonacci(n - 2);
}

print("Fibonacci sequence:");
for (i in range(0, 15)) {
    print("fib(" + str(i) + ") = " + str(fibonacci(i)));
}
```

## Pipeline Demo

Showcasing the pipeline operator:

```rust
fn double(x: Int) -> Int {
    return x * 2;
}

fn addOne(x: Int) -> Int {
    return x + 1;
}

fn square(x: Int) -> Int {
    return x * x;
}

fn add(a: Int, b: Int) -> Int {
    return a + b;
}

// Chain transformations
print("5 |> double() |> addOne() =", 5 |> double() |> addOne());
print("3 |> square() |> double() =", 3 |> square() |> double());

// With multiple arguments
print("5 |> add(3) =", 5 |> add(3));
print("10 |> add(5) |> double() =", 10 |> add(5) |> double());
```

## Object-Oriented Shapes

A complete OOP example:

```rust
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

    fn getPosition() -> String {
        return "(" + str(this.x) + ", " + str(this.y) + ")";
    }
}

class Circle extends Shape implements Drawable {
    radius: Float;

    new(x: Float, y: Float, radius: Float) {
        this.x = x;
        this.y = y;
        this.radius = radius;
    }

    fn getArea() -> Float {
        return 3.14159 * this.radius * this.radius;
    }

    fn draw() -> String {
        return "Circle at " + this.getPosition() +
               " with radius " + str(this.radius);
    }
}

class Rectangle extends Shape implements Drawable {
    width: Float;
    height: Float;

    new(x: Float, y: Float, width: Float, height: Float) {
        this.x = x;
        this.y = y;
        this.width = width;
        this.height = height;
    }

    fn getArea() -> Float {
        return this.width * this.height;
    }

    fn draw() -> String {
        return "Rectangle at " + this.getPosition() +
               " (" + str(this.width) + "x" + str(this.height) + ")";
    }
}

// Create shapes
let circle = new Circle(10.0, 20.0, 5.0);
let rect = new Rectangle(0.0, 0.0, 10.0, 5.0);

print("=== Shapes Demo ===");
print(circle.draw());
print("Area: " + str(circle.getArea()));
print("");
print(rect.draw());
print("Area: " + str(rect.getArea()));
```

## Array Processing

Working with collections:

```rust
fn sum(numbers: Int[]) -> Int {
    let total = 0;
    for (n in numbers) {
        total = total + n;
    }
    return total;
}

fn average(numbers: Int[]) -> Float {
    return float(sum(numbers)) / float(len(numbers));
}

fn filterEven(numbers: Int[]) -> Int[] {
    let result: Int[] = [];
    for (n in numbers) {
        if (n % 2 == 0) {
            push(result, n);
        }
    }
    return result;
}

fn doubleAll(numbers: Int[]) -> Int[] {
    let result: Int[] = [];
    for (n in numbers) {
        push(result, n * 2);
    }
    return result;
}

let numbers = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];

print("Numbers:", numbers);
print("Sum:", sum(numbers));
print("Average:", average(numbers));
print("Even numbers:", filterEven(numbers));
print("Doubled:", doubleAll(numbers));

// Pipeline processing
let processed = numbers |> filterEven() |> doubleAll() |> sum();
print("Sum of doubled evens:", processed);
```

## Simple Calculator

A basic stack calculator:

```rust
class Calculator {
    result: Float;

    new() {
        this.result = 0.0;
    }

    fn set(value: Float) -> Calculator {
        this.result = value;
        return this;
    }

    fn add(value: Float) -> Calculator {
        this.result = this.result + value;
        return this;
    }

    fn subtract(value: Float) -> Calculator {
        this.result = this.result - value;
        return this;
    }

    fn multiply(value: Float) -> Calculator {
        this.result = this.result * value;
        return this;
    }

    fn divide(value: Float) -> Calculator {
        if (value != 0.0) {
            this.result = this.result / value;
        }
        return this;
    }

    fn getResult() -> Float {
        return this.result;
    }
}

let calc = new Calculator();
calc.set(10.0).add(5.0).multiply(2.0).subtract(3.0);
print("Result:", calc.getResult());  // 27.0
```

## Prime Numbers

Finding prime numbers:

```rust
fn isPrime(n: Int) -> Bool {
    if (n <= 1) {
        return false;
    }
    if (n <= 3) {
        return true;
    }
    if (n % 2 == 0) {
        return false;
    }

    let i = 3;
    while (i * i <= n) {
        if (n % i == 0) {
            return false;
        }
        i = i + 2;
    }
    return true;
}

fn findPrimes(limit: Int) -> Int[] {
    let primes: Int[] = [];
    for (n in range(2, limit)) {
        if (isPrime(n)) {
            push(primes, n);
        }
    }
    return primes;
}

print("Primes up to 50:", findPrimes(50));
```

## Countdown Timer

Using the clock function:

```rust
fn sleep(seconds: Float) {
    let start = clock();
    while (clock() - start < seconds) {
        // Wait
    }
}

fn countdown(from: Int) {
    let i = from;
    while (i >= 0) {
        print(i);
        sleep(0.1);  // Short delay for demo
        i = i - 1;
    }
    print("Liftoff!");
}

countdown(5);
```

## Bank Account

A practical OOP example:

```rust
class BankAccount {
    holder: String;
    balance: Float;

    new(holder: String, initialDeposit: Float) {
        this.holder = holder;
        this.balance = initialDeposit;
    }

    fn deposit(amount: Float) -> Bool {
        if (amount > 0.0) {
            this.balance = this.balance + amount;
            print(this.holder + " deposited $" + str(amount));
            return true;
        }
        return false;
    }

    fn withdraw(amount: Float) -> Bool {
        if (amount > 0.0 && amount <= this.balance) {
            this.balance = this.balance - amount;
            print(this.holder + " withdrew $" + str(amount));
            return true;
        }
        print("Withdrawal failed for " + this.holder);
        return false;
    }

    fn getBalance() -> Float {
        return this.balance;
    }

    fn printStatement() {
        print("Account holder: " + this.holder);
        print("Balance: $" + str(this.balance));
    }
}

let account = new BankAccount("Alice", 1000.0);
account.printStatement();
print("");

account.deposit(250.0);
account.withdraw(100.0);
account.withdraw(2000.0);  // Should fail
print("");

account.printStatement();
```

## Hash Dictionary

Working with hashes (Ruby-style):

```rust
// Create a hash
let person = {
    "name" => "Alice",
    "age" => 30,
    "city" => "New York"
};

print("Person:", person);
print("Name:", person["name"]);

// Add and modify
person["email"] = "alice@example.com";
person["age"] = 31;

// Hash functions
print("Keys:", keys(person));
print("Has email?", has_key(person, "email"));

// Iteration
for (pair in entries(person)) {
    print(pair[0] + ":", pair[1]);
}

// Merge hashes
let defaults = {"theme" => "dark", "language" => "en"};
let settings = {"theme" => "light"};
let config = merge(defaults, settings);
print("Config:", config);  // {theme => light, language => en}
```

## Counting Words

Using hashes for counting:

```rust
fn count_words(text: String) -> Any {
    let words = ["the", "quick", "brown", "fox", "the", "lazy", "dog", "the"];
    let counts = {};

    for (word in words) {
        if (has_key(counts, word)) {
            counts[word] = counts[word] + 1;
        } else {
            counts[word] = 1;
        }
    }

    return counts;
}

let result = count_words("the quick brown fox the lazy dog the");
print(result);  // {the => 3, quick => 1, brown => 1, fox => 1, lazy => 1, dog => 1}
```

## HTTP Server

Building a REST API server:

```rust
// In-memory user store
let users = {};
let next_id = 1;

fn handle_home(req: Any) -> Any {
    return {"status": 200, "body": "Welcome to the API!"};
}

fn handle_list_users(req: Any) -> Any {
    return {
        "status": 200,
        "headers": {"Content-Type": "application/json"},
        "body": json_stringify(values(users))
    };
}

fn handle_get_user(req: Any) -> Any {
    let id = req["params"]["id"];
    if (has_key(users, id)) {
        return {
            "status": 200,
            "headers": {"Content-Type": "application/json"},
            "body": json_stringify(users[id])
        };
    } else {
        return {
            "status": 404,
            "headers": {"Content-Type": "application/json"},
            "body": json_stringify({"error": "User not found"})
        };
    }
}

fn handle_create_user(req: Any) -> Any {
    let data = json_parse(req["body"]);
    let id = str(next_id);
    next_id = next_id + 1;

    let user = {"id": id, "name": data["name"], "email": data["email"]};
    users[id] = user;

    return {
        "status": 201,
        "headers": {"Content-Type": "application/json"},
        "body": json_stringify(user)
    };
}

fn handle_delete_user(req: Any) -> Any {
    let id = req["params"]["id"];
    if (has_key(users, id)) {
        let user = users[id];
        delete(users, id);
        return {
            "status": 200,
            "headers": {"Content-Type": "application/json"},
            "body": json_stringify({"deleted": user})
        };
    } else {
        return {"status": 404, "body": json_stringify({"error": "User not found"})};
    }
}

// Register routes
http_server_get("/", handle_home);
http_server_get("/users", handle_list_users);
http_server_get("/users/:id", handle_get_user);
http_server_post("/users", handle_create_user);
http_server_delete("/users/:id", handle_delete_user);

// Start server (blocks)
println("Server running on http://localhost:3000");
http_server_listen(3000);
```

Test with curl:
```bash
# Get all users
curl http://localhost:3000/users

# Create a user
curl -X POST -H "Content-Type: application/json" \
  -d '{"name":"Alice","email":"alice@example.com"}' \
  http://localhost:3000/users

# Get a specific user
curl http://localhost:3000/users/1

# Delete a user
curl -X DELETE http://localhost:3000/users/1
```

## Running Examples

Save any example to a `.soli` file and run with:

```bash
soli example.soli
```

Or experiment interactively in the REPL:

```bash
soli
```
