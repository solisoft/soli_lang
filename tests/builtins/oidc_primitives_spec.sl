// ============================================================================
// OIDC Primitive Composition Test Suite
//
// These cover the *compositions* an OpenID Connect provider is built from —
// PKCE challenges, JWK thumbprints, id_token claims — rather than the
// individual builtins, which have their own unit tests. Each path here crosses
// several builtins, so a break in the seam between them would otherwise slip
// through every per-builtin test.
// ============================================================================

// Unpadded base64url of the raw bytes behind a hex digest. `Crypto.sha256`
// returns hex, so it must be decoded before encoding — base64url of the *hex
// text* is a different (and wrong) value.
def base64url_digest(hex_digest) {
    return Base64.urlsafe_encode(Hex.decode(hex_digest));
}

// A `let` at describe scope is not visible inside the test closures, so the
// shared secret is a top-level function instead.
def test_secret {
    return "0123456789abcdef0123456789abcdef";
}

describe("PKCE (RFC 7636)", fn() {
    test("S256 challenge matches the RFC 7636 Appendix B vector", fn() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = base64url_digest(Crypto.sha256(verifier));

        assert_eq(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    });

    test("challenge is url-safe and unpadded", fn() {
        let challenge = base64url_digest(Crypto.sha256(Crypto.random_token()));

        assert_not(challenge.contains("+"));
        assert_not(challenge.contains("/"));
        assert_not(challenge.contains("="));
    });

    test("a wrong verifier does not reproduce the challenge", fn() {
        let challenge = base64url_digest(Crypto.sha256("the-real-verifier"));
        let forged = base64url_digest(Crypto.sha256("the-wrong-verifier"));

        assert_not(Crypto.secure_compare(challenge, forged));
    });
});

describe("JWK thumbprint (RFC 7638)", fn() {
    test("canonical_json emits the required lexicographic member ordering", fn() {
        // RFC 7638 §3.3: for an RSA key the hash input is exactly the members
        // e, kty, n — sorted, no whitespace.
        let members = {"n": "0vx7ag", "kty": "RSA", "e": "AQAB"};

        assert_eq(Crypto.canonical_json(members), "{\"e\":\"AQAB\",\"kty\":\"RSA\",\"n\":\"0vx7ag\"}");
    });

    test("thumbprint is insertion-order independent", fn() {
        let one = {"e": "AQAB", "kty": "RSA", "n": "0vx7ag"};
        let two = {"n": "0vx7ag", "e": "AQAB", "kty": "RSA"};

        assert_eq(base64url_digest(Crypto.sha256(Crypto.canonical_json(one))),
                  base64url_digest(Crypto.sha256(Crypto.canonical_json(two))));
    });
});

describe("Secure token generation", fn() {
    test("random_token() defaults to 256 bits", fn() {
        // 32 bytes of base64url with no padding is 43 characters.
        assert_eq(Crypto.random_token().length(), 43);
    });

    test("random_hex(n) counts bytes, not characters", fn() {
        assert_eq(Crypto.random_hex(32).length(), 64);
    });

    test("tokens do not repeat", fn() {
        assert_not(Crypto.random_token() == Crypto.random_token());
        assert_not(Crypto.random_hex(16) == Crypto.random_hex(16));
    });

    test("random_bytes(n) returns n bytes in range", fn() {
        let bytes = Crypto.random_bytes(16);

        assert_eq(len(bytes), 16);
        bytes.each(fn(b) assert(b >= 0 && b <= 255));
    });
});

describe("id_token composition", fn() {
    test("registered claims and the kid header survive a round trip", fn() {
        let token = jwt_sign({"sub": "user-1"}, test_secret(), {
            "expires_in": 600,
            "kid": "key-1",
            "aud": "client-1",
            "iss": "https://op.example",
            "jti": "jti-1"
        });

        let header = JSON.parse(Base64.urlsafe_decode(token.split(".")[0]));
        assert_eq(header["kid"], "key-1");

        let claims = jwt_verify(token, test_secret(), {
            "audience": "client-1",
            "issuer": "https://op.example"
        });
        assert_null(claims["error"]);
        assert_eq(claims["sub"], "user-1");
        assert_eq(claims["aud"], "client-1");
        assert_eq(claims["iss"], "https://op.example");
        assert_eq(claims["jti"], "jti-1");
    });

    test("a token minted for another client is rejected", fn() {
        let token = jwt_sign({"sub": "user-1"}, test_secret(), {
            "expires_in": 600,
            "aud": "client-1"
        });

        assert_eq(jwt_verify(token, test_secret(), {"audience": "client-2"})["error"], true);
    });

    test("a token from another issuer is rejected", fn() {
        let token = jwt_sign({"sub": "user-1"}, test_secret(), {
            "expires_in": 600,
            "iss": "https://evil.example"
        });

        assert_eq(jwt_verify(token, test_secret(), {"issuer": "https://op.example"})["error"], true);
    });

    test("at_hash is the left-most 128 bits of the access token digest", fn() {
        let access_token = Crypto.random_token();
        let digest = Hex.decode(Crypto.sha256(access_token));
        let at_hash = Base64.urlsafe_encode(digest.take(16));

        // 16 bytes of base64url with no padding is 22 characters.
        assert_eq(at_hash.length(), 22);
        assert_eq(len(digest), 32);
    });
});
