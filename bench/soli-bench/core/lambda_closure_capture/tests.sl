describe("lambda_closure_capture", fn() {
    test("counter increments", fn() {
        let c = make_counter(0);
        assert_eq(c, 0);
        assert_eq(c, 1);
        assert_eq(c, 2);
    });

    test("counter from non-zero start", fn() {
        let c = make_counter(10);
        assert_eq(c, 10);
        assert_eq(c, 11);
    });

    test("two counters are independent", fn() {
        let c1 = make_counter(0);
        let c2 = make_counter(100);
        assert_eq(c1, 0);
        assert_eq(c1, 1);
        assert_eq(c2, 100);
        assert_eq(c1, 2);
        assert_eq(c2, 101);
    });
});
