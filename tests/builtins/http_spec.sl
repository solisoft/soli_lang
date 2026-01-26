// ============================================================================
// HTTP Functions Test Suite
// ============================================================================
// Tests for HTTP request functions
// ============================================================================

describe("HTTP GET Functions", fn() {
    test("http_get() returns a Future", fn() {
        let future = http_get("https://httpbin.org/get");
        assert_not_null(future);
    });

    test("http_get_json() returns parsed JSON", fn() {
        let response = http_get_json("https://httpbin.org/json");
        assert_not_null(response);
    });

    test("http_get_json() parses headers", fn() {
        let response = http_get_json("https://httpbin.org/headers");
        assert_not_null(response);
    });
});

describe("HTTP POST Functions", fn() {
    test("http_post() returns a Future", fn() {
        let future = http_post("https://httpbin.org/post", "body content");
        assert_not_null(future);
    });

    test("http_post_json() sends JSON data", fn() {
        let data = hash();
        data["name"] = "test";
        let response = http_post_json("https://httpbin.org/post", data);
        assert_not_null(response);
    });

    test("http_post() with headers", fn() {
        let headers = hash();
        headers["Content-Type"] = "text/plain";
        let response = http_post("https://httpbin.org/post", "test", headers);
        assert_not_null(response);
    });
});

describe("HTTP Request Functions", fn() {
    test("http_request() with GET method", fn() {
        let response = http_request("GET", "https://httpbin.org/get");
        assert_not_null(response);
        assert_eq(response["status"], 200);
    });

    test("http_request() with POST method", fn() {
        let response = http_request("POST", "https://httpbin.org/post", null, "data");
        assert_not_null(response);
    });

    test("http_request() with custom headers", fn() {
        let headers = hash();
        headers["Authorization"] = "Bearer token123";
        let response = http_request("GET", "https://httpbin.org/headers", headers);
        assert_not_null(response);
    });

    test("http_request() with PUT method", fn() {
        let response = http_request("PUT", "https://httpbin.org/put");
        assert_not_null(response);
        assert_eq(response["status"], 200);
    });

    test("http_request() with DELETE method", fn() {
        let response = http_request("DELETE", "https://httpbin.org/delete");
        assert_not_null(response);
        assert_eq(response["status"], 200);
    });
});

describe("HTTP Status Check Functions", fn() {
    test("http_ok() returns true for 2xx status", fn() {
        let response = http_request("GET", "https://httpbin.org/get");
        assert(http_ok(response));
    });

    test("http_ok() returns false for 4xx status", fn() {
        let response = http_request("GET", "https://httpbin.org/status/404");
        assert_not(http_ok(response));
    });

    test("http_success() is alias for http_ok()", fn() {
        let response = http_request("GET", "https://httpbin.org/get");
        assert(http_success(response));
    });

    test("http_redirect() returns true for 3xx status", fn() {
        let response = http_request("GET", "https://httpbin.org/redirect-to?url=/get");
        assert(http_redirect(response));
    });

    test("http_client_error() returns true for 4xx status", fn() {
        let response = http_request("GET", "https://httpbin.org/status/400");
        assert(http_client_error(response));
    });

    test("http_client_error() returns false for 200", fn() {
        let response = http_request("GET", "https://httpbin.org/get");
        assert_not(http_client_error(response));
    });

    test("http_server_error() returns true for 5xx status", fn() {
        let response = http_request("GET", "https://httpbin.org/status/500");
        assert(http_server_error(response));
    });

    test("http_server_error() returns false for 200", fn() {
        let response = http_request("GET", "https://httpbin.org/get");
        assert_not(http_server_error(response));
    });
});

describe("HTTP Response Properties", fn() {
    test("response has status property", fn() {
        let response = http_request("GET", "https://httpbin.org/get");
        assert(response["status"] > 0);
    });

    test("response has headers property", fn() {
        let response = http_request("GET", "https://httpbin.org/get");
        assert_not_null(response["headers"]);
    });

    test("response has body property", fn() {
        let response = http_request("GET", "https://httpbin.org/get");
        assert_not_null(response["body"]);
    });

    test("response has url property", fn() {
        let response = http_request("GET", "https://httpbin.org/get");
        assert_not_null(response["url"]);
    });

    test("response has method property", fn() {
        let response = http_request("GET", "https://httpbin.org/get");
        assert_eq(response["method"], "GET");
    });
});
