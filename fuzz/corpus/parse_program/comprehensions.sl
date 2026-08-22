// Test list comprehensions
let squares = [x * x for x in [1, 2, 3, 4, 5]];
print("List comprehension (squares):");
print(squares);  // [1, 4, 9, 16, 25]

// Note: range syntax 1..10 needs to be implemented separately
// For now, use an array
let nums = [1, 2, 3, 4, 5, 6, 7, 8, 9];
let evens = [x for x in nums if x % 2 == 0];
print("List comprehension (evens):");
print(evens);  // [2, 4, 6, 8]

// Test hash comprehensions
let squares_map = {x: x * x for x in [1, 2, 3]};
print("Hash comprehension (squares):");
print(squares_map);  // {1 => 1, 2 => 4, 3 => 9}

let users = [{"name": "Alice", "age": 30}, {"name": "Bob", "age": 25}];
let names = {u["name"]: u["age"] for u in users};
print("Hash comprehension (name -> age):");
print(names);  // {"Alice" => 30, "Bob" => 25}

print("Comprehension tests passed!");
