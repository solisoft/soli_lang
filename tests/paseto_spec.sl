# PASETO v4 tokens through the interpreter's class dispatch. The Rust unit tests
# in src/interpreter/builtins/paseto.rs call the native functions directly; this
# spec goes through `Paseto.*` the way an app does, so a registration or
# dispatch regression fails here even when the native side is fine.

const RAW_HEX_KEY = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"

# Fresh keys for each test
local_key = nil
key_pair = nil

def seconds_between(earlier, later)
  DateTime.parse(later).to_unix - DateTime.parse(earlier).to_unix
end

describe("Paseto") do
  before_each() do
    local_key = Paseto.generate_local_key()
    key_pair = Paseto.generate_key_pair()
  end

  context("keys") do
    test("generate_local_key returns a k4.local PASERK of 32 bytes") do
      assert_match(local_key, "^k4\\.local\\.[A-Za-z0-9_-]{43}$")
      assert_ne(Paseto.generate_local_key(), local_key)
    end

    test("generate_key_pair returns both halves, purpose-tagged") do
      assert_match(key_pair["secret"], "^k4\\.secret\\.[A-Za-z0-9_-]{86}$")
      assert_match(key_pair["public"], "^k4\\.public\\.[A-Za-z0-9_-]{43}$")
    end

    test("public_key derives the verifying half from the secret") do
      assert_eq(Paseto.public_key(key_pair["secret"]), key_pair["public"])
    end

    test("key_id is a PASERK id, distinct per purpose and stable per key") do
      assert(Paseto.key_id(key_pair["public"]).starts_with("k4.pid."))
      assert(Paseto.key_id(local_key).starts_with("k4.lid."))
      assert_eq(Paseto.key_id(local_key), Paseto.key_id(local_key))
    end

    test("key_id refuses a raw hex key, which carries no purpose") do
      assert_raises("Paseto.key_id(): expects a PASERK key string") do
        Paseto.key_id(RAW_HEX_KEY)
      end
    end
  end

  context("local tokens") do
    test("round-trip custom and registered claims") do
      token = Paseto.encrypt({"user_id": 42, "role": "admin"}, local_key, {"expires_in": 900})
      assert(token.starts_with("v4.local."))
      claims = Paseto.decrypt(token, local_key)
      assert_eq(claims["user_id"], 42)
      assert_eq(claims["role"], "admin")
      # PASETO dates are RFC 3339 strings, not Unix ints
      assert_match(claims["exp"], "^\\d{4}-\\d{2}-\\d{2}T\\d{2}:\\d{2}:\\d{2}Z$")
      assert_eq(seconds_between(claims["iat"], claims["exp"]), 900)
    end

    test("a raw 64-hex key works, so `openssl rand -hex 32` does") do
      assert_eq(Paseto.decrypt(Paseto.encrypt({"sub": "alice"}, RAW_HEX_KEY), RAW_HEX_KEY)["sub"], "alice")
    end

    test("the wrong key is rejected") do
      token = Paseto.encrypt({"sub": "alice"}, local_key)
      assert_raises("Paseto.decrypt(): the token is not authentic (wrong key, or it was tampered with") do
        Paseto.decrypt(token, Paseto.generate_local_key())
      end
    end

    test("an expired token is rejected") do
      token = Paseto.encrypt({"sub": "alice"}, local_key, {"exp": 1000000000})
      assert_raises("Paseto.decrypt(): the token has expired") do
        Paseto.decrypt(token, local_key)
      end
    end

    test("the payload is unreadable without the key") do
      token = Paseto.encrypt({"secret_note": "classified"}, local_key)
      assert_not(token.contains("classified"))
      assert_eq(Paseto.decode_unsafe(token), {
        "unverified": true,
        "version": "v4",
        "purpose": "local",
        "claims": nil,
        "footer": nil
      })
    end
  end

  context("public tokens") do
    test("verify with the public half") do
      token = Paseto.sign({"sub": "bob"}, key_pair["secret"], {"expires_in": 600})
      assert(token.starts_with("v4.public."))
      assert_eq(Paseto.verify(token, key_pair["public"])["sub"], "bob")
    end

    test("do not verify under another key pair") do
      token = Paseto.sign({"sub": "bob"}, key_pair["secret"])
      assert_raises("Paseto.verify(): the token is not authentic") do
        Paseto.verify(token, Paseto.generate_key_pair()["public"])
      end
    end

    test("audience and issuer are checked when expected") do
      token = Paseto.sign({"sub": "bob"}, key_pair["secret"], {"aud": "api.example.com", "iss": "https://issuer.test"})
      expected = {"audience": "api.example.com", "issuer": "https://issuer.test"}
      claims = Paseto.verify(token, key_pair["public"], expected)
      assert_eq(claims["aud"], "api.example.com")
      assert_eq(claims["iss"], "https://issuer.test")
      assert_raises("the `aud` claim does not match the expected audience") do
        Paseto.verify(token, key_pair["public"], {"audience": "other.example.com"})
      end
      assert_raises("the `iss` claim does not match the expected issuer") do
        Paseto.verify(token, key_pair["public"], {"issuer": "https://other.test"})
      end
    end

    test("verifying with the secret key is refused") do
      token = Paseto.sign({"sub": "bob"}, key_pair["secret"])
      assert_raises("Paseto.verify(): got a secret key") do
        Paseto.verify(token, key_pair["secret"])
      end
    end

    test("a tampered token fails") do
      token = Paseto.sign({"sub": "alice"}, key_pair["secret"])
      tampered = token.substring(0, token.length - 4) + "AAAA"
      assert_raises("Paseto.verify(): the token is not authentic") do
        Paseto.verify(tampered, key_pair["public"])
      end
    end
  end

  context("purposes and options") do
    test("a local token cannot be verified as a signed one") do
      local_token = Paseto.encrypt({"sub": "a"}, local_key)
      assert_raises("Paseto.verify(): got a v4.local token — use Paseto.decrypt()") do
        Paseto.verify(local_token, key_pair["public"])
      end
    end

    test("a signed token cannot be decrypted as a local one") do
      signed_token = Paseto.sign({"sub": "a"}, key_pair["secret"])
      assert_raises("Paseto.decrypt(): got a v4.public token — use Paseto.verify()") do
        Paseto.decrypt(signed_token, local_key)
      end
    end

    test("a non-expiring token must be opted into on both ends") do
      token = Paseto.encrypt({"sub": "a"}, local_key, {"non_expiring": true})
      assert_raises("Paseto.decrypt(): the token has no `exp` claim") do
        Paseto.decrypt(token, local_key)
      end
      assert_eq(Paseto.decrypt(token, local_key, {"allow_non_expiring": true})["sub"], "a")
    end

    test("a token minted without options expires after the documented 3600s") do
      # Forgetting `expires_in` must yield a short-lived token, never an eternal one
      claims = Paseto.decrypt(Paseto.encrypt({"sub": "a"}, local_key), local_key)
      assert_eq(seconds_between(claims["iat"], claims["exp"]), 3600)
      assert_eq(claims["nbf"], claims["iat"])
    end

    test("aud is a single string — the JWT array form is refused, not dropped") do
      assert_raises("Paseto.encrypt(): `aud` expects a String, got array") do
        Paseto.encrypt({"sub": "a"}, local_key, {"aud": ["a", "b"]})
      end
    end

    test("an unknown option raises rather than being ignored") do
      assert_raises("Paseto.encrypt(): unknown option `expires`") do
        Paseto.encrypt({"sub": "a"}, local_key, {"expires": 60})
      end
      token = Paseto.encrypt({"sub": "a"}, local_key)
      assert_raises("Paseto.decrypt(): unknown option `audiance`") do
        Paseto.decrypt(token, local_key, {"audiance": "api"})
      end
    end

    test("an implicit assertion must match, and never travels in the token") do
      token = Paseto.encrypt({"sub": "a"}, local_key, {"implicit": "session-42"})
      assert_not(token.contains("session-42"))
      assert_eq(Paseto.decrypt(token, local_key, {"implicit": "session-42"})["sub"], "a")
      assert_raises("Paseto.decrypt(): the token is not authentic") do
        Paseto.decrypt(token, local_key, {"implicit": "session-99"})
      end
    end
  end

  context("key rotation") do
    test("the footer kid picks the key before verification") do
      previous = Paseto.generate_key_pair()
      # A token still in flight, signed with the key being rotated out
      token = Paseto.sign({"sub": "alice"}, previous["secret"], {"kid": Paseto.key_id(previous["public"])})
      kid = Paseto.decode_unsafe(token)["footer"]["kid"]
      verifying_key = [key_pair["public"], previous["public"]].find { |candidate| Paseto.key_id(candidate) == kid }
      assert_eq(verifying_key, previous["public"])
      assert_eq(Paseto.verify(token, verifying_key)["sub"], "alice")
    end

    test("a kid that is not a PASERK id is refused") do
      assert_raises("Paseto.sign(): `kid` must be a PASERK key id from Paseto.key_id(key)") do
        Paseto.sign({"sub": "a"}, key_pair["secret"], {"kid": "k1"})
      end
    end

    test("decode_unsafe keeps unverified claims out of the top level") do
      peek = Paseto.decode_unsafe(Paseto.sign({"sub": "alice"}, key_pair["secret"]))
      assert_eq(peek["unverified"], true)
      assert_eq(peek["purpose"], "public")
      assert_eq(peek["claims"]["sub"], "alice")
      # Reaching for the claim directly must not yield a trusted-looking value
      assert_null(peek["sub"])
    end
  end
end
