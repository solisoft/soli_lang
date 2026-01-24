// Test exception handling

print("Test 1: Basic try/catch");
try {
    throw "Error from try block";
    print("This should not print");
} catch (e) {
    print("Caught: " + str(e));
}

print("\nTest 2: Try/catch/finally");
try {
    throw "Error in try";
} catch (e) {
    print("Catch: " + str(e));
} finally {
    print("Finally block executed");
}

print("\nTest 3: No exception (finally only)");
try {
    print("In try block - no error");
} finally {
    print("Finally executed");
}

print("\nAll tests completed!");
