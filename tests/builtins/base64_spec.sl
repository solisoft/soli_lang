// ============================================================================
// Base64 Test Suite
// ============================================================================

describe("Base64", fn() {
    test("encode string", fn() {
        let encoded = Base64.encode("Hello World");
        assert_eq(encoded, "SGVsbG8gV29ybGQ=");
    });

    test("decode string", fn() {
        let decoded = Base64.decode("SGVsbG8gV29ybGQ=");
        assert_eq(decoded, "Hello World");
    });

    test("encode and decode roundtrip", fn() {
        let original = "Test string with special chars: !@#$%";
        let encoded = Base64.encode(original);
        let decoded = Base64.decode(encoded);
        assert_eq(decoded, original);
    });

    test("encode empty string", fn() {
        let encoded = Base64.encode("");
        assert_eq(encoded, "");
    });

    test("decode empty string", fn() {
        let decoded = Base64.decode("");
        assert_eq(decoded, "");
    });

    test("encode numbers as string", fn() {
        let encoded = Base64.encode("12345");
        assert_eq(encoded, "MTIzNDU=");
        let decoded = Base64.decode(encoded);
        assert_eq(decoded, "12345");
    });

    test("encode unicode characters", fn() {
        let encoded = Base64.encode("héllo");
        let decoded = Base64.decode(encoded);
        assert_eq(decoded, "héllo");
    });
});
