describe("postfix_guards", fn() {
    test("greet_adult — adult", fn() {
        assert_eq(greet_adult(30), "hello adult");
    });

    test("greet_adult — minor", fn() {
        assert_eq(greet_adult(12), "hello minor");
    });

    test("safe_value — empty string falls back", fn() {
        assert_eq(safe_value(""), "default");
    });

    test("safe_value — non-empty passes through", fn() {
        assert_eq(safe_value("ok"), "ok");
    });

    test("maybe_double — non-null", fn() {
        assert_eq(maybe_double(5), 10);
    });

    test("maybe_double — null stays null", fn() {
        assert_null(maybe_double(null));
    });
});
