// Regression: `.nil?`, `.class`, `.blank?`, `.present?`, `.inspect` must work on
// Function values. They're advertised as "universal methods on ALL types" and
// several defensive patterns (e.g. view partials doing
// `type(x) != "function" && !x.nil?`) rely on the nil? check not crashing when
// a name accidentally resolves to a function.
//
// Note: functions with zero parameters auto-invoke on bare access, so these
// tests use multi-arg functions where the function value survives to member
// access without being called.

describe("universal methods on functions", fn() {
    test("user function responds to .nil? (false)", fn() {
        let f = fn(x) { x + 1 };
        assert_eq(f.nil?, false);
    });

    test("user function responds to .blank? (false)", fn() {
        let f = fn(x) { x };
        assert_eq(f.blank?, false);
    });

    test("user function responds to .present? (true)", fn() {
        let f = fn(x) { x };
        assert_eq(f.present?, true);
    });

    test("user function responds to .class (returns \"Function\")", fn() {
        let f = fn(x) { x };
        assert_eq(f.class, "Function");
    });
});
