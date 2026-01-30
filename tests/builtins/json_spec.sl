// ============================================================================
// JSON Functions Test Suite
// ============================================================================

describe("JSON Functions", fn() {
    test("json_parse() parses JSON string", fn() {
        let obj = json_parse("{\"name\": \"test\", \"value\": 42}");
        assert_eq(obj["name"], "test");
        assert_eq(obj["value"], 42);
    });

    test("json_parse() parses arrays", fn() {
        let arr = json_parse("[1, 2, 3]");
        assert_eq(len(arr), 3);
        assert_eq(arr[0], 1);
    });

    test("json_parse() parses nested objects", fn() {
        let obj = json_parse("{\"data\": {\"items\": [1, 2, 3]}}");
        assert_eq(obj["data"]["items"][0], 1);
    });

    test("json_stringify() converts to JSON", fn() {
        let h = hash();
        h["name"] = "test";
        let json = json_stringify(h);
        assert_contains(json, "\"name\"");
        assert_contains(json, "\"test\"");
    });

    test("json_stringify() handles arrays", fn() {
        let arr = [1, 2, 3];
        let json = json_stringify(arr);
        // JSON output is compact without spaces
        assert_eq(json, "[1,2,3]");
    });

    test("json_stringify() handles nested structures", fn() {
        let h = hash();
        h["data"] = hash();
        h["data"]["items"] = [1, 2, 3];
        let json = json_stringify(h);
        assert_contains(json, "\"data\"");
        assert_contains(json, "items");
    });

    test("assert_json validates JSON strings", fn() {
        assert_json("{\"valid\": true}");
        assert_json("[1, 2, 3]");
    });

    test("json_parse() handles booleans", fn() {
        let obj = json_parse("{\"active\": true, \"deleted\": false}");
        assert_eq(obj["active"], true);
        assert_eq(obj["deleted"], false);
    });

    test("json_parse() handles null", fn() {
        let obj = json_parse("{\"value\": null}");
        assert_null(obj["value"]);
    });
});
