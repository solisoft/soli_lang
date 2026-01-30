// ============================================================================
// Security Headers Functions Test Suite
// ============================================================================
// Tests for HTTP security headers configuration
// ============================================================================

describe("Security Headers Enable/Disable", fn() {
    test("enable_security_headers() activates security headers", fn() {
        disable_security_headers();
        let before = security_headers_enabled();
        assert_not(before);
        let result = enable_security_headers();
        assert(result);
        let after = security_headers_enabled();
        assert(after);
    });

    test("disable_security_headers() deactivates security headers", fn() {
        enable_security_headers();
        let before = security_headers_enabled();
        assert(before);
        let result = disable_security_headers();
        assert(result);
        let after = security_headers_enabled();
        assert_not(after);
    });

    test("security_headers_enabled() returns current state", fn() {
        disable_security_headers();
        let result = security_headers_enabled();
        assert_not(result);
    });
});

describe("Content Security Policy", fn() {
    test("set_csp() sets policy", fn() {
        reset_security_headers();
        let result = set_csp("default-src 'self'");
        assert_null(result);
    });

    test("set_csp() with report_only flag", fn() {
        reset_security_headers();
        let result = set_csp("default-src 'self'", true);
        assert_null(result);
    });

    test("set_csp_default_src() builds policy", fn() {
        reset_security_headers();
        set_csp_default_src("'self'", "https://example.com");
        let headers = get_security_headers();
        let csp = headers["Content-Security-Policy"];
        assert_contains(csp, "default-src");
    });

    test("set_csp_script_src() builds script policy", fn() {
        reset_security_headers();
        set_csp_script_src("'self'", "'unsafe-inline'");
        let headers = get_security_headers();
        let csp = headers["Content-Security-Policy"];
        assert_contains(csp, "script-src");
    });

    test("set_csp_style_src() builds style policy", fn() {
        reset_security_headers();
        set_csp_style_src("'self'", "'unsafe-inline'");
        let headers = get_security_headers();
        let csp = headers["Content-Security-Policy"];
        assert_contains(csp, "style-src");
    });
});

describe("HTTP Strict Transport Security", fn() {
    test("set_hsts() sets HSTS header", fn() {
        reset_security_headers();
        set_hsts(31536000);
        let headers = get_security_headers();
        let hsts = headers["Strict-Transport-Security"];
        assert_contains(hsts, "max-age=31536000");
    });

    test("set_hsts() with include_subdomains", fn() {
        reset_security_headers();
        set_hsts(31536000, true);
        let headers = get_security_headers();
        let hsts = headers["Strict-Transport-Security"];
        assert_contains(hsts, "includeSubDomains");
    });

    test("set_hsts() with preload flag", fn() {
        reset_security_headers();
        set_hsts(31536000, true, true);
        let headers = get_security_headers();
        let hsts = headers["Strict-Transport-Security"];
        assert_contains(hsts, "preload");
    });
});

describe("Frame Options", fn() {
    test("prevent_clickjacking() sets DENY", fn() {
        reset_security_headers();
        prevent_clickjacking();
        let headers = get_security_headers();
        assert_eq(headers["X-Frame-Options"], "DENY");
    });

    test("allow_same_origin_frames() sets SAMEORIGIN", fn() {
        reset_security_headers();
        allow_same_origin_frames();
        let headers = get_security_headers();
        assert_eq(headers["X-Frame-Options"], "SAMEORIGIN");
    });
});

describe("XSS Protection", fn() {
    test("set_xss_protection() sets X-XSS-Protection", fn() {
        reset_security_headers();
        set_xss_protection("block");
        let headers = get_security_headers();
        let xss = headers["X-XSS-Protection"];
        assert_contains(xss, "1; mode=block");
    });

    test("set_xss_protection() with different modes", fn() {
        reset_security_headers();
        set_xss_protection("report");
        let headers = get_security_headers();
        let xss = headers["X-XSS-Protection"];
        assert_contains(xss, "1; mode=report");
    });
});

describe("Content Type Options", fn() {
    test("set_content_type_options() sets nosniff", fn() {
        reset_security_headers();
        set_content_type_options();
        let headers = get_security_headers();
        assert_eq(headers["X-Content-Type-Options"], "nosniff");
    });
});

describe("Referrer Policy", fn() {
    test("set_referrer_policy() sets Referrer-Policy", fn() {
        reset_security_headers();
        set_referrer_policy("strict-origin-when-cross-origin");
        let headers = get_security_headers();
        assert_eq(headers["Referrer-Policy"], "strict-origin-when-cross-origin");
    });

    test("set_referrer_policy() with various values", fn() {
        reset_security_headers();
        set_referrer_policy("no-referrer");
        let headers = get_security_headers();
        assert_eq(headers["Referrer-Policy"], "no-referrer");
    });
});

describe("Permissions Policy", fn() {
    test("set_permissions_policy() sets Permissions-Policy", fn() {
        reset_security_headers();
        set_permissions_policy("geolocation=(), microphone=()");
        let headers = get_security_headers();
        assert_eq(headers["Permissions-Policy"], "geolocation=(), microphone=()");
    });
});

describe("Cross-Origin Policies", fn() {
    test("set_coep() sets Cross-Origin-Embedder-Policy", fn() {
        reset_security_headers();
        set_coep("require-corp");
        let headers = get_security_headers();
        assert_eq(headers["Cross-Origin-Embedder-Policy"], "require-corp");
    });

    test("set_coop() sets Cross-Origin-Opener-Policy", fn() {
        reset_security_headers();
        set_coop("same-origin");
        let headers = get_security_headers();
        assert_eq(headers["Cross-Origin-Opener-Policy"], "same-origin");
    });

    test("set_corp() sets Cross-Origin-Resource-Policy", fn() {
        reset_security_headers();
        set_corp("same-site");
        let headers = get_security_headers();
        assert_eq(headers["Cross-Origin-Resource-Policy"], "same-site");
    });
});

describe("Preset Security Header Configurations", fn() {
    test("secure_headers() sets comprehensive headers", fn() {
        reset_security_headers();
        secure_headers();
        let headers = get_security_headers();
        assert_not_null(headers["X-Frame-Options"]);
        assert_not_null(headers["X-Content-Type-Options"]);
        assert_not_null(headers["Referrer-Policy"]);
        assert_not_null(headers["Permissions-Policy"]);
    });

    test("secure_headers_basic() sets basic headers", fn() {
        reset_security_headers();
        secure_headers_basic();
        let headers = get_security_headers();
        assert_not_null(headers["X-Frame-Options"]);
        assert_not_null(headers["X-Content-Type-Options"]);
    });

    test("secure_headers_strict() sets strict headers with CSP", fn() {
        reset_security_headers();
        secure_headers_strict();
        let headers = get_security_headers();
        assert_not_null(headers["Content-Security-Policy"]);
        assert_not_null(headers["Strict-Transport-Security"]);
        assert_not_null(headers["X-Frame-Options"]);
        assert_not_null(headers["Cross-Origin-Embedder-Policy"]);
    });

    test("secure_headers_api() sets API-appropriate headers", fn() {
        reset_security_headers();
        secure_headers_api();
        let headers = get_security_headers();
        assert_not_null(headers["X-Content-Type-Options"]);
        assert_not_null(headers["Referrer-Policy"]);
    });
});

describe("Security Headers Reset and Get", fn() {
    test("reset_security_headers() clears configuration", fn() {
        enable_security_headers();
        set_csp("'self'");
        set_hsts(31536000);
        reset_security_headers();
        let headers = get_security_headers();
        assert_null(headers["Content-Security-Policy"]);
        assert_null(headers["Strict-Transport-Security"]);
    });

    test("get_security_headers() returns configured headers", fn() {
        reset_security_headers();
        set_content_type_options();
        let headers = get_security_headers();
        assert_not_null(headers);
        assert_eq(headers["X-Content-Type-Options"], "nosniff");
    });

    test("get_security_headers() returns empty when disabled", fn() {
        disable_security_headers();
        let headers = get_security_headers();
        assert_null(headers);
    });
});
