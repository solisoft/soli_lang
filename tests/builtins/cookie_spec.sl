# Cookie builtins without a server: set_cookie / read_cookie on the response jar,
# option checking, and the signed / encrypted jar. Cookie header parsing and
# Set-Cookie emission are covered end to end in tests/server_e2e_test.rs.

describe("plain cookies") do
  test("set_cookie returns nil and read_cookie reads the value back") do
    assert_null(set_cookie("theme", "dark"))
    assert_eq(read_cookie("theme"), "dark")
  end

  test("read_cookie returns nil for an absent cookie, in any mode") do
    assert_null(read_cookie("no_such_cookie"))
    assert_null(read_cookie("no_such_cookie", {"encrypted": true}))
    assert_null(read_cookie("no_such_cookie", {"signed": true}))
  end

  test("read_cookie refuses an unknown option") do
    assert_raises("read_cookie() unknown option \"encrytped\" (encrypted, signed)") do
      read_cookie("x", {"encrytped": true})
    end
  end

  test("set_cookie refuses an unknown option") do
    assert_raises("set_cookie() unknown option \"bogus\"") do
      set_cookie("x", "y", {"bogus": 1})
    end
  end
end

describe("signed and encrypted cookie jar") do
  before_all() do
    session_configure({"secret": "spec-secret-0123456789abcdef-0123"})
  end

  test("an encrypted cookie round-trips structured values") do
    set_cookie("prefs", {"theme": "dark", "cols": [1, 2]}, {"encrypted": true})
    assert_eq(read_cookie("prefs", {"encrypted": true}), {"theme": "dark", "cols": [1, 2]})
  end

  test("a plain read of an encrypted cookie sees only the opaque wire value") do
    set_cookie("sealed", "hello", {"encrypted": true})
    raw = read_cookie("sealed")
    assert(raw.starts_with("enc.v1."))
    assert_not(raw.includes?("hello"))
  end

  test("a signed cookie round-trips and its wire value is sig.v1") do
    set_cookie("uid", 42, {"signed": true})
    assert_eq(read_cookie("uid", {"signed": true}), 42)
    assert(read_cookie("uid").starts_with("sig.v1."))
  end

  test("reading in the wrong mode gives nil, never a decoded value") do
    set_cookie("uid_mode", 42, {"signed": true})
    assert_null(read_cookie("uid_mode", {"encrypted": true}))
  end

  test("a tampered signed cookie reads as nil") do
    set_cookie("uid_tamper", 42, {"signed": true})
    set_cookie("uid_tamper", read_cookie("uid_tamper") + "x")
    assert_null(read_cookie("uid_tamper", {"signed": true}))
  end

  test("a sealed value moved to another cookie name reads as nil") do
    set_cookie("uid_source", 42, {"signed": true})
    set_cookie("uid_target", read_cookie("uid_source"))
    assert_null(read_cookie("uid_target", {"signed": true}))
  end

  test("encrypted and signed are mutually exclusive") do
    assert_raises("mutually exclusive") do
      set_cookie("bad", 1, {"encrypted": true, "signed": true})
    end
  end
end
