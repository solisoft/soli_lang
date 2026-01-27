# Testing Assertions

Soli provides assertion helper functions for writing tests with clear, expressive syntax. These functions are typically defined in `tests/helpers/assertions.sl` and can be imported into test files.

## Assertion Functions

### assert_equal(expected, actual, message)

Asserts that two values are equal.

```soli
assert_equal(42, result, "should return 42");
assert_equal("hello", str, "string should match");
assert_equal(true, is_valid, "should be valid");
```

**Returns:** `{passed: boolean, message: string, expected: Any, actual: Any}`

### assert_true(value, message)

Asserts that a value is `true`.

```soli
assert_true(user.is_active, "user should be active");
assert_true(contains(items, "test"), "should contain test item");
```

**Returns:** `{passed: boolean, message: string, expected: true, actual: Any}`

### assert_false(value, message)

Asserts that a value is `false`.

```soli
assert_false(user.is_blocked, "user should not be blocked");
assert_false(is_empty(items), "items should not be empty");
```

**Returns:** `{passed: boolean, message: string, expected: false, actual: Any}`

### assert_contains(haystack, needle, message)

Asserts that a collection contains a specific value.

```soli
assert_contains(users, "admin", "should contain admin user");
assert_contains([1, 2, 3], 2, "should contain 2");
```

**Returns:** `{passed: boolean, message: string}`

### assert_nil(value, message)

Asserts that a value is `nil` (null).

```soli
assert_nil(result.error, "should have no error");
assert_nil(user.deleted_at, "should not be deleted");
```

**Returns:** `{passed: boolean, message: string, expected: nil, actual: Any}`

### assert_not_nil(value, message)

Asserts that a value is not `nil`.

```soli
assert_not_nil(user.id, "user should have an id");
assert_not_nil(response.body, "response should have body");
```

**Returns:** `{passed: boolean, message: string, expected: "not nil", actual: Any}`

## Result Structure

All assertion functions return a result hash with the following structure:

```soli
{
    "passed": true,
    "message": "description of the test",
    "expected": value_that_was_expected,
    "actual": value_that_was_actual
}
```

Example:

```soli
let result = assert_equal(42, response.code, "status should be 42");

# result is:
# {
#     "passed": true,
#     "message": "status should be 42",
#     "expected": 42,
#     "actual": 42
# }
```

## Test Runner Pattern

A typical test file structure with assertions:

```soli
class UserModelTest {
    static fn run() {
        let results = [];

        # Test 1: Create user
        let user = User.create({"email": "test@example.com", "name": "Test"});
        results.push(assert_true(contains(user, "id"), "create() returns user with id"));
        results.push(assert_equal("test@example.com", user["email"], "email is stored correctly"));

        # Test 2: Find user
        let found = User.find_by_email("test@example.com");
        results.push(assert_not_nil(found, "find_by_email() returns user"));
        results.push(assert_equal(user["id"], found["id"], "same user is returned"));

        return results;
    }
}
```

## Running Tests

```bash
# Run test runner
soli tests/run_tests.sl

# Run with npm (if configured in package.json)
npm test
```

## Test Results

After running tests, collect and report results:

```soli
let passed = 0;
let failed = 0;

for result in results {
    if result["passed"] {
        passed = passed + 1;
        print("  [PASS] " + result["message"]);
    } else {
        failed = failed + 1;
        print("  [FAIL] " + result["message"]);
        print("         Expected: " + string(result["expected"]));
        print("         Actual: " + string(result["actual"]));
    }
}

print("");
print("Summary: " + string(passed) + " passed, " + string(failed) + " failed");
```

## Custom Assertions

You can create custom assertions by defining new functions:

```soli
fn assert_length(expected_len, collection, message) {
    let actual_len = len(collection);
    return {
        "passed": actual_len == expected_len,
        "message": message,
        "expected": expected_len,
        "actual": actual_len
    };
}

fn assert_starts_with(prefix, str, message) {
    let starts_with = len(str) >= len(prefix) && substring(str, 0, len(prefix)) == prefix;
    return {
        "passed": starts_with,
        "message": message,
        "expected": prefix + "...",
        "actual": str
    };
}
```

## Best Practices

1. **Use descriptive messages**: Always provide clear test messages
2. **Test one thing per assertion**: Separate assertions for separate concerns
3. **Use appropriate assertions**: Use `assert_nil` instead of `assert_equal(nil, ...)`
4. **Check both positive and negative cases**: Test both success and failure scenarios
5. **Return all results**: Collect results in an array and return them from test functions

## Example Test Suite

```soli
class TransactionModelTest {
    static fn run() {
        let results = [];
        MockDatabase.reset();

        # Test create
        let tx = TransactionModel.create({"amount": 100, "currency": "EUR"});
        results.push(assert_not_nil(tx["id"], "create() returns id"));
        results.push(assert_equal("pending", tx["status"], "default status is pending"));

        # Test find
        let found = TransactionModel.find_by_id(tx["id"]);
        results.push(assert_not_nil(found, "find_by_id() returns transaction"));
        results.push(assert_equal(tx["amount"], found["amount"], "amount matches"));

        # Test update
        let updated = TransactionModel.update_status(tx["id"], "paid");
        results.push(assert_equal("paid", updated["status"], "status updated"));

        # Test stats
        let stats = TransactionModel.stats();
        results.push(assert_equal(1, stats["total"], "one transaction in stats"));

        return results;
    }
}
```
