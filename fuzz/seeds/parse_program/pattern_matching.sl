// Pattern Matching Examples for Soli

// 1. Basic literal matching
let x = 42;
let result1 = match x {
    42 => "the answer to everything",
    _ => "something else",
};
print("1. Literal match: " + result1);

// 2. Variable binding
let value = 100;
let result2 = match value {
    n => "captured value: " + str(n),
};
print("2. Variable binding: " + result2);

// 3. Guard clauses
let n = -5;
let result3 = match n {
    n if n > 0 => "positive",
    n if n < 0 => "negative",
    0 => "zero",
};
print("3. Guard clause: " + result3);

// 4. Multiple arms
let status = 404;
let result4 = match status {
    200 => "OK",
    201 => "Created",
    404 => "Not Found",
    500 => "Server Error",
    _ => "Unknown",
};
print("4. Multiple arms: " + result4);

// 5. Array matching (first element)
let arr = [1, 2, 3];
let result5 = match arr {
    [] => "empty array",
    [first, ...rest] => "first element: " + str(first),
    _ => "other",
};
print("5. Array matching: " + result5);

// 6. Hash destructuring
let user = {"name": "Alice", "age": 30};
let result6 = match user {
    {name: n} => "name is: " + n,
    {name: n, age: a} => n + " is " + str(a) + " years old",
    _ => "unknown user",
};
print("6. Hash destructuring: " + result6);

// 7. Nested patterns
let data = {"user": {"name": "Bob"}};
let result7 = match data {
    {user: {name: n}} => "nested: " + n,
    _ => "no match",
};
print("7. Nested pattern: " + result7);

// 8. Wildcard pattern
let x = 999;
let result8 = match x {
    1 => "one",
    2 => "two",
    _ => "anything else",
};
print("8. Wildcard: " + result8);

// 9. String matching
let command = "start";
let result9 = match command {
    "start" => "Starting...",
    "stop" => "Stopping...",
    "restart" => "Restarting...",
    _ => "Unknown command",
};
print("9. String match: " + result9);

// 10. Boolean matching
let flag = false;
let result10 = match flag {
    true => "enabled",
    false => "disabled",
};
print("10. Boolean match: " + result10);

// 11. Complex guard conditions
let score = 85;
let grade = match score {
    s if s >= 90 => "A",
    s if s >= 80 => "B",
    s if s >= 70 => "C",
    s if s >= 60 => "D",
    _ => "F",
};
print("11. Grade: " + grade + " (score: " + str(score) + ")");

// 12. Null handling
let maybeValue: Any = null;
let result12 = match maybeValue {
    null => "it's null",
    _ => "has a value",
};
print("12. Null check: " + result12);

print("\nAll pattern matching examples completed!");
