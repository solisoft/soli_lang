# The compositions an OpenID Connect provider is built from — PKCE challenges,
# JWK thumbprints, id_token claims — rather than the individual builtins. Each
# path crosses several builtins, so a break in the seam between them shows here.

const SECRET = "0123456789abcdef0123456789abcdef"

# RFC 7638 §3.1 example RSA key modulus
const RFC7638_MODULUS = "0vx7agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM78LhWx4cbbfAAtVT86zwu1RK7aPFFxuhDR1L6tSoc_" +
  "BJECPebWKRXjBZCiFV4n3oknjhMstn64tZ_2W-5JsGY4Hc5n9yBXArwl93lqt7_RN5w6Cf0h4QyQ5v-65YGjQR0_FDW2QvzqY368QQMicAt" +
  "aSqzs8KJZgnYb9c7d0zgdAZHzu6qMQvRL5hajrn1n91CbOpbISD08qNLyrdkt-bFTWhAI4vMQFh6WeZu0fM4lFd2NcRwr3XPksINHaQ-G_x" +
  "BniIqbw0Ls1jF44-csFCur-kEgU8awapJzKnqDKgw"

# Unpadded base64url of the raw bytes behind a hex digest. `Crypto.sha256`
# returns hex, so it must be decoded first — base64url of the hex text is a
# different (and wrong) value.
def base64url_digest(hex_digest)
  Base64.urlsafe_encode(Hex.decode(hex_digest))
end

def thumbprint(jwk)
  base64url_digest(Crypto.sha256(Crypto.canonical_json(jwk)))
end

describe("PKCE (RFC 7636)") do
  test("the S256 challenge matches the Appendix B vector") do
    challenge = base64url_digest(Crypto.sha256("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"))
    assert_eq(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM")
  end

  test("a challenge is 43 url-safe chars with no padding") do
    assert_match(base64url_digest(Crypto.sha256(Crypto.random_token())), "^[A-Za-z0-9_-]{43}$")
  end

  test("a wrong verifier does not reproduce the challenge") do
    challenge = base64url_digest(Crypto.sha256("the-real-verifier"))
    forged = base64url_digest(Crypto.sha256("the-wrong-verifier"))
    assert_not(Crypto.secure_compare(challenge, forged))
  end
end

describe("JWK thumbprint (RFC 7638)") do
  test("canonical_json emits the members sorted, with no whitespace") do
    members = {"n": "0vx7ag", "kty": "RSA", "e": "AQAB"}
    assert_eq(Crypto.canonical_json(members), "{\"e\":\"AQAB\",\"kty\":\"RSA\",\"n\":\"0vx7ag\"}")
  end

  test("the thumbprint of the §3.1 example key matches the RFC") do
    jwk = {"kty": "RSA", "n": RFC7638_MODULUS, "e": "AQAB"}
    assert_eq(thumbprint(jwk), "NzbLsXh8uDCcd-6MNwXF4W_7noWXFZAfHkxZsRGC9Xs")
  end

  test("the thumbprint does not depend on insertion order") do
    one = {"e": "AQAB", "kty": "RSA", "n": "0vx7ag"}
    two = {"n": "0vx7ag", "e": "AQAB", "kty": "RSA"}
    assert_eq(thumbprint(one), thumbprint(two))
  end
end

describe("secure token generation") do
  test("random_token defaults to 256 bits of unpadded base64url") do
    assert_match(Crypto.random_token(), "^[A-Za-z0-9_-]{43}$")
  end

  test("random_hex(n) counts bytes, not characters") do
    assert_match(Crypto.random_hex(32), "^[0-9a-f]{64}$")
  end

  test("tokens do not repeat") do
    assert_ne(Crypto.random_token(), Crypto.random_token())
    assert_ne(Crypto.random_hex(16), Crypto.random_hex(16))
  end

  test("random_bytes(n) returns n bytes in range") do
    bytes = Crypto.random_bytes(16)
    assert_eq(bytes.length, 16)
    assert(bytes.all? { |byte| byte >= 0 && byte <= 255 })
  end
end

describe("id_token composition") do
  test("registered claims and the kid header survive a round trip") do
    token = jwt_sign({"sub": "user-1"}, SECRET, {
      "expires_in": 600,
      "kid": "key-1",
      "aud": "client-1",
      "iss": "https://op.example",
      "jti": "jti-1"
    })
    header = JSON.parse(Base64.urlsafe_decode(token.split(".")[0]))
    assert_eq(header, {"typ": "JWT", "alg": "HS256", "kid": "key-1"})

    claims = jwt_verify(token, SECRET, {"audience": "client-1", "issuer": "https://op.example"})
    assert_null(claims["error"])
    assert_eq(claims["sub"], "user-1")
    assert_eq(claims["aud"], "client-1")
    assert_eq(claims["iss"], "https://op.example")
    assert_eq(claims["jti"], "jti-1")
  end

  test("a token minted for another client is rejected") do
    token = jwt_sign({"sub": "user-1"}, SECRET, {"expires_in": 600, "aud": "client-1"})
    assert_eq(jwt_verify(token, SECRET, {"audience": "client-2"}), {"error": true, "message": "InvalidAudience"})
  end

  test("a token from another issuer is rejected") do
    token = jwt_sign({"sub": "user-1"}, SECRET, {"expires_in": 600, "iss": "https://evil.example"})
    assert_eq(jwt_verify(token, SECRET, {"issuer": "https://op.example"}), {"error": true, "message": "InvalidIssuer"})
  end

  test("at_hash is the left-most 128 bits of the access token digest") do
    digest = Hex.decode(Crypto.sha256("jHkWEdUXMU1BwAsC4vtUsZwnNjw"))
    assert_eq(digest.length, 32)
    assert_eq(Base64.urlsafe_encode(digest.take(16)), "hA6a5yhKa55OKnh_Kpp2VA")
  end
end
