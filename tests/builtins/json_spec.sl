// ============================================================================
// JSON Class Test Suite
// ============================================================================

describe("JSON Class", fn() {
    test("JSON.parse() parses JSON string", fn() {
        let obj = JSON.parse("{\"name\": \"test\", \"value\": 42}");
        assert_eq(obj["name"], "test");
        assert_eq(obj["value"], 42);
    });

    test("JSON.parse() parses arrays", fn() {
        let arr = JSON.parse("[1, 2, 3]");
        assert_eq(len(arr), 3);
        assert_eq(arr[0], 1);
    });

    test("JSON.parse() parses nested objects", fn() {
        let obj = JSON.parse("{\"data\": {\"items\": [1, 2, 3]}}");
        assert_eq(obj["data"]["items"][0], 1);
    });

    test("JSON.stringify() converts to JSON", fn() {
        let h = hash();
        h["name"] = "test";
        let json = JSON.stringify(h);
        assert_contains(json, "\"name\"");
        assert_contains(json, "\"test\"");
    });

    test("JSON.stringify() handles arrays", fn() {
        let arr = [1, 2, 3];
        let json = JSON.stringify(arr);
        assert_eq(json, "[1,2,3]");
    });

    test("JSON.stringify() handles nested structures", fn() {
        let h = hash();
        h["data"] = hash();
        h["data"]["items"] = [1, 2, 3];
        let json = JSON.stringify(h);
        assert_contains(json, "\"data\"");
        assert_contains(json, "items");
    });

    test("assert_json validates JSON strings", fn() {
        assert_json("{\"valid\": true}");
        assert_json("[1, 2, 3]");
    });

    test("JSON.parse() handles booleans", fn() {
        let obj = JSON.parse("{\"active\": true, \"deleted\": false}");
        assert_eq(obj["active"], true);
        assert_eq(obj["deleted"], false);
    });

    test("JSON.parse() handles null", fn() {
        let obj = JSON.parse("{\"value\": null}");
        assert_null(obj["value"]);
    });
});
