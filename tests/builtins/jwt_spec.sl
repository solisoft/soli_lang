// ============================================================================
// JWT Functions Test Suite
// ============================================================================

describe("JWT Functions", fn() {
    test("jwt_encode() and jwt_decode() work", fn() {
        let payload = hash();
        payload["user_id"] = 123;
        payload["role"] = "admin";

        let secret = "my-secret-key";
        let token = jwt_encode(payload, secret);
        assert_not_null(token);
        assert_contains(token, ".");

        let decoded = jwt_decode(token, secret);
        assert_eq(decoded["user_id"], 123);
        assert_eq(decoded["role"], "admin");
    });

    test("jwt_decode() fails with wrong secret", fn() {
        let payload = hash();
        payload["data"] = "test";
        let token = jwt_encode(payload, "secret1");
        let decoded = jwt_decode(token, "wrong-secret");
        assert_null(decoded);
    });

    test("jwt_encode() with expiration", fn() {
        let payload = hash();
        payload["exp"] = 9999999999;
        payload["data"] = "test";

        let secret = "test-secret";
        let token = jwt_encode(payload, secret);
        assert_not_null(token);
    });

    test("jwt_decode() returns null for expired token", fn() {
        let payload = hash();
        payload["exp"] = 0;
        payload["data"] = "test";

        let secret = "test-secret";
        let token = jwt_encode(payload, secret);
        let decoded = jwt_decode(token, secret);
        assert_null(decoded);
    });

    test("jwt_token has three parts", fn() {
        let payload = hash();
        payload["test"] = "data";
        let token = jwt_encode(payload, "secret");
        let parts = split(token, ".");
        assert_eq(len(parts), 3);
    });
});
