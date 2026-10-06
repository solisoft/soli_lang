# JWT builtins: jwt_sign / jwt_verify / jwt_decode_unsafe — claims, registered
# claim options, verification failures and argument checks.

const SECRET = "this-is-a-32-byte-secret-for-test!"
const OTHER_SECRET = "different-secret-thats-also-32-bytes-long!"

# base64url header segments: {"typ":"JWT","alg":"HS256"}, then HS384, HS512,
# and HS256 with "kid":"k1"
const HS256_HEADER = "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9"
const HS384_HEADER = "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzM4NCJ9"
const HS512_HEADER = "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzUxMiJ9"
const KID_HEADER = "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiIsImtpZCI6ImsxIn0"

describe("JWT") do
  context("jwt_sign") do
    test("returns three base64url segments with an HS256 JWT header") do
      token = jwt_sign({"sub": "alice", "role": "admin"}, SECRET, {"expires_in": 3600})
      assert_eq(token.split(".").length, 3)
      assert_match(token, "^[A-Za-z0-9_-]+\\.[A-Za-z0-9_-]+\\.[A-Za-z0-9_-]+$")
      assert_eq(token.split(".")[0], HS256_HEADER)
    end

    test("names the chosen algorithm in the header") do
      assert_eq(jwt_sign({"sub": "a"}, SECRET, {"algorithm": "HS512"}).split(".")[0], HS512_HEADER)
      assert_eq(jwt_sign({"sub": "a"}, SECRET, {"algorithm": "HS384"}).split(".")[0], HS384_HEADER)
    end

    test("puts a kid in the header") do
      token = jwt_sign({"sub": "a"}, SECRET, {"expires_in": 60, "kid": "k1"})
      assert_eq(token.split(".")[0], KID_HEADER)
    end

    test("expires_in sets exp relative to iat") do
      claims = jwt_decode_unsafe(jwt_sign({"sub": "bob"}, SECRET, {"expires_in": 3600}))["claims"]
      assert_eq(claims["exp"] - claims["iat"], 3600)
      assert_eq(claims["sub"], "bob")
    end

    test("the exp option is an absolute timestamp") do
      claims = jwt_decode_unsafe(jwt_sign({"sub": "a"}, SECRET, {"exp": 4102444800}))["claims"]
      assert_eq(claims["exp"], 4102444800)
    end

    test("an exp in the payload is ignored: registered claims come from the options") do
      claims = jwt_decode_unsafe(jwt_sign({"sub": "a", "exp": 1000}, SECRET))["claims"]
      assert_not(claims.has_key("exp"))
    end

    test("refuses exp and expires_in together") do
      assert_raises("accepts either `exp` (absolute timestamp) or `expires_in`") do
        jwt_sign({"sub": "a"}, SECRET, {"exp": 4102444800, "expires_in": 60})
      end
    end

    test("refuses a secret shorter than 32 bytes") do
      assert_raises("jwt_sign() secret must be at least 32 bytes for security (got 9)") do
        jwt_sign({"sub": "test"}, "too-short")
      end
    end

    test("refuses a secret that is not a string") do
      assert_raises("jwt_sign() expects string secret, got int") do
        jwt_sign({"sub": "test"}, 123)
      end
    end

    test("refuses the none algorithm") do
      assert_raises("Unsupported algorithm: none") do
        jwt_sign({"sub": "a"}, SECRET, {"algorithm": "none"})
      end
    end
  end

  context("jwt_verify") do
    test("returns the claims of a valid token") do
      token = jwt_sign({"sub": "alice", "role": "admin", "tags": ["x"]}, SECRET, {"expires_in": 3600, "jti": "id-1"})
      claims = jwt_verify(token, SECRET)
      assert_eq(claims["sub"], "alice")
      assert_eq(claims["role"], "admin")
      assert_eq(claims["tags"], ["x"])
      assert_eq(claims["jti"], "id-1")
      assert_eq(claims["exp"] - claims["iat"], 3600)
    end

    test("checks an explicit algorithm") do
      token = jwt_sign({"sub": "bob"}, SECRET, {"expires_in": 3600, "algorithm": "HS512"})
      assert_eq(jwt_verify(token, SECRET, {"algorithm": "HS512"})["sub"], "bob")
    end

    test("raises when the token's algorithm is not the expected one") do
      token = jwt_sign({"sub": "alice"}, SECRET, {"expires_in": 3600, "algorithm": "HS256"})
      assert_raises("token algorithm HS256 does not match expected (HS512)") do
        jwt_verify(token, SECRET, {"algorithm": "HS512"})
      end
    end

    test("reports a tampered signature") do
      token = jwt_sign({"sub": "alice"}, SECRET, {"expires_in": 3600})
      assert_eq(jwt_verify(token + "x", SECRET), {"error": true, "message": "InvalidSignature"})
    end

    test("reports a token signed with another secret") do
      token = jwt_sign({"sub": "alice"}, SECRET, {"expires_in": 3600})
      assert_eq(jwt_verify(token, OTHER_SECRET), {"error": true, "message": "InvalidSignature"})
    end

    test("requires an exp claim") do
      token = jwt_sign({"sub": "a"}, SECRET)
      assert_eq(jwt_verify(token, SECRET), {"error": true, "message": "Missing required claim: exp"})
    end

    test("reports an expired token") do
      token = jwt_sign({"sub": "a"}, SECRET, {"expires_in": -120})
      assert_eq(jwt_verify(token, SECRET), {"error": true, "message": "ExpiredSignature"})
    end

    test("tolerates a recently expired token within the default leeway, but not with leeway 0") do
      token = jwt_sign({"sub": "a"}, SECRET, {"expires_in": -30})
      assert_eq(jwt_verify(token, SECRET)["sub"], "a")
      assert_eq(jwt_verify(token, SECRET, {"leeway": 0}), {"error": true, "message": "ExpiredSignature"})
    end

    test("reports a token that is not valid yet") do
      token = jwt_sign({"sub": "a"}, SECRET, {"expires_in": 60, "nbf": 4102444800})
      assert_eq(jwt_verify(token, SECRET), {"error": true, "message": "ImmatureSignature"})
    end

    test("checks audience, issuer and subject") do
      token = jwt_sign({"sub": "a"}, SECRET, {"expires_in": 60, "aud": "api", "iss": "me"})
      claims = jwt_verify(token, SECRET, {"audience": "api", "issuer": "me"})
      assert_eq(claims["aud"], "api")
      assert_eq(claims["iss"], "me")
      assert_eq(jwt_verify(token, SECRET, {"audience": "other"})["message"], "InvalidAudience")
      assert_eq(jwt_verify(token, SECRET, {"issuer": "them"})["message"], "InvalidIssuer")
      assert_eq(jwt_verify(token, SECRET, {"subject": "b"})["message"], "InvalidSubject")
    end

    test("raises on a string that is not a JWT") do
      assert_raises("Failed to parse JWT header: InvalidToken") do
        jwt_verify("not-a-token", SECRET)
      end
    end

    test("refuses a secret shorter than 32 bytes") do
      assert_raises("jwt_verify() secret must be at least 32 bytes for security (got 9)") do
        jwt_verify("any.token.value", "too-short")
      end
    end

    test("refuses a token that is not a string") do
      assert_raises("jwt_verify() expects string token, got int") do
        jwt_verify(123, SECRET)
      end
    end

    test("refuses a fourth argument") do
      assert_raises("jwt_verify() expects 2 or 3 arguments (token, secret, options?), got 4") do
        jwt_verify("t", "s", {}, "extra")
      end
    end
  end

  context("decoding without verification") do
    test("jwt_decode_unsafe wraps the claims and flags them unverified") do
      result = jwt_decode_unsafe(jwt_sign({"sub": "alice"}, SECRET, {"expires_in": 3600}))
      assert_eq(result["unverified"], true)
      assert_eq(result["claims"]["sub"], "alice")
    end

    test("jwt_decode_unsafe reports a malformed token") do
      assert_eq(jwt_decode_unsafe("not-a-token"), {"error": true, "message": "InvalidToken"})
    end

    test("jwt_decode is removed (SEC-029)") do
      assert_raises("jwt_decode() has been removed (SEC-029)") do
        jwt_decode("any.token.value")
      end
    end
  end
end
