// Test exception handling

// Test 1: Basic try/catch
print("Test 1: Basic try/catch");
try {
    throw "Error from try block";
    print("This should not print");
} catch (e) {
    print("Caught: " + str(e));
}

// Test 2: Try/catch/finally
print("\nTest 2: Try/catch/finally");
try {
    throw "Error in try";
    print("After throw");
} catch (e) {
    print("Catch: " + str(e));
} finally {
    print("Finally block executed");
}

// Test 3: No exception
print("\nTest 3: No exception (finally only)");
try {
    print("In try block - no error");
} finally {
    print("Finally executed");
}

// Test 4: Nested try/catch
print("\nTest 4: Nested try/catch");
try {
    try {
        throw "Inner error";
    } catch (inner) {
        print("Inner catch: " + str(inner));
        throw "Outer error";
    }
} catch (outer) {
    print("Outer catch: " + str(outer));
}

// Test 5: Throw from catch (rethrow)
print("\nTest 5: Rethrow");
try {
    try {
        throw "Original error";
    } catch (e) {
        print("First catch: " + str(e));
        throw "Re-thrown error";
    }
} catch (e2) {
    print("Second catch: " + str(e2));
}

// Test 6: Using Error class
print("\nTest 6: Error class");
try {
    throw ValueError.new("Custom error message");
} catch (e) {
    print("Caught ValueError: " + str(e));
}

// Test 7: Throw statement
print("\nTest 7: Throw statement");
throw "Throw statement error";
print("This should not print");

print("\nAll tests completed!");
