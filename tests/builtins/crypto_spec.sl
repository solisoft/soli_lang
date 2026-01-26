// ============================================================================
// Cryptography Functions Test Suite
// ============================================================================

describe("Cryptography Functions", fn() {
    test("argon2_hash() and argon2_verify() work", fn() {
        let password = "secret123";
        let hashed = argon2_hash(password);
        assert_not_null(hashed);
        assert(argon2_verify(password, hashed));
        assert_not(argon2_verify("wrong", hashed));
    });

    test("password_hash() and password_verify() aliases work", fn() {
        let password = "mysecret";
        let hashed = password_hash(password);
        assert(password_verify(password, hashed));
    });

    test("x25519_keypair() generates key pair", fn() {
        let keypair = x25519_keypair();
        assert_hash_has_key(keypair, "public");
        assert_hash_has_key(keypair, "private");
        assert(len(keypair["public"]) > 0);
        assert(len(keypair["private"]) > 0);
    });

    test("ed25519_keypair() generates signing key pair", fn() {
        let keypair = ed25519_keypair();
        assert_hash_has_key(keypair, "public");
        assert_hash_has_key(keypair, "private");
    });

    test("sha256() generates hash", fn() {
        let hash = sha256("hello");
        assert_not_null(hash);
        assert(len(hash) > 0);
    });

    test("sha512() generates hash", fn() {
        let hash = sha512("hello");
        assert_not_null(hash);
        assert(len(hash) > 0);
    });

    test("md5() generates hash", fn() {
        let hash = md5("hello");
        assert_not_null(hash);
        assert_eq(len(hash), 32);
    });

    test("hmac() generates MAC", fn() {
        let mac = hmac("message", "secret");
        assert_not_null(mac);
        assert(len(mac) > 0);
    });

    test("base64_encode() and base64_decode() work", fn() {
        let original = "hello world";
        let encoded = base64_encode(original);
        assert_not_null(encoded);
        let decoded = base64_decode(encoded);
        assert_eq(decoded, original);
    });
});
