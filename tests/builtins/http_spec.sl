// HTTP Class Tests
// Tests HTTP.get, HTTP.post, HTTP.put, HTTP.delete, HTTP.request against a
// loopback mock server (was: httpbin.org — slow + flaky + offline-hostile).

let port = mock_http_server_start();
let base = "http://127.0.0.1:" + str(port);

describe("HTTP", fn() {
    test("HTTP.get fetches a URL", fn() {
        let response = HTTP.get(base + "/get");
        assert(response.len() > 0);
    });

    test("HTTP.post sends data", fn() {
        let response = HTTP.post(base + "/post", {"key" => "value"});
        assert(response.len() > 0);
    });

    test("HTTP.put updates data", fn() {
        let response = HTTP.put(base + "/put", {"data" => "test"});
        assert(response.len() > 0);
    });

    test("HTTP.delete removes resource", fn() {
        let response = HTTP.delete(base + "/delete");
        assert(response.len() > 0);
    });

    test("HTTP.request with custom method", fn() {
        let response = HTTP.request("GET", base + "/get", {});
        assert(response.len() > 0);
    });

    test("HTTP.get_all fetches multiple URLs in parallel as raw text", fn() {
        let urls = [base + "/a", base + "/b", base + "/c"];
        let responses = HTTP.get_all(urls);
        assert_eq(responses.len(), 3);
        for r in responses {
            assert_eq(r, "{\"ok\":true}");
        }
    });

    test("HTTP.get_all_json fetches multiple URLs in parallel and parses JSON", fn() {
        let urls = [base + "/a", base + "/b", base + "/c"];
        let responses = HTTP.get_all_json(urls);
        assert_eq(responses.len(), 3);
        for r in responses {
            assert_eq(r["ok"], true);
        }
    });

    test("HTTP.get_all_json returns empty array for empty input", fn() {
        let responses = HTTP.get_all_json([]);
        assert_eq(responses.len(), 0);
    });
});
