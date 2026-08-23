// ============================================================================
// Security Headers Extended Test Suite
//
// Covers the response security-headers API as pure configuration calls:
// every setter records header state retrievable via get_security_headers().
// Also covers the h() / j() escaping helpers. Complements
// security_headers_spec.sl.
// ============================================================================

describe("CSP setters", fn() {
    test("set_csp records a full policy", fn() {
        reset_security_headers();
        set_csp("default-src 'none'");
        let headers = get_security_headers();
        assert_eq(headers["Content-Security-Policy"], "default-src 'none'");
    });

    # NOTE: set_csp()'s optional second argument (report-only) and
    # set_hsts()'s includeSubDomains/preload flags are declared with a
    # strict arity of 1, so they cannot be exercised from Soli code.
    test("set_csp_default_src builds the directive", fn() {
        reset_security_headers();
        set_csp_default_src("'self'");
        let headers = get_security_headers();
        assert_eq(headers["Content-Security-Policy"], "default-src 'self'");
    });

    test("set_csp_script_src builds the directive", fn() {
        reset_security_headers();
        set_csp_script_src("'self'");
        let headers = get_security_headers();
        assert_eq(headers["Content-Security-Policy"], "script-src 'self'");
    });

    test("set_csp_style_src builds the directive", fn() {
        reset_security_headers();
        set_csp_style_src("'unsafe-inline'");
        let headers = get_security_headers();
        assert_eq(headers["Content-Security-Policy"], "style-src 'unsafe-inline'");
    });
});

describe("HSTS", fn() {
    test("defaults to a one-year policy with subdomains", fn() {
        reset_security_headers();
        set_hsts(31536000);
        assert_eq(get_security_headers()["Strict-Transport-Security"], "max-age=31536000; includeSubDomains");
    });

    test("records a custom max-age", fn() {
        reset_security_headers();
        set_hsts(600);
        assert_eq(get_security_headers()["Strict-Transport-Security"], "max-age=600; includeSubDomains");
    });
});

describe("frame and sniffing protections", fn() {
    test("prevent_clickjacking emits X-Frame-Options DENY", fn() {
        reset_security_headers();
        prevent_clickjacking();
        assert_eq(get_security_headers()["X-Frame-Options"], "DENY");
    });

    test("allow_same_origin_frames emits SAMEORIGIN", fn() {
        reset_security_headers();
        allow_same_origin_frames();
        assert_eq(get_security_headers()["X-Frame-Options"], "SAMEORIGIN");
    });

    test("set_content_type_options emits nosniff", fn() {
        reset_security_headers();
        set_content_type_options();
        assert_eq(get_security_headers()["X-Content-Type-Options"], "nosniff");
    });

    test("set_xss_protection formats the mode", fn() {
        reset_security_headers();
        set_xss_protection("block");
        assert_eq(get_security_headers()["X-XSS-Protection"], "1; mode=block");
    });
});

describe("policy setters", fn() {
    test("set_referrer_policy records the policy verbatim", fn() {
        reset_security_headers();
        set_referrer_policy("no-referrer");
        assert_eq(get_security_headers()["Referrer-Policy"], "no-referrer");
    });

    test("set_permissions_policy records the policy verbatim", fn() {
        reset_security_headers();
        set_permissions_policy("geolocation=(self), camera=()");
        assert_eq(get_security_headers()["Permissions-Policy"], "geolocation=(self), camera=()");
    });

    test("cross-origin isolation setters record their policies", fn() {
        reset_security_headers();
        set_coep("require-corp");
        set_coop("same-origin");
        set_corp("same-origin");
        let headers = get_security_headers();
        assert_eq(headers["Cross-Origin-Embedder-Policy"], "require-corp");
        assert_eq(headers["Cross-Origin-Opener-Policy"], "same-origin");
        assert_eq(headers["Cross-Origin-Resource-Policy"], "same-origin");
    });
});

describe("presets", fn() {
    test("secure_headers_basic sets frame and sniffing protections", fn() {
        reset_security_headers();
        secure_headers_basic();
        let headers = get_security_headers();
        assert_eq(headers["X-Frame-Options"], "SAMEORIGIN");
        assert_eq(headers["X-Content-Type-Options"], "nosniff");
    });

    test("secure_headers_api tightens referrer and sniffing only", fn() {
        reset_security_headers();
        secure_headers_api();
        let headers = get_security_headers();
        assert_eq(headers["Referrer-Policy"], "strict-origin");
        assert_eq(headers["X-Content-Type-Options"], "nosniff");
        assert_null(headers["X-Frame-Options"]);
    });

    test("secure_headers applies the standard hardening preset", fn() {
        reset_security_headers();
        secure_headers();
        let headers = get_security_headers();
        assert_eq(headers["X-Frame-Options"], "SAMEORIGIN");
        assert_eq(headers["X-Content-Type-Options"], "nosniff");
        assert_eq(headers["Referrer-Policy"], "strict-origin-when-cross-origin");
        assert_eq(headers["Permissions-Policy"], "geolocation=(), microphone=(), camera=()");
        assert_eq(
            headers["Strict-Transport-Security"],
            "max-age=31536000; includeSubDomains"
        );
    });

    test("secure_headers_strict is the most restrictive preset", fn() {
        reset_security_headers();
        secure_headers_strict();
        let headers = get_security_headers();
        assert_eq(
            headers["Content-Security-Policy"],
            "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'"
        );
        assert_eq(headers["X-Frame-Options"], "DENY");
        assert_eq(headers["Referrer-Policy"], "strict-origin");
        assert_eq(headers["Cross-Origin-Embedder-Policy"], "require-corp");
        assert_eq(
            headers["Strict-Transport-Security"],
            "max-age=31536000; includeSubDomains"
        );
    });
});

describe("enablement and cookie flags", fn() {
    test("security_headers_enabled reflects the toggle", fn() {
        enable_security_headers();
        assert(security_headers_enabled());
        disable_security_headers();
        assert(!security_headers_enabled());
        enable_security_headers();
        assert(security_headers_enabled());
    });

    test("force secure cookies toggles round-trip", fn() {
        disable_force_secure_cookies();
        assert(!force_secure_cookies_enabled());
        enable_force_secure_cookies();
        assert(force_secure_cookies_enabled());
        disable_force_secure_cookies();
        assert(!force_secure_cookies_enabled());
    });
});

describe("set_header (test request header)", fn() {
    test("records a header for subsequent test requests without error", fn() {
        clear_headers();
        assert_null(set_header("X-Custom-Trace", "spec-run"));
    });
});

describe("h() HTML escaping", fn() {
    test("escapes angle brackets", fn() {
        assert_eq(h("<b>"), "&lt;b&gt;");
    });

    test("escapes ampersand and quotes", fn() {
        assert_eq(h("a & b"), "a &amp; b");
        let quoted = h("say \"hi\"");
        assert(quoted.contains("&quot;"));
        let single = h("it's");
        assert(single.contains("&#x27;"));
    });

    test("leaves plain text untouched", fn() {
        assert_eq(h("hello world"), "hello world");
    });
});

describe("j() JavaScript escaping", fn() {
    test("escapes quotes, backslashes and newlines", fn() {
        assert_eq(j("a\\b"), "a\\\\b");
        assert(j("\"").contains("\\\""));
        assert(j("\n").contains("\\n"));
    });

    test("escapes angle brackets and ampersand to entities", fn() {
        assert_eq(j("<script>"), "&lt;script&gt;");
        assert(j("&").contains("&amp;"));
    });

    test("leaves safe text untouched", fn() {
        assert_eq(j("safe text 123"), "safe text 123");
    });
});
