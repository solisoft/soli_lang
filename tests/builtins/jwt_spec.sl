describe("JWT", fn() {
    test("jwt_sign returns a token string", fn() {
        let secret = "this-is-a-32-byte-secret-for-test!";
        let payload = {"sub": "alice", "role": "admin"};
        let token = jwt_sign(payload, secret);
        assert(token.is_a?("string"));
        assert(len(token) > 0);
    });

    test("jwt_sign with expires_in option", fn() {
        let secret = "this-is-a-32-byte-secret-for-test!";
        let payload = {"sub": "bob"};
        let token = jwt_sign(payload, secret, {"expires_in": 3600});
        assert(token.is_a?("string"));
    });

    test("jwt_verify returns claims on valid token", fn() {
        let secret = "this-is-a-32-byte-secret-for-test!";
        let payload = {"sub": "alice", "role": "admin"};
        let token = jwt_sign(payload, secret, {"expires_in": 3600});
        let claims = jwt_verify(token, secret);
        assert_eq(claims["sub"], "alice");
        assert_eq(claims["role"], "admin");
        assert(claims["iat"] > 0);
        assert(claims["exp"] > 0);
    });

    test("jwt_verify with explicit algorithm", fn() {
        let secret = "this-is-a-32-byte-secret-for-test!";
        let payload = {"sub": "bob"};
        let token = jwt_sign(payload, secret, {"expires_in": 3600, "algorithm": "HS256"});
        let claims = jwt_verify(token, secret, {"algorithm": "HS256"});
        assert_eq(claims["sub"], "bob");
    });

    test("jwt_verify rejects tampered token", fn() {
        let secret = "this-is-a-32-byte-secret-for-test!";
        let payload = {"sub": "alice"};
        let token = jwt_sign(payload, secret, {"expires_in": 3600});
        let tampered = token + "x";
        let result = jwt_verify(tampered, secret);
        assert(result["error"] == true);
        assert(result["message"].is_a?("string"));
    });

    test("jwt_verify rejects token with wrong secret", fn() {
        let secret = "this-is-a-32-byte-secret-for-test!";
        let payload = {"sub": "alice"};
        let token = jwt_sign(payload, secret, {"expires_in": 3600});
        let result = jwt_verify(token, "different-secret-thats-also-32-bytes-long!");
        assert(result["error"] == true);
    });

    test("jwt_verify rejects token with mismatched algorithm", fn() {
        let secret = "this-is-a-32-byte-secret-for-test!";
        let payload = {"sub": "alice"};
        let token = jwt_sign(payload, secret, {"expires_in": 3600, "algorithm": "HS256"});
        let caught = false;
        try {
            jwt_verify(token, secret, {"algorithm": "HS512"});
        } catch (e) {
            caught = true;
        }
        assert(caught);
    });

    test("jwt_decode returns error (removed)", fn() {
        let caught = false;
        try {
            jwt_decode("any.token.value");
        } catch (e) {
            caught = true;
        }
        assert(caught);
    });

    test("jwt_decode_unsafe returns wrapped claims", fn() {
        let secret = "this-is-a-32-byte-secret-for-test!";
        let payload = {"sub": "alice"};
        let token = jwt_sign(payload, secret, {"expires_in": 3600});
        let result = jwt_decode_unsafe(token);
        assert(result["unverified"] == true);
        assert(result["claims"]["sub"] == "alice");
    });

    test("jwt_sign rejects short secret", fn() {
        let caught = false;
        try {
            jwt_sign({"sub": "test"}, "too-short");
        } catch (e) {
            caught = true;
        }
        assert(caught);
    });

    test("jwt_verify rejects short secret", fn() {
        let caught = false;
        try {
            jwt_verify("any.token.value", "too-short");
        } catch (e) {
            caught = true;
        }
        assert(caught);
    });

    test("jwt_sign expects string secret", fn() {
        let caught = false;
        try {
            jwt_sign({"sub": "test"}, 123);
        } catch (e) {
            caught = true;
        }
        assert(caught);
    });

    test("jwt_verify expects string token", fn() {
        let secret = "this-is-a-32-byte-secret-for-test!";
        let caught = false;
        try {
            jwt_verify(123, secret);
        } catch (e) {
            caught = true;
        }
        assert(caught);
    });

    test("jwt_verify with 4 arguments is rejected", fn() {
        let caught = false;
        try {
            jwt_verify("t", "s", {}, "extra");
        } catch (e) {
            caught = true;
        }
        assert(caught);
    });
});
