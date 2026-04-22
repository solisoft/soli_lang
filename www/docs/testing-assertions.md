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

## Expect API

Soli provides a chainable `expect()` API for more expressive assertions:

### expect(value)

Creates an expectation with the given value. Chain with `to_*()` methods:

```soli
expect(42).to_equal(42);
expect("hello").to_contain("ell");
expect(10).to_be_greater_than(5);
expect(user).to_not_be_null();
```

### to_be(expected)

Asserts that the actual value is the same as expected (identity check):

```soli
expect(42).to_be(42);
expect(true).to_be(true);
```

### to_equal(expected)

Asserts that the actual value equals expected (value equality):

```soli
expect(42).to_equal(42);
expect("hello").to_equal("hello");
```

### to_not_be(expected)

Asserts that the actual value is not the same as expected:

```soli
expect(42).to_not_be(43);
expect("hello").to_not_be("world");
```

### to_not_equal(expected)

Asserts that the actual value does not equal expected:

```soli
expect(42).to_not_equal(43);
expect("hello").to_not_equal("world");
```

### to_be_null()

Asserts that the actual value is null:

```soli
expect(result.error).to_be_null();
expect(user.deleted_at).to_be_null();
```

### to_not_be_null()

Asserts that the actual value is not null:

```soli
expect(user.id).to_not_be_null();
expect(response.body).to_not_be_null();
```

### to_be_greater_than(expected)

Asserts that the actual number is greater than expected:

```soli
expect(10).to_be_greater_than(5);
expect(count).to_be_greater_than(0);
```

### to_be_less_than(expected)

Asserts that the actual number is less than expected:

```soli
expect(5).to_be_less_than(10);
expect(len(items)).to_be_less_than(100);
```

### to_be_greater_than_or_equal(expected)

Asserts that the actual number is greater than or equal to expected:

```soli
expect(10).to_be_greater_than_or_equal(10);
expect(count).to_be_greater_than_or_equal(1);
```

### to_be_less_than_or_equal(expected)

Asserts that the actual number is less than or equal to expected:

```soli
expect(5).to_be_less_than_or_equal(5);
expect(len(items)).to_be_less_than_or_equal(10);
```

### to_contain(item)

Asserts that the actual value (array or string) contains the given item:

```soli
expect([1, 2, 3]).to_contain(2);
expect("hello world").to_contain("world");
```

### to_be_valid_json()

Asserts that the actual string is valid JSON:

```soli
expect('{"name": "Alice"}').to_be_valid_json();
expect(response.body).to_be_valid_json();
```

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
class UserModelTest
    static fn run
        let results = []

        # Test 1: Create user
        let user = User.create({"email": "test@example.com", "name": "Test"})
        results.push(assert_true(contains(user, "id"), "create() returns user with id"))
        results.push(assert_equal("test@example.com", user["email"], "email is stored correctly"))

        # Test 2: Find user
        let found = User.find_by_email("test@example.com")
        results.push(assert_not_nil(found, "find_by_email() returns user"))
        results.push(assert_equal(user["id"], found["id"], "same user is returned"))

        results
    end
end
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
let passed = 0
let failed = 0

for result in results
    if result["passed"]
        passed = passed + 1
        print("  [PASS] " + result["message"])
    else
        failed = failed + 1
        print("  [FAIL] " + result["message"])
        print("         Expected: " + string(result["expected"]))
        print("         Actual: " + string(result["actual"]))
    end
end

print("")
print("Summary: " + string(passed) + " passed, " + string(failed) + " failed")
```

## Custom Assertions

You can create custom assertions by defining new functions:

```soli
fn assert_length(expected_len, collection, message)
    let actual_len = len(collection)
    {
        "passed": actual_len == expected_len,
        "message": message,
        "expected": expected_len,
        "actual": actual_len
    }
end

fn assert_starts_with(prefix, str, message)
    let starts_with = len(str) >= len(prefix) && substring(str, 0, len(prefix)) == prefix
    {
        "passed": starts_with,
        "message": message,
        "expected": prefix + "...",
        "actual": str
    }
end
```

## Best Practices

1. **Use descriptive messages**: Always provide clear test messages
2. **Test one thing per assertion**: Separate assertions for separate concerns
3. **Use appropriate assertions**: Use `assert_nil` instead of `assert_equal(nil, ...)`
4. **Check both positive and negative cases**: Test both success and failure scenarios
5. **Return all results**: Collect results in an array and return them from test functions

## Example Test Suite

```soli
class TransactionModelTest
    static fn run
        let results = []
        MockDatabase.reset()

        # Test create
        let tx = TransactionModel.create({"amount": 100, "currency": "EUR"})
        results.push(assert_not_nil(tx["id"], "create() returns id"))
        results.push(assert_equal("pending", tx["status"], "default status is pending"))

        # Test find
        let found = TransactionModel.find_by_id(tx["id"])
        results.push(assert_not_nil(found, "find_by_id() returns transaction"))
        results.push(assert_equal(tx["amount"], found["amount"], "amount matches"))

        # Test update
        let updated = TransactionModel.update_status(tx["id"], "paid")
        results.push(assert_equal("paid", updated["status"], "status updated"))

        # Test stats
        let stats = TransactionModel.stats()
        results.push(assert_equal(1, stats["total"], "one transaction in stats"))

        results
    end
end
```
