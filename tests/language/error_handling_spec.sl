// ============================================================================
// Error Handling (Try/Catch/Finally) Test Suite
// ============================================================================

describe("Try/Catch/Finally", fn() {
    test("try without error executes normally", fn() {
        let result = 0;
        try {
            result = 42;
        } catch (e) {
            result = -1;
        }
        assert_eq(result, 42);
    });

    test("catch handles thrown error", fn() {
        let result = "";
        try {
            throw "error message";
            result = "not reached";
        } catch (e) {
            result = "caught";
        }
        assert_eq(result, "caught");
    });

    test("finally always executes after try", fn() {
        let finally_ran = false;
        try {
            let x = 1;
        } catch (e) {
            let x = 2;
        } finally {
            finally_ran = true;
        }
        assert(finally_ran);
    });

    test("finally runs after catch", fn() {
        let sequence = [];
        try {
            throw "error";
        } catch (e) {
            sequence.push("catch");
        } finally {
            sequence.push("finally");
        }
        assert_eq(len(sequence), 2);
        assert_eq(sequence[0], "catch");
        assert_eq(sequence[1], "finally");
    });

    test("nested try/catch", fn() {
        let result = "";
        try {
            try {
                throw "inner";
            } catch (e) {
                result = "inner caught";
                throw "outer";
            }
        } catch (e) {
            result = result + " outer caught";
        }
        assert_eq(result, "inner caught outer caught");
    });

    test("try with return in try block", fn() {
        let finally_ran = false;
        fn test_fn() {
            try {
                return 42;
            } finally {
                finally_ran = true;
            }
        }
        let result = test_fn();
        assert_eq(result, 42);
        assert(finally_ran);
    });

    test("try with return in catch block", fn() {
        let finally_ran = false;
        fn test_fn() {
            try {
                throw "error";
            } catch (e) {
                return 100;
            } finally {
                finally_ran = true;
            }
        }
        let result = test_fn();
        assert_eq(result, 100);
        assert(finally_ran);
    });

    test("catch with different error types", fn() {
        let caught_type = "";
        try {
            throw 42;
        } catch (e) {
            caught_type = type(e);
        }
        assert_eq(caught_type, "int");

        let caught_string = "";
        try {
            throw "error";
        } catch (e) {
            caught_string = e;
        }
        assert_eq(caught_string, "error");
    });

    test("empty try block", fn() {
        let ran = false;
        try {
        } finally {
            ran = true;
        }
        assert(ran);
    });

    test("finally with nested try", fn() {
        let order = [];
        try {
            try {
                throw "inner";
            } finally {
                order.push("inner finally");
            }
        } catch (e) {
            order.push("outer catch");
        } finally {
            order.push("outer finally");
        }
        assert_eq(len(order), 3);
        assert_eq(order[0], "inner finally");
        assert_eq(order[1], "outer catch");
        assert_eq(order[2], "outer finally");
    });
});
