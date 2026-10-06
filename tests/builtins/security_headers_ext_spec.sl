# Response security headers as configuration: every setter records header state
# that get_security_headers() returns. Also the enable/disable switches, the
# force-secure-cookies flag, and the h() / j() escaping helpers.

describe("security headers") do
  before_each() do
    reset_security_headers()
  end

  after_each() do
    reset_security_headers()
  end

  test("reset leaves no configured header") do
    prevent_clickjacking()
    reset_security_headers()
    assert_eq(get_security_headers(), {})
  end

  context("CSP setters") do
    test("set_csp records a full policy") do
      set_csp("default-src 'none'")
      assert_eq(get_security_headers(), {"Content-Security-Policy": "default-src 'none'"})
    end

    test("set_csp takes a report-only flag, as documented") do
      pending("bug: set_csp is declared with arity 1, so the documented report_only argument raises")
      set_csp("default-src 'none'", true)
      assert_eq(get_security_headers()["Content-Security-Policy-Report-Only"], "default-src 'none'")
    end

    test("set_csp_default_src builds the directive") do
      set_csp_default_src("'self'")
      assert_eq(get_security_headers()["Content-Security-Policy"], "default-src 'self'")
    end

    test("set_csp_default_src joins several sources, as documented") do
      pending("bug: set_csp_default_src is declared with arity 1, the documented ...sources form raises")
      set_csp_default_src("'self'", "https://cdn.example")
      assert_eq(get_security_headers()["Content-Security-Policy"], "default-src 'self' https://cdn.example")
    end

    test("set_csp_script_src builds the directive") do
      set_csp_script_src("'self'")
      assert_eq(get_security_headers()["Content-Security-Policy"], "script-src 'self'")
    end

    test("set_csp_style_src builds the directive") do
      set_csp_style_src("'unsafe-inline'")
      assert_eq(get_security_headers()["Content-Security-Policy"], "style-src 'unsafe-inline'")
    end

    test("each directive setter replaces the whole policy rather than adding to it") do
      set_csp_default_src("'self'")
      set_csp_script_src("'self'")
      assert_eq(get_security_headers()["Content-Security-Policy"], "script-src 'self'")
    end
  end

  context("HSTS") do
    test("a one-year max-age includes subdomains") do
      set_hsts(31536000)
      assert_eq(get_security_headers()["Strict-Transport-Security"], "max-age=31536000; includeSubDomains")
    end

    test("records a custom max-age") do
      set_hsts(600)
      assert_eq(get_security_headers()["Strict-Transport-Security"], "max-age=600; includeSubDomains")
    end

    test("takes include_subdomains and preload flags, as documented") do
      pending("bug: set_hsts is declared with arity 1, so the documented include_subdomains/preload raise")
      set_hsts(600, true, true)
      assert_eq(get_security_headers()["Strict-Transport-Security"], "max-age=600; includeSubDomains; preload")
    end

    test("refuses a max-age that is not an integer") do
      assert_raises("set_hsts() expects int max_age, got string") do
        set_hsts("x")
      end
    end
  end

  context("frame and sniffing protections") do
    test("prevent_clickjacking emits X-Frame-Options DENY") do
      prevent_clickjacking()
      assert_eq(get_security_headers(), {"X-Frame-Options": "DENY"})
    end

    test("allow_same_origin_frames emits SAMEORIGIN") do
      allow_same_origin_frames()
      assert_eq(get_security_headers(), {"X-Frame-Options": "SAMEORIGIN"})
    end

    test("set_content_type_options emits nosniff") do
      set_content_type_options()
      assert_eq(get_security_headers(), {"X-Content-Type-Options": "nosniff"})
    end

    test("set_xss_protection formats the mode") do
      set_xss_protection("block")
      assert_eq(get_security_headers(), {"X-XSS-Protection": "1; mode=block"})
    end
  end

  context("policy setters") do
    test("set_referrer_policy records the policy verbatim") do
      set_referrer_policy("no-referrer")
      assert_eq(get_security_headers(), {"Referrer-Policy": "no-referrer"})
    end

    test("set_permissions_policy records the policy verbatim") do
      set_permissions_policy("geolocation=(self), camera=()")
      assert_eq(get_security_headers(), {"Permissions-Policy": "geolocation=(self), camera=()"})
    end

    test("cross-origin isolation setters record their policies") do
      set_coep("require-corp")
      set_coop("same-origin")
      set_corp("same-origin")
      assert_eq(get_security_headers(), {
        "Cross-Origin-Embedder-Policy": "require-corp",
        "Cross-Origin-Opener-Policy": "same-origin",
        "Cross-Origin-Resource-Policy": "same-origin"
      })
    end
  end

  context("presets") do
    test("secure_headers_basic sets frame and sniffing protections") do
      secure_headers_basic()
      headers = get_security_headers()
      assert_eq(headers["X-Frame-Options"], "SAMEORIGIN")
      assert_eq(headers["X-Content-Type-Options"], "nosniff")
    end

    test("secure_headers_api tightens referrer and sniffing only") do
      secure_headers_api()
      headers = get_security_headers()
      assert_eq(headers["Referrer-Policy"], "strict-origin")
      assert_eq(headers["X-Content-Type-Options"], "nosniff")
      assert_null(headers["X-Frame-Options"])
    end

    test("secure_headers applies the standard hardening preset") do
      secure_headers()
      headers = get_security_headers()
      assert_eq(headers["X-Frame-Options"], "SAMEORIGIN")
      assert_eq(headers["X-Content-Type-Options"], "nosniff")
      assert_eq(headers["Referrer-Policy"], "strict-origin-when-cross-origin")
      assert_eq(headers["Permissions-Policy"], "geolocation=(), microphone=(), camera=()")
      assert_eq(headers["Strict-Transport-Security"], "max-age=31536000; includeSubDomains")
    end

    test("secure_headers_strict is the most restrictive preset") do
      secure_headers_strict()
      headers = get_security_headers()
      assert_eq(
        headers["Content-Security-Policy"],
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'"
      )
      assert_eq(headers["X-Frame-Options"], "DENY")
      assert_eq(headers["Referrer-Policy"], "strict-origin")
      assert_eq(headers["Cross-Origin-Embedder-Policy"], "require-corp")
      assert_eq(headers["Strict-Transport-Security"], "max-age=31536000; includeSubDomains")
    end
  end
end

describe("enablement and cookie flags") do
  after_each() do
    enable_security_headers()
    disable_force_secure_cookies()
  end

  test("security headers are enabled by default and force-secure cookies are not") do
    assert(security_headers_enabled())
    assert_not(force_secure_cookies_enabled())
  end

  test("security_headers_enabled reflects the toggle") do
    assert_eq(disable_security_headers(), true)
    assert_not(security_headers_enabled())
    assert_eq(enable_security_headers(), true)
    assert(security_headers_enabled())
  end

  test("force secure cookies toggles round-trip") do
    enable_force_secure_cookies()
    assert(force_secure_cookies_enabled())
    disable_force_secure_cookies()
    assert_not(force_secure_cookies_enabled())
  end
end

describe("set_header (test request header)") do
  after_each() do
    clear_headers()
  end

  test("records a header for later test requests and returns nil") do
    assert_null(set_header("X-Custom-Trace", "spec-run"))
  end
end

describe("h() HTML escaping") do
  test("escapes angle brackets") do
    assert_eq(h("<b>"), "&lt;b&gt;")
  end

  test("escapes ampersand and both quote kinds") do
    assert_eq(h("a & b"), "a &amp; b")
    assert_eq(h("say \"hi\""), "say &quot;hi&quot;")
    assert_eq(h("it's"), "it&#x27;s")
  end

  test("leaves plain text and the empty string untouched") do
    assert_eq(h("hello world"), "hello world")
    assert_eq(h(""), "")
  end

  test("stringifies a non-string") do
    assert_eq(h(5), "5")
  end

  test("renders nil as the empty string") do
    pending("bug: h(nil) returns the string \"null\", which a view would print")
    assert_eq(h(nil), "")
  end
end

describe("j() JavaScript escaping") do
  test("escapes quotes, backslashes and newlines") do
    assert_eq(j("a\\b"), "a\\\\b")
    assert_eq(j("\""), "\\\"")
    assert_eq(j("'"), "\\'")
    assert_eq(j("\n"), "\\n")
  end

  test("escapes angle brackets and ampersand to entities") do
    assert_eq(j("<script>"), "&lt;script&gt;")
    assert_eq(j("&"), "&amp;")
  end

  test("leaves safe text untouched") do
    assert_eq(j("safe text 123"), "safe text 123")
  end
end
