# Base64: the standard alphabet (encode/decode, padded) and the URL-safe one
# (urlsafe_encode/urlsafe_decode, unpadded).

describe("Base64.encode") do
  test("encodes a string") do
    assert_eq(Base64.encode("hello"), "aGVsbG8=")
    assert_eq(Base64.encode("Hello World"), "SGVsbG8gV29ybGQ=")
    assert_eq(Base64.encode("12345"), "MTIzNDU=")
  end

  test("encodes control characters") do
    assert_eq(Base64.encode("Hello\nWorld\t!"), "SGVsbG8KV29ybGQJIQ==")
  end

  test("encodes the UTF-8 bytes of a string") do
    assert_eq(Base64.encode("ÿ"), "w78=")
  end

  test("encodes an array of bytes") do
    assert_eq(Base64.encode([255, 254, 253]), "//79")
  end

  test("an empty string encodes to an empty string") do
    assert_eq(Base64.encode(""), "")
  end

  test("raises on a number") do
    assert_raises("Base64.encode() expects string or array, got int") do
      Base64.encode([5][0])
    end
  end
end

describe("Base64.decode") do
  test("decodes to a string") do
    assert_eq(Base64.decode("aGVsbG8="), "hello")
    assert_eq(Base64.decode("SGVsbG8gV29ybGQ="), "Hello World")
  end

  test("an empty string decodes to an empty string") do
    assert_eq(Base64.decode(""), "")
  end

  test("returns bytes when they are not UTF-8") do
    assert_eq(Base64.decode("/w=="), [255])
  end

  test("raises on a character outside the alphabet") do
    assert_raises("Base64 decode error: Invalid symbol 33") do
      Base64.decode("!!!")
    end
  end

  test("requires the padding") do
    assert_raises("Invalid padding") do
      Base64.decode("aGVsbG8")
    end
  end

  test("rejects the URL-safe alphabet") do
    assert_raises("Invalid symbol 45") do
      Base64.decode("-_8")
    end
  end
end

describe("Base64 round trips") do
  test("punctuation and digits") do
    original = "Test string with special chars: !@#$%"
    assert_eq(Base64.decode(Base64.encode(original)), original)
  end

  test("characters that are special in URLs") do
    assert_eq(Base64.decode(Base64.encode("foo/bar?query=value")), "foo/bar?query=value")
  end

  test("non-ASCII text") do
    assert_eq(Base64.decode(Base64.encode("héllo")), "héllo")
  end
end

describe("Base64 URL-safe") do
  test("urlsafe_encode uses - and _ and no padding") do
    assert_eq(Base64.urlsafe_encode([255, 254, 253]), "__79")
    assert_eq(Base64.urlsafe_encode("Hello, World!"), "SGVsbG8sIFdvcmxkIQ")
  end

  test("urlsafe_decode accepts padding or none") do
    assert_eq(Base64.urlsafe_decode("SGVsbG8sIFdvcmxkIQ"), "Hello, World!")
    assert_eq(Base64.urlsafe_decode("SGVsbG8sIFdvcmxkIQ=="), "Hello, World!")
  end

  test("urlsafe_decode returns bytes when they are not UTF-8") do
    assert_eq(Base64.urlsafe_decode("__79"), [255, 254, 253])
  end
end
