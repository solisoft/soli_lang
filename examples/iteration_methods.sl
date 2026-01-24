// Array iteration methods: map, filter, each
// These work similarly to Ruby arrays

let numbers = [1, 2, 3, 4, 5];

// map - transforms each element and returns new array
let doubled = numbers.map(fn(x) x * 2);
print("Original:", numbers);
print("Doubled (map x*2):", doubled);

// filter - selects elements where predicate returns truthy
let evens = numbers.filter(fn(x) x % 2 == 0);
print("Evens (filter x%2==0):", evens);

// Chaining - map and filter together
let result = numbers
    .map(fn(x) x * 2)
    .filter(fn(x) x > 5);
print("Doubled then filtered (>5):", result);

// each - executes function for side effects, returns original array
print("Each iteration:");
numbers.each(fn(x) print("  " + x));

// Block syntax with explicit return
let squares = numbers.map(fn(n) {
    return n * n;
});
print("Squares (block syntax):", squares);

// Hash iteration methods
let hash = {"name": "Alice", "age": 30, "city": "Paris"};
print("\nOriginal hash:", hash);

// Hash map - function receives [key, value] array, returns [new_key, new_value]
let prefixed = hash.map(fn(pair) {
    return ["prefix_" + pair[0], pair[1]];
});
print("Prefixed keys:", prefixed);

// Hash filter - function receives [key, value] array, returns boolean
let adults = hash.filter(fn(pair) {
    return pair[0] == "age" && pair[1] >= 18;
});
print("Adults filter:", adults);

// Hash each - function receives [key, value] array
print("Hash each:");
hash.each(fn(pair) {
    print("  " + pair[0] + ": " + pair[1]);
});

print("\nAll iteration tests passed!");
