# Push notifications without any configured target: Push.deliver reaches
# nobody, and the FCM and VAPID builtins refuse bad input before any network
# call. Signing and encryption: vapid_spec.sl.

describe("Push.deliver with no targets") do
  test("reaches nobody and reports empty results") do
    assert_eq(Push.deliver("spec:push-channel", {"title": "Hello"}), {
      "reached_live": 0,
      "transport": "none",
      "sent": [],
      "failed": [],
      "prune": []
    })
  end

  test("needs a channel and a payload") do
    assert_raises("Push.deliver() expects (channel, payload, options?)") do
      Push.deliver("only-channel")
    end
  end

  test("refuses options that are not a hash") do
    assert_raises("Push.deliver(): options must be a hash, got string") do
      Push.deliver("spec:push-channel", {"title": "x"}, "bogus")
    end
  end
end

describe("FCM input validation") do
  test("send refuses an implausible device token") do
    assert_raises("Fcm.send(): implausible device token") do
      Fcm.send("", {}, {})
    end
  end

  test("access_token refuses a service account that is not JSON") do
    assert_raises("service_account is not a valid service-account JSON") do
      Fcm.access_token("{ definitely not json")
    end
  end
end

describe("VAPID input validation") do
  test("vapid_generate_keys returns a P-256 keypair in unpadded base64url") do
    keys = vapid_generate_keys()
    assert_eq(keys["public_key"].length, 87)
    assert_eq(keys["private_key"].length, 43)
    assert_match(keys["public_key"], "^[A-Za-z0-9_-]+$")
    assert_match(keys["private_key"], "^[A-Za-z0-9_-]+$")
  end

  test("vapid_send needs a subject") do
    keys = vapid_generate_keys()
    assert_raises("vapid_send() expects 5 or 6 arguments") do
      vapid_send({"endpoint": "https://127.0.0.1:9/x"}, "payload", keys["private_key"], keys["public_key"])
    end
  end

  test("vapid_send refuses a subscription without keys") do
    keys = vapid_generate_keys()
    assert_raises("subscription is missing 'keys' hash with p256dh/auth") do
      vapid_send(
        {"endpoint": "https://127.0.0.1:9/x"},
        "payload",
        keys["private_key"],
        keys["public_key"],
        "mailto:test@example.com"
      )
    end
  end
end
