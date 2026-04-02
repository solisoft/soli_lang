// ============================================================================
// Type Annotations Test Suite - Additional Coverage
// ============================================================================

describe("Void Type", fn() {
    test("function returning void allows implicit return", fn() {
        fn do_nothing() -> Void {
            let x = 1;
        }
        let result = do_nothing();
        assert_null(result);
    });

    test("function returning void can explicitly return null", fn() {
        fn returns_void() -> Void {
            return null;
        }
        let result = returns_void();
        assert_null(result);
    });
});

describe("Function Type Annotations", fn() {
    test("function type annotation can be assigned", fn() {
        let fn_type: (Int, Int) -> Int;
        fn_type = fn(a, b) { a + b };
        assert_eq(fn_type(2, 3), 5);
    });

    test("function type with no parameters", fn() {
        let fn_type: () -> String;
        fn_type = fn() { "hello" };
        assert_eq(fn_type(), "hello");
    });

    test("function type with single parameter", fn() {
        let fn_type: (Int) -> Int;
        fn_type = fn(x) { x * 2 };
        assert_eq(fn_type(5), 10);
    });

    test("function type with different return type", fn() {
        let fn_type: (Int, Int) -> String;
        fn_type = fn(a, b) { str(a + b) };
        assert_eq(fn_type(2, 3), "5");
    });
});

describe("Nullable Type", fn() {
    test("nullable type accepts value", fn() {
        let x: Int? = 5;
        assert_eq(x, 5);
    });

    test("nullable type accepts null", fn() {
        let x: Int? = null;
        assert_null(x);
    });
});

describe("Array Type Annotation", fn() {
    test("array type annotation", fn() {
        let arr: Int[] = [1, 2, 3];
        assert_eq(len(arr), 3);
    });

    test("array type with strings", fn() {
        let arr: String[] = ["a", "b", "c"];
        assert_eq(len(arr), 3);
    });
});
