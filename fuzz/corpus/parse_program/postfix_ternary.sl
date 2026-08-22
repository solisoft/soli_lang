// Ruby-like postfix if/unless and ternary operator examples

// Postfix if
let x = 10;
print("x is big") if (x > 5);
print("x is small") if (x < 5);
print("---");

// Postfix unless
let y = 3;
print("y is not big") unless (y > 5);
print("y is big") unless (y <= 5);
print("---");

// Ternary operator
let z = 15;
let size = z > 10 ? "large" : "small";
print("z is " + size);

// Nested ternary
let grade = 85;
let letter = grade >= 90 ? "A" : grade >= 80 ? "B" : grade >= 70 ? "C" : "F";
print("Grade: " + letter);

// Ternary with blocks
let status = true;
let message = status ? "Active" : "Inactive";
print("Status: " + message);

print("All tests passed!");
