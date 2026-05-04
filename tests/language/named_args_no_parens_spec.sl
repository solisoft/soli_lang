// ============================================================================
// Tests for Ruby-style method calls without parentheses
// ============================================================================

describe("Method call with named args, no parens", fn() {
    test("single named arg on function", fn() {
        fn greet(name: String) { "Hello, " + name + "!" }
        let result = greet name: "Alice";
        assert_eq(result, "Hello, Alice!");
    });

    test("multiple named args on function", fn() {
        fn configure(host: String, port: Int, debug: Bool) {
            host + ":" + str(port) + " (debug: " + str(debug) + ")"
        }
        let result = configure host: "example.com", port: 3000, debug: true;
        assert_eq(result, "example.com:3000 (debug: true)");
    });

    test("method call on object with named args", fn() {
        class User {
            name: String;
            age: Int;

            new() {
                this.name = "default";
                this.age = 0;
            }

            fn update(name: String, age: Int) {
                this.name = name;
                this.age = age;
                "updated"
            }
        }
        let u = new User();
        let result = u.update name: "Bob", age: 25;
        assert_eq(result, "updated");
    });

    test("named args with comma separator", fn() {
        fn add(a: Int, b: Int, c: Int) { a + b + c }
        let result = add a: 1, b: 2, c: 3;
        assert_eq(result, 6);
    });
});