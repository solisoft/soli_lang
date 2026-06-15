describe("blank_check", fn() {
    test("empty string greets guest", fn() {
        assert_eq(greeting(""), "Hello, Guest!");
    });

    test("nil also greets guest", fn() {
        assert_eq(greeting(null), "Hello, Guest!");
    });

    test("named greeting", fn() {
        assert_eq(greeting("Alice"), "Hello, Alice!");
    });
});
