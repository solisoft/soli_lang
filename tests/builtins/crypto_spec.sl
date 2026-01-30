// ============================================================================
// Cryptography Functions Test Suite
// ============================================================================

describe("Cryptography Standalone Functions", fn() {
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

describe("Crypto Class Static Methods", fn() {
    test("Crypto.sha256() generates hash", fn() {
        let hash = Crypto.sha256("hello");
        assert_not_null(hash);
        assert_eq(len(hash), 64);  // SHA256 produces 32 bytes = 64 hex chars
    });

    test("Crypto.sha512() generates hash", fn() {
        let hash = Crypto.sha512("hello");
        assert_not_null(hash);
        assert_eq(len(hash), 128);  // SHA512 produces 64 bytes = 128 hex chars
    });

    test("Crypto.md5() generates hash", fn() {
        let hash = Crypto.md5("hello");
        assert_not_null(hash);
        assert_eq(len(hash), 32);  // MD5 produces 16 bytes = 32 hex chars
    });

    test("Crypto.hmac() generates MAC", fn() {
        let mac = Crypto.hmac("message", "secret");
        assert_not_null(mac);
        assert_eq(len(mac), 64);  // HMAC-SHA256 produces 32 bytes = 64 hex chars
    });

    test("Crypto.argon2_hash() and Crypto.argon2_verify() work", fn() {
        let password = "test_password";
        let hashed = Crypto.argon2_hash(password);
        assert_not_null(hashed);
        assert(Crypto.argon2_verify(password, hashed));
        assert_not(Crypto.argon2_verify("wrong_password", hashed));
    });

    test("Crypto.password_hash() and Crypto.password_verify() work", fn() {
        let password = "another_secret";
        let hashed = Crypto.password_hash(password);
        assert(Crypto.password_verify(password, hashed));
    });

    test("Crypto.x25519_keypair() generates key pair", fn() {
        let keypair = Crypto.x25519_keypair();
        assert_hash_has_key(keypair, "public");
        assert_hash_has_key(keypair, "private");
        assert_eq(len(keypair["public"]), 64);   // 32 bytes = 64 hex chars
        assert_eq(len(keypair["private"]), 64);
    });

    test("Crypto.ed25519_keypair() generates signing key pair", fn() {
        let keypair = Crypto.ed25519_keypair();
        assert_hash_has_key(keypair, "public");
        assert_hash_has_key(keypair, "private");
        assert_eq(len(keypair["public"]), 64);
        assert_eq(len(keypair["private"]), 64);
    });

    test("Crypto.base64_encode() and Crypto.base64_decode() work", fn() {
        let original = "Test data for base64";
        let encoded = Crypto.base64_encode(original);
        assert_not_null(encoded);
        let decoded = Crypto.base64_decode(encoded);
        assert_eq(decoded, original);
    });

    test("Crypto.x25519_shared_secret() computes shared secret", fn() {
        let alice = Crypto.x25519_keypair();
        let bob = Crypto.x25519_keypair();

        let alice_secret = Crypto.x25519_shared_secret(alice["private"], bob["public"]);
        let bob_secret = Crypto.x25519_shared_secret(bob["private"], alice["public"]);

        assert_eq(alice_secret, bob_secret);
    });

    test("Crypto.x25519_public_key() derives public key", fn() {
        let keypair = Crypto.x25519_keypair();
        let derived_public = Crypto.x25519_public_key(keypair["private"]);
        assert_eq(derived_public, keypair["public"]);
    });
});

describe("Hash Function Consistency", fn() {
    test("sha256 produces consistent results", fn() {
        let hash1 = sha256("test");
        let hash2 = sha256("test");
        assert_eq(hash1, hash2);
    });

    test("sha256 produces different results for different inputs", fn() {
        let hash1 = sha256("hello");
        let hash2 = sha256("world");
        assert_not(hash1 == hash2);
    });

    test("hmac produces consistent results", fn() {
        let mac1 = hmac("message", "key");
        let mac2 = hmac("message", "key");
        assert_eq(mac1, mac2);
    });

    test("hmac produces different results for different keys", fn() {
        let mac1 = hmac("message", "key1");
        let mac2 = hmac("message", "key2");
        assert_not(mac1 == mac2);
    });
});
