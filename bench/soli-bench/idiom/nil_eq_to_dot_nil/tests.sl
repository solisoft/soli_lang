describe("nil_eq_to_dot_nil", fn() {
    test("nil user is anonymous", fn() {
        assert_eq(display_name(null), "Anonymous");
    });

    test("named user", fn() {
        assert_eq(display_name({"name": "Alice"}), "Alice");
    });

    test("registered when present", fn() {
        assert_eq(is_registered({"name": "Al"}), true);
    });

    test("not registered when nil", fn() {
        assert_eq(is_registered(null), false);
    });
});
