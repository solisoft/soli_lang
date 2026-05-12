// ============================================================================
// VAPID / Web Push Test Suite
// ============================================================================
//
// Covers the four native builtins that replace the `web-push` Node module:
// vapid_generate_keys, vapid_sign, vapid_encrypt, vapid_send. End-to-end
// delivery against a real push service is not exercised here — see the
// TODO at the bottom for what the integration test would need.

describe("VAPID builtins", fn() {
    test("vapid_generate_keys() returns a hash with two non-empty base64url strings", fn() {
        let keys = vapid_generate_keys();
        assert_eq(type(keys), "hash");
        assert(has_key(keys, "public_key"));
        assert(has_key(keys, "private_key"));
        let pub_key = keys["public_key"];
        let priv_key = keys["private_key"];
        assert_eq(type(pub_key), "string");
        assert_eq(type(priv_key), "string");
        assert(len(pub_key) > 0);
        assert(len(priv_key) > 0);
        // Fresh keys: two consecutive calls must not collide.
        let other = vapid_generate_keys();
        assert_not(other["public_key"] == pub_key);
        assert_not(other["private_key"] == priv_key);
    });

    test("vapid_sign(priv, aud, sub) produces a three-segment JWT", fn() {
        let keys = vapid_generate_keys();
        let token = vapid_sign(
            keys["private_key"],
            "https://fcm.googleapis.com",
            "mailto:dev@example.com"
        );
        assert_eq(type(token), "string");
        let segments = token.split(".");
        assert_eq(len(segments), 3);
        // No empty segments — header, claims, signature all present.
        for seg in segments {
            assert(len(seg) > 0);
        }
    });

    test("vapid_sign rejects a non-URL audience", fn() {
        let keys = vapid_generate_keys();
        let threw = false;
        try {
            vapid_sign(
                keys["private_key"],
                "fcm.googleapis.com",
                "mailto:dev@example.com"
            );
        } catch (e) {
            threw = true;
        }
        assert(threw);
    });

    test("vapid_encrypt returns ciphertext + salt + server_public_key", fn() {
        // Build a fake subscriber by generating a VAPID-style keypair as
        // the user agent's p256dh, plus a 16-byte auth secret. The exact
        // bytes don't matter — we just need a structurally valid
        // subscription for the encryption path.
        let ua_keys = vapid_generate_keys();
        let auth_secret = "qZQVc1lCQGsKkV0HZNI3RA";  // 16 bytes, base64url
        let subscription = {
            "endpoint": "https://example.invalid/push/abc",
            "keys": {
                "p256dh": ua_keys["public_key"],
                "auth": auth_secret
            }
        };
        let server_keys = vapid_generate_keys();
        let result = vapid_encrypt(
            "{\"title\":\"Hello\"}",
            subscription,
            server_keys["public_key"],
            server_keys["private_key"]
        );
        assert_eq(type(result), "hash");
        assert(has_key(result, "ciphertext"));
        assert(has_key(result, "salt"));
        assert(has_key(result, "server_public_key"));
        assert(len(result["ciphertext"]) > 0);
        assert(len(result["salt"]) > 0);
        assert(len(result["server_public_key"]) > 0);
    });

    // TODO: integration test for vapid_send — would need either a real
    // push endpoint (FCM/Mozilla) with rotating subscriptions, or a
    // mock_http server tuned to accept POST with the aes128gcm body and
    // the `Authorization: vapid t=..., k=...` header. Skipped because
    // a meaningful assertion has to talk to a real service.
});
