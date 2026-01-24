describe("Calculator", fn() {
    test("adds two numbers", fn() {
        assert_eq(2 + 2, 4);
    });

    test("multiplies correctly", fn() {
        assert_eq(3 * 4, 12);
    });
});

describe("String utilities", fn() {
    test("string contains substring", fn() {
        assert_contains("hello world", "world");
    });

    test("regex matching", fn() {
        assert_match("hello@example.com", "@");
    });
});

context("with user data", fn() {
    before_each(fn() {
        print("Setting up test data");
    });

    test("user has valid email", fn() {
        let user = hash();
        user["email"] = "test@example.com";
        assert_match(user["email"], "@");
    });
});
