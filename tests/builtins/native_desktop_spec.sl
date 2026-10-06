# The desktop-shell builtins outside a built app: Native's live channel has no
# subscribers, and Updater has no auto-update channel to talk to.

describe("Native channel") do
  test("notify on a channel nobody listens to reaches 0 clients") do
    assert_eq(Native.notify("spec:nobody", {"title": "Hi"}), 0)
  end

  test("a channel nobody listens to has 0 subscribers") do
    assert_eq(Native.subscribers("spec:nobody"), 0)
  end

  test("notify refuses a channel name containing a dot") do
    assert_raises("native channel name may not contain '.'") do
      Native.notify("bad.channel", {"title": "Hi"})
    end
  end

  test("subscribers refuses an empty channel name") do
    assert_raises("native channel name cannot be empty") do
      Native.subscribers("")
    end
  end

  describe("channel_token") do
    before_all() do
      session_configure({"secret": "spec-secret-0123456789abcdef-0123"})
    end

    test("is a three-part token whose first part encodes the channel") do
      parts = Native.channel_token("spec:a").split(".")
      assert_eq(parts.length, 3)
      assert_eq(Base64.urlsafe_decode(parts[0]), "spec:a")
    end

    test("expires twelve hours after it is minted") do
      minted_at = DateTime.now.to_unix
      expires_at = int(Base64.urlsafe_decode(Native.channel_token("spec:a").split(".")[1]))
      lifetime = expires_at - minted_at
      assert(lifetime >= 43199 && lifetime <= 43200, "lifetime was #{lifetime}")
    end

    test("differs between channels") do
      assert_ne(Native.channel_token("spec:a"), Native.channel_token("spec:b"))
    end
  end
end

describe("Updater outside a built artifact") do
  test("version is nil") do
    assert_null(Updater.version)
  end

  test("check reports no configured update channel") do
    assert_eq(Updater.check, {
      "available": false,
      "configured": false,
      "error": "this build has no auto-update channel (--update-url)"
    })
  end

  test("apply reports not-configured") do
    assert_eq(Updater.apply, {
      "status": "not-configured",
      "error": "this build has no auto-update channel (--update-url)"
    })
  end
end
