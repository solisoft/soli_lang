// ============================================================================
// String Interpolation Test Suite
// ============================================================================

describe("String Interpolation", fn() {
    test("basic interpolation with \\()", fn() {
        let name = "World";
        let greeting = "Hello \(name)!";
        assert_eq(greeting, "Hello World!");
    });

    test("interpolation with expressions", fn() {
        let a = 2;
        let b = 3;
        let result = "Sum is \(a + b)";
        assert_eq(result, "Sum is 5");
    });

    test("multiple interpolations", fn() {
        let first = "John";
        let last = "Doe";
        let full = "\(first) \(last)";
        assert_eq(full, "John Doe");
    });

    test("nested expression in interpolation", fn() {
        let x = 10;
        let msg = "Double is \(x * 2)";
        assert_eq(msg, "Double is 20");
    });

    test("interpolation with function call", fn() {
        fn get_name() {
            return "Alice";
        }
        let greeting = "Hello \(get_name())!";
        assert_eq(greeting, "Hello Alice!");
    });

    test("interpolation with method call", fn() {
        let text = "hello";
        let result = "Uppercase: \(text.upcase())";
        assert_eq(result, "Uppercase: HELLO");
    });

    test("interpolation with array access", fn() {
        let names = ["Alice", "Bob"];
        let result = "First: \(names[0])";
        assert_eq(result, "First: Alice");
    });

    test("interpolation with hash access", fn() {
        let person = {name: "Charlie"};
        let result = "Name: \(person["name"])";
        assert_eq(result, "Name: Charlie");
    });

    test("interpolation with ternary", fn() {
        let x = 5;
        let result = "Value is \(x > 10 ? "big" : "small")";
        assert_eq(result, "Value is small");
    });
});
