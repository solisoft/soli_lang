// Array operations in Solilang

// Create an array
let numbers = [1, 2, 3, 4, 5];
print("Original array:", numbers);
print("Length:", len(numbers));

// Access elements
print("First element:", numbers[0]);
print("Last element:", numbers[4]);

// Modify elements
numbers[2] = 30;
print("After modification:", numbers);

// Array with push and pop
let stack = [10, 20];
stack.push(30);
stack.push(40);
print("Stack after pushes:", stack);

let top = stack.pop();
print("Popped:", top);
print("Stack after pop:", stack);

// Iterate with for loop
print("Iterating:");
for (num in numbers) {
    print("  -", num);
}

// Using range
print("Range 0 to 5:");
for (i in range(0, 5)) {
    print("  ", i);
}
