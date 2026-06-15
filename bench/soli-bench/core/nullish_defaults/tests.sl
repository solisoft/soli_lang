describe("nullish_defaults", fn() {
    test("with_default — null returns default", fn() {
        assert_eq(with_default(null, 0), 0);
    });

    test("with_default — value returns value", fn() {
        assert_eq(with_default(42, 0), 42);
    });

    test("coalesce — first non-null", fn() {
        assert_eq(coalesce("a", "b", "c"), "a");
    });

    test("coalesce — middle non-null", fn() {
        assert_eq(coalesce(null, "b", "c"), "b");
    });

    test("coalesce — last non-null", fn() {
        assert_eq(coalesce(null, null, "c"), "c");
    });

    test("safe_lookup — key present", fn() {
        assert_eq(safe_lookup({"a": 1}, "a"), 1);
    });

    test("safe_lookup — key missing", fn() {
        assert_eq(safe_lookup({}, "x"), "missing");
    });
});
