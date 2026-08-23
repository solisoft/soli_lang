// PASETO v4 tokens through the interpreter's class dispatch.
//
// The Rust unit tests in src/interpreter/builtins/paseto.rs call the native
// functions directly; this spec goes through `Paseto.*` the way an app does, so
// a registration or dispatch regression fails here even when the native side is
// fine.

describe("Paseto keys", fn() {
    test("generate_local_key returns a k4.local PASERK string", fn() {
        let key = Paseto.generate_local_key();
        assert(key.starts_with("k4.local."));
    });

    test("generate_key_pair returns both halves, purpose-tagged", fn() {
        let pair = Paseto.generate_key_pair();
        assert(pair["secret"].starts_with("k4.secret."));
        assert(pair["public"].starts_with("k4.public."));
    });

    test("public_key derives the verifying half from the secret", fn() {
        let pair = Paseto.generate_key_pair();
        assert_eq(Paseto.public_key(pair["secret"]), pair["public"]);
    });

    test("key_id is a PASERK id, distinct per purpose", fn() {
        let pair = Paseto.generate_key_pair();
        assert(Paseto.key_id(pair["public"]).starts_with("k4.pid."));
        assert(Paseto.key_id(Paseto.generate_local_key()).starts_with("k4.lid."));
    });

    test("key_id refuses a raw hex key, which carries no purpose", fn() {
        let failed = false;
        try {
            Paseto.key_id("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef");
        } catch error {
            failed = true;
        }
        assert(failed);
    });
});

describe("Paseto local tokens", fn() {
    test("round-trips custom and registered claims", fn() {
        let key = Paseto.generate_local_key();
        let token = Paseto.encrypt({ "user_id": 42, "role": "admin" }, key, { "expires_in": 900 });
        assert(token.starts_with("v4.local."));

        let claims = Paseto.decrypt(token, key);
        assert_eq(claims["user_id"], 42);
        assert_eq(claims["role"], "admin");
        // PASETO dates are RFC 3339 strings, not Unix ints.
        assert(claims["exp"].contains("T"));
    });

    test("a raw 64-hex key works, so `openssl rand -hex 32` does", fn() {
        let key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let claims = Paseto.decrypt(Paseto.encrypt({ "sub": "alice" }, key), key);
        assert_eq(claims["sub"], "alice");
    });

    test("the wrong key is rejected", fn() {
        let token = Paseto.encrypt({ "sub": "alice" }, Paseto.generate_local_key());
        let claims = Paseto.decrypt(token, Paseto.generate_local_key()) rescue null;
        assert_null(claims);
    });

    test("an expired token is rejected", fn() {
        let key = Paseto.generate_local_key();
        let token = Paseto.encrypt({ "sub": "alice" }, key, { "exp": 1000000000 });
        assert_null(Paseto.decrypt(token, key) rescue null);
    });

    test("a payload is unreadable without the key", fn() {
        let token = Paseto.encrypt({ "secret_note": "classified" }, Paseto.generate_local_key());
        let peek = Paseto.decode_unsafe(token);
        assert_eq(peek["purpose"], "local");
        assert_null(peek["claims"]);
    });
});

describe("Paseto public tokens", fn() {
    test("verifies with the public half", fn() {
        let pair = Paseto.generate_key_pair();
        let token = Paseto.sign({ "sub": "bob" }, pair["secret"], { "expires_in": 600 });
        assert(token.starts_with("v4.public."));
        assert_eq(Paseto.verify(token, pair["public"])["sub"], "bob");
    });

    test("does not verify under another key pair", fn() {
        let pair = Paseto.generate_key_pair();
        let token = Paseto.sign({ "sub": "bob" }, pair["secret"]);
        assert_null(Paseto.verify(token, Paseto.generate_key_pair()["public"]) rescue null);
    });

    test("audience and issuer are checked when expected", fn() {
        let pair = Paseto.generate_key_pair();
        let token = Paseto.sign({ "sub": "bob" }, pair["secret"], {
            "aud": "api.example.com",
            "iss": "https://issuer.test"
        });

        let claims = Paseto.verify(token, pair["public"], {
            "audience": "api.example.com",
            "issuer": "https://issuer.test"
        });
        assert_eq(claims["aud"], "api.example.com");

        let wrong = Paseto.verify(token, pair["public"], { "audience": "other.example.com" }) rescue null;
        assert_null(wrong);
    });

    test("verifying with the secret key is refused", fn() {
        let pair = Paseto.generate_key_pair();
        let token = Paseto.sign({ "sub": "bob" }, pair["secret"]);
        assert_null(Paseto.verify(token, pair["secret"]) rescue null);
    });
});

describe("Paseto purposes and options", fn() {
    test("a local token cannot be verified as a signed one, or the reverse", fn() {
        let key = Paseto.generate_local_key();
        let pair = Paseto.generate_key_pair();
        let local_token = Paseto.encrypt({ "sub": "a" }, key);
        let signed_token = Paseto.sign({ "sub": "a" }, pair["secret"]);

        assert_null(Paseto.verify(local_token, pair["public"]) rescue null);
        assert_null(Paseto.decrypt(signed_token, key) rescue null);
    });

    test("a non-expiring token must be opted into on both ends", fn() {
        let key = Paseto.generate_local_key();
        let token = Paseto.encrypt({ "sub": "a" }, key, { "non_expiring": true });

        assert_null(Paseto.decrypt(token, key) rescue null);
        let claims = Paseto.decrypt(token, key, { "allow_non_expiring": true });
        assert_eq(claims["sub"], "a");
    });

    test("a token minted without options still expires", fn() {
        // Documented default: 3600s. Forgetting `expires_in` must yield a
        // short-lived token, never an eternal one.
        let key = Paseto.generate_local_key();
        let claims = Paseto.decrypt(Paseto.encrypt({ "sub": "a" }, key), key);
        assert(claims["exp"].present?);
        assert(claims["iat"].present?);
        assert(claims["nbf"].present?);
    });

    test("aud is a single string — the JWT array form is refused, not dropped", fn() {
        let key = Paseto.generate_local_key();
        assert_null(Paseto.encrypt({ "sub": "a" }, key, { "aud": ["a", "b"] }) rescue null);
    });

    test("an unknown option raises rather than being ignored", fn() {
        let key = Paseto.generate_local_key();
        assert_null(Paseto.encrypt({ "sub": "a" }, key, { "expires": 60 }) rescue null);
        let token = Paseto.encrypt({ "sub": "a" }, key);
        assert_null(Paseto.decrypt(token, key, { "audiance": "api" }) rescue null);
    });

    test("an implicit assertion must match, and never travels in the token", fn() {
        let key = Paseto.generate_local_key();
        let token = Paseto.encrypt({ "sub": "a" }, key, { "implicit": "session-42" });

        assert(!token.contains("session-42"));
        assert_eq(Paseto.decrypt(token, key, { "implicit": "session-42" })["sub"], "a");
        assert_null(Paseto.decrypt(token, key, { "implicit": "session-99" }) rescue null);
    });
});

describe("Paseto key rotation", fn() {
    test("the footer kid picks the key before verification", fn() {
        let current = Paseto.generate_key_pair();
        let previous = Paseto.generate_key_pair();
        // A token still in flight, signed with the key being rotated out.
        let token = Paseto.sign({ "sub": "alice" }, previous["secret"], {
            "kid": Paseto.key_id(previous["public"])
        });

        let kid = Paseto.decode_unsafe(token)["footer"]["kid"];
        let key = null;
        for candidate in [current["public"], previous["public"]] {
            key = candidate if Paseto.key_id(candidate) == kid;
        }
        assert_eq(key, previous["public"]);
        assert_eq(Paseto.verify(token, key)["sub"], "alice");
    });

    test("decode_unsafe keeps unverified claims out of the top level", fn() {
        let pair = Paseto.generate_key_pair();
        let token = Paseto.sign({ "sub": "alice" }, pair["secret"]);
        let peek = Paseto.decode_unsafe(token);

        assert_eq(peek["unverified"], true);
        assert_eq(peek["claims"]["sub"], "alice");
        // Reaching for the claim directly must not yield a trusted-looking value.
        assert_null(peek["sub"]);
    });

    test("a tampered token fails", fn() {
        let pair = Paseto.generate_key_pair();
        let token = Paseto.sign({ "sub": "alice" }, pair["secret"]);
        let tampered = token.substring(0, token.length() - 4) + "AAAA";
        assert_null(Paseto.verify(tampered, pair["public"]) rescue null);
    });
});
