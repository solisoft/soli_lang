// ============================================================================
// URL encoding / decoding test suite
// ============================================================================

describe("url_encode", fn() {
    test("percent-encodes spaces", fn() {
        assert_eq(url_encode("hello world"), "hello%20world");
    });

    test("percent-encodes reserved chars (component encoding)", fn() {
        // Strict RFC 3986 component encoding — `/`, `?`, `&`, `=`, `#` all
        // get encoded so the result is safe to splice into any URL component.
        assert_eq(url_encode("a/b?c=d&e#f"), "a%2Fb%3Fc%3Dd%26e%23f");
    });

    test("leaves unreserved chars alone", fn() {
        assert_eq(
            url_encode("ABCabc123-_.~"),
            "ABCabc123-_.~"
        );
    });

    test("encodes empty string to empty string", fn() {
        assert_eq(url_encode(""), "");
    });

    test("encodes UTF-8 multibyte chars", fn() {
        // "café" → "café" UTF-8 is 63 61 66 c3 a9
        assert_eq(url_encode("café"), "caf%C3%A9");
    });

    test("accepts non-string scalars", fn() {
        assert_eq(url_encode(42), "42");
        assert_eq(url_encode(true), "true");
        assert_eq(url_encode(null), "");
    });
});

describe("url_decode", fn() {
    test("decodes percent-encoded bytes", fn() {
        assert_eq(url_decode("hello%20world"), "hello world");
    });

    test("decodes plus to space (form style)", fn() {
        assert_eq(url_decode("hello+world"), "hello world");
    });

    test("decodes a full encoded URL component", fn() {
        assert_eq(url_decode("a%2Fb%3Fc%3Dd%26e%23f"), "a/b?c=d&e#f");
    });

    test("decodes UTF-8 multibyte chars", fn() {
        assert_eq(url_decode("caf%C3%A9"), "café");
    });

    test("decodes empty string to empty string", fn() {
        assert_eq(url_decode(""), "");
    });

    test("treats null as empty string", fn() {
        assert_eq(url_decode(null), "");
    });

    test("roundtrips through encode", fn() {
        let original = "Test: a/b?c=d&e#f g+h";
        assert_eq(url_decode(url_encode(original)), original);
    });

    test("passes through invalid percent-escapes literally", fn() {
        // `%ZZ` is not a valid hex byte. The crate keeps it as-is rather
        // than raising — convenient when decoding pre-existing data.
        assert_eq(url_decode("bad%ZZinput"), "bad%ZZinput");
    });
});
