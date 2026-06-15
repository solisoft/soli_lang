describe("includes_check", fn() {
    test("valid status matches", fn() {
        assert_eq(is_valid_status("up"), true);
        assert_eq(is_valid_status("overdue"), true);
    });

    test("invalid status rejected", fn() {
        assert_eq(is_valid_status("done"), false);
    });

    test("privileged role excludes the blocked list", fn() {
        assert_eq(is_privileged("admin"), true);
    });

    test("blocked role is not privileged", fn() {
        assert_eq(is_privileged("guest"), false);
        assert_eq(is_privileged("banned"), false);
    });
});
