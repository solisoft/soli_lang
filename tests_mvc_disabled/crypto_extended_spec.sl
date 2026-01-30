// ============================================================================
// Crypto Extended Test Suite
// ============================================================================
// Additional tests for cryptographic functions
// ============================================================================

describe("X25519 Key Exchange", fn() {
    test("x25519_keypair() generates valid key pair", fn() {
        let keypair = x25519_keypair();
        assert_hash_has_key(keypair, "public");
        assert_hash_has_key(keypair, "private");
        assert(len(keypair["public"]) == 64);
        assert(len(keypair["private"]) == 128);
    });

    test("x25519_shared_secret() computes shared secret", fn() {
        let alice = x25519_keypair();
        let bob = x25519_keypair();

        let secret1 = x25519_shared_secret(alice["private"], bob["public"]);
        let secret2 = x25519_shared_secret(bob["private"], alice["public"]);

        assert_eq(secret1, secret2);
        assert(len(secret1) == 64);
    });

    test("x25519_public_key() derives from private", fn() {
        let keypair = x25519_keypair();
        let pub_key = x25519_public_key(keypair["private"]);

        assert_eq(pub_key, keypair["public"]);
    });

    test("x25519() performs scalar multiplication", fn() {
        let keypair = x25519_keypair();
        let basepoint = "0900000000000000000000000000000000000000000000000000000000000000";

        let result = x25519(basepoint, keypair["private"]);
        assert(len(result) == 64);
    });

    test("x25519_shared_secret() fails with invalid key", fn() {
        let result = x25519_shared_secret("invalid", "invalid");
        assert_null(result);
    });
});

describe("Ed25519 Digital Signatures", fn() {
    test("ed25519_keypair() generates key pair", fn() {
        let keypair = ed25519_keypair();
        assert_hash_has_key(keypair, "public");
        assert_hash_has_key(keypair, "private");
        assert(len(keypair["public"]) == 64);
        assert(len(keypair["private"]) == 128);
    });

    test("ed25519_sign() creates signature", fn() {
        let keypair = ed25519_keypair();
        let message = "Test message for signing";

        let signature = ed25519_sign(message, keypair["private"]);
        assert_not_null(signature);
        assert(len(signature) == 128);
    });

    test("ed25519_verify() validates signature", fn() {
        let keypair = ed25519_keypair();
        let message = "Test message";
        let signature = ed25519_sign(message, keypair["private"]);

        let result = ed25519_verify(message, signature, keypair["public"]);
        assert(result);
    });

    test("ed25519_verify() rejects invalid signature", fn() {
        let keypair = ed25519_keypair();
        let message = "Original message";
        let wrong_message = "Tampered message";
        let signature = ed25519_sign(message, keypair["private"]);

        let result = ed25519_verify(wrong_message, signature, keypair["public"]);
        assert_not(result);
    });

    test("ed25519_sign() with empty message", fn() {
        let keypair = ed25519_keypair();
        let signature = ed25519_sign("", keypair["private"]);
        assert_not_null(signature);
    });
});

describe("Additional Hash Functions", fn() {
    test("sha256() produces correct hash length", fn() {
        let hash = sha256("hello");
        assert(len(hash) == 64);
    });

    test("sha512() produces correct hash length", fn() {
        let hash = sha512("hello");
        assert(len(hash) == 128);
    });

    test("sha256() is deterministic", fn() {
        let hash1 = sha256("test");
        let hash2 = sha256("test");
        assert_eq(hash1, hash2);
    });

    test("sha256() produces different hashes for different inputs", fn() {
        let hash1 = sha256("hello");
        let hash2 = sha256("world");
        assert_ne(hash1, hash2);
    });

    test("md5() produces correct hash length", fn() {
        let hash = md5("hello");
        assert(len(hash) == 32);
    });
});

describe("HMAC Functions", fn() {
    test("hmac() produces valid MAC", fn() {
        let mac = hmac("message", "secret");
        assert(len(mac) == 64);
    });

    test("hmac_sha256() produces MAC", fn() {
        let mac = hmac_sha256("message", "secret");
        assert(len(mac) == 64);
    });

    test("hmac_sha512() produces MAC", fn() {
        let mac = hmac_sha512("message", "secret");
        assert(len(mac) == 128);
    });

    test("hmac() is deterministic", fn() {
        let mac1 = hmac("msg", "key");
        let mac2 = hmac("msg", "key");
        assert_eq(mac1, mac2);
    });

    test("hmac() different keys produce different results", fn() {
        let mac1 = hmac("message", "key1");
        let mac2 = hmac("message", "key2");
        assert_ne(mac1, mac2);
    });
});

describe("Base Encoding Functions", fn() {
    test("base64_encode() encodes correctly", fn() {
        let encoded = Base64.encode("hello world");
        assert_not_null(encoded);
    });

    test("base64_decode() decodes correctly", fn() {
        let original = "Hello, World!";
        let encoded = Base64.encode(original);
        let decoded = Base64.decode(encoded);
        assert_eq(decoded, original);
    });

    test("base64_encode() handles binary data", fn() {
        let data = [0, 1, 2, 255, 254, 253];
        let encoded = Base64.encode(data);
        let decoded = Base64.decode(encoded);
        assert_eq(len(decoded), len(data));
    });

    test("base64url_encode() produces URL-safe output", fn() {
        let encoded = base64url_encode("hello+world/test");
        assert_not_contains(encoded, "+");
        assert_not_contains(encoded, "/");
    });

    test("hex_encode() encodes to hex", fn() {
        let encoded = hex_encode("hello");
        assert_not_null(encoded);
    });

    test("hex_decode() decodes from hex", fn() {
        let original = "hello";
        let encoded = hex_encode(original);
        let decoded = hex_decode(encoded);
        assert_eq(decoded, original);
    });
});

describe("Cryptographic Randomness", fn() {
    test("random_bytes() produces bytes", fn() {
        let bytes = random_bytes(32);
        assert_eq(len(bytes), 32);
    });

    test("random_bytes() different each time", fn() {
        let bytes1 = random_bytes(16);
        let bytes2 = random_bytes(16);
        assert_ne(bytes1, bytes2);
    });

    test("random_string() produces random string", fn() {
        let str = random_string(32);
        assert_eq(len(str), 32);
    });

    test("random_int() within range", fn() {
        let n = random_int(1, 100);
        assert(n >= 1);
        assert(n <= 100);
    });
});

describe("Key Derivation", fn() {
    test("bcrypt() hashes password", fn() {
        let hash = bcrypt("password");
        assert_contains(hash, "$2b$");
    });

    test("bcrypt_verify() verifies password", fn() {
        let hash = bcrypt("secret");
        assert(bcrypt_verify("secret", hash));
        assert_not(bcrypt_verify("wrong", hash));
    });
});

describe("Cryptographic Utility Functions", fn() {
    test("constant_time_eq() compares in constant time", fn() {
        assert(constant_time_eq("abc", "abc"));
        assert_not(constant_time_eq("abc", "def"));
    });

    test("secure_compare() compares securely", fn() {
        assert(secure_compare("test", "test"));
        assert_not(secure_compare("test", "TEST"));
    });
});
