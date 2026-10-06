# url_encode / url_decode: strict RFC 3986 component encoding and form-style decoding.

describe("url_encode") do
  test("percent-encodes spaces") do
    assert_eq(url_encode("hello world"), "hello%20world")
  end

  test("percent-encodes reserved characters (component encoding)") do
    # `/`, `?`, `&`, `=`, `#` are all encoded so the result is safe in any URL component
    assert_eq(url_encode("a/b?c=d&e#f"), "a%2Fb%3Fc%3Dd%26e%23f")
  end

  test("leaves unreserved characters alone") do
    assert_eq(url_encode("ABCabc123-_.~"), "ABCabc123-_.~")
  end

  test("encodes the empty string to the empty string") do
    assert_eq(url_encode(""), "")
  end

  test("encodes UTF-8 multibyte characters byte by byte") do
    # "é" is c3 a9 in UTF-8
    assert_eq(url_encode("café"), "caf%C3%A9")
  end

  test("accepts non-string scalars and treats nil as empty") do
    assert_eq(url_encode(42), "42")
    assert_eq(url_encode(1.5), "1.5")
    assert_eq(url_encode(true), "true")
    assert_eq(url_encode(nil), "")
  end
end

describe("url_decode") do
  test("decodes percent-encoded bytes") do
    assert_eq(url_decode("hello%20world"), "hello world")
    assert_eq(url_decode("%41%42"), "AB")
  end

  test("decodes plus to space (form style)") do
    assert_eq(url_decode("hello+world"), "hello world")
  end

  test("decodes a fully encoded URL component") do
    assert_eq(url_decode("a%2Fb%3Fc%3Dd%26e%23f"), "a/b?c=d&e#f")
  end

  test("decodes UTF-8 multibyte characters") do
    assert_eq(url_decode("caf%C3%A9"), "café")
  end

  test("decodes the empty string and nil to the empty string") do
    assert_eq(url_decode(""), "")
    assert_eq(url_decode(nil), "")
  end

  test("round-trips through url_encode") do
    original = "Test: a/b?c=d&e#f g+h"
    assert_eq(url_decode(url_encode(original)), original)
  end

  test("passes invalid percent-escapes through literally") do
    # `%ZZ` is not a hex byte; it is kept rather than raising
    assert_eq(url_decode("bad%ZZinput"), "bad%ZZinput")
  end
end
