// ============================================================================
// Pipeline Operator Test Suite
// ============================================================================

describe("Pipeline Operator", fn() {
    test("basic pipeline", fn() {
        fn double(x) { return x * 2; }
        fn addOne(x) { return x + 1; }

        let result = 5 |> double();
        assert_eq(result, 10);
    });

    test("chained pipeline", fn() {
        fn double(x) { return x * 2; }
        fn addTen(x) { return x + 10; }

        let result = 5 |> double() |> addTen();
        assert_eq(result, 20);
    });

    test("pipeline with method call", fn() {
        let result = "hello" |> fn(s) { return s.upcase(); };
        assert_eq(result, "HELLO");
    });

    test("pipeline with custom function", fn() {
        fn square(x) { return x * x; }
        fn toString(x) { return str(x); }

        let result = 5 |> square() |> toString();
        assert_eq(result, "25");
    });

    test("pipeline with multiple transformations", fn() {
        fn add(x, n) { return x + n; }
        fn multiply(x, n) { return x * n; }

        let result = 2 |> add(3) |> multiply(4);
        assert_eq(result, 20);
    });

    test("pipeline preserves types", fn() {
        fn get_len(s) { return len(s); }

        let result = "hello" |> get_len();
        assert_eq(result, 5);
        assert_eq(type(result), "int");
    });
});
