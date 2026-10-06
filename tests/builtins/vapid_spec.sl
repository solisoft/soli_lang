# VAPID / Web Push builtins that replace the `web-push` Node module:
# vapid_generate_keys, vapid_sign and vapid_encrypt. vapid_send needs a real
# push service, so it is not exercised here.

const AUTH_SECRET = "qZQVc1lCQGsKkV0HZNI3RA"   # 16 bytes, base64url

# Fresh keys for the subscriber (user agent) and the application server
subscriber_keys = nil
server_keys = nil
subscription = nil

describe("VAPID builtins") do
  before_each() do
    subscriber_keys = vapid_generate_keys()
    server_keys = vapid_generate_keys()
    subscription = {
      "endpoint": "https://example.invalid/push/abc",
      "keys": {"p256dh": subscriber_keys["public_key"], "auth": AUTH_SECRET}
    }
  end

  context("vapid_generate_keys") do
    test("returns an uncompressed P-256 public key and a 32-byte private key, base64url") do
      assert_eq(subscriber_keys.keys, ["public_key", "private_key"])
      # 65 bytes -> 87 chars, 32 bytes -> 43 chars, no padding
      assert_match(subscriber_keys["public_key"], "^[A-Za-z0-9_-]{87}$")
      assert_match(subscriber_keys["private_key"], "^[A-Za-z0-9_-]{43}$")
      # 0x04 (uncompressed point) encodes as a leading "B"
      assert(subscriber_keys["public_key"].starts_with("B"))
    end

    test("never repeats a key") do
      assert_ne(server_keys["public_key"], subscriber_keys["public_key"])
      assert_ne(server_keys["private_key"], subscriber_keys["private_key"])
    end
  end

  context("vapid_sign") do
    test("produces an ES256 JWT carrying aud and sub") do
      token = vapid_sign(server_keys["private_key"], "https://fcm.googleapis.com", "mailto:dev@example.com")
      segments = token.split(".")
      assert_eq(segments.length, 3)
      assert_eq(segments[0], "eyJ0eXAiOiJKV1QiLCJhbGciOiJFUzI1NiJ9")   # {"typ":"JWT","alg":"ES256"}
      claims = jwt_decode_unsafe(token)["claims"]
      assert_eq(claims["aud"], "https://fcm.googleapis.com")
      assert_eq(claims["sub"], "mailto:dev@example.com")
    end

    test("expires twelve hours from now") do
      claims = jwt_decode_unsafe(vapid_sign(server_keys["private_key"], "https://a.example", "mailto:x@y.z"))["claims"]
      seconds_left = claims["exp"] - DateTime.now.to_unix
      assert_gt(seconds_left, 12 * 3600 - 60)
      assert_lt(seconds_left, 12 * 3600 + 1)
    end

    test("rejects an audience that is not an http(s) origin") do
      assert_raises("vapid_sign(): audience must be an http(s) origin, got 'fcm.googleapis.com'") do
        vapid_sign(server_keys["private_key"], "fcm.googleapis.com", "mailto:dev@example.com")
      end
    end

    test("rejects a private key that is not base64url") do
      assert_raises("vapid_sign(): private_key is not valid base64url") do
        vapid_sign("short", "https://a.example", "mailto:dev@example.com")
      end
    end
  end

  context("vapid_encrypt") do
    test("returns an aes128gcm body, its salt and the ephemeral server key") do
      payload = "{\"title\":\"Hello\"}"
      result = vapid_encrypt(payload, subscription, server_keys["public_key"], server_keys["private_key"])
      assert_eq(result.keys, ["ciphertext", "salt", "server_public_key"])
      assert_match(result["salt"], "^[A-Za-z0-9_-]{22}$")              # 16 bytes
      assert_match(result["server_public_key"], "^[A-Za-z0-9_-]{87}$") # 65 bytes
      # header (16 salt + 4 rs + 1 idlen + 65 key) + 17 plaintext + 1 delimiter + 16 tag = 120 bytes
      assert_eq(result["ciphertext"].length, 160)
      # the body opens with the salt
      assert_eq(result["ciphertext"].substring(0, 21), result["salt"].substring(0, 21))
    end

    test("an empty payload still carries the header, delimiter and tag") do
      result = vapid_encrypt("", subscription, server_keys["public_key"], server_keys["private_key"])
      assert_eq(result["ciphertext"].length, 138)   # 103 bytes
    end

    test("uses a fresh salt and ephemeral key on every call") do
      first = vapid_encrypt("x", subscription, server_keys["public_key"], server_keys["private_key"])
      second = vapid_encrypt("x", subscription, server_keys["public_key"], server_keys["private_key"])
      assert_ne(first["salt"], second["salt"])
      assert_ne(first["server_public_key"], second["server_public_key"])
      assert_ne(first["server_public_key"], server_keys["public_key"])
    end

    test("rejects a subscription without keys") do
      assert_raises("vapid_encrypt(): subscription is missing 'keys' hash with p256dh/auth") do
        vapid_encrypt("x", {"endpoint": "https://x"}, server_keys["public_key"], server_keys["private_key"])
      end
    end
  end
end
