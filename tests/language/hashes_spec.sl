// ============================================================================
// Hashes Test Suite
// ============================================================================

describe("Hash Operations", fn() {
    test("hash key access", fn() {
        let h = {"name" => "Alice", "age" => 30};
        assert_eq(h["name"], "Alice");
        assert_eq(h["age"], 30);
    });

    test("hash key assignment", fn() {
        let h = hash();
        h["key"] = "value";
        assert_eq(h["key"], "value");
    });

    test("hash with integer keys", fn() {
        let h = {1 => "one", 2 => "two"};
        assert_eq(h[1], "one");
        assert_eq(h[2], "two");
    });

    test("hash dot notation for string keys", fn() {
        let h = {name: "Bob"};
        assert_eq(h["name"], "Bob");
    });

    test("nested hash access", fn() {
        let h = {
            "person" => {
                "name" => "Alice",
                "address" => {
                    "city" => "NYC"
                }
            }
        };
        assert_eq(h["person"]["name"], "Alice");
        assert_eq(h["person"]["address"]["city"], "NYC");
    });

    test("hash key with spaces", fn() {
        let h = {"first name" => "John", "last name" => "Doe"};
        assert_eq(h["first name"], "John");
        assert_eq(h["last name"], "Doe");
    });

    test("hash with boolean keys", fn() {
        let h = {true => "yes", false => "no"};
        assert_eq(h[true], "yes");
        assert_eq(h[false], "no");
    });
});
