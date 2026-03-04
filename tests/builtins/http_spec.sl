// HTTP Class Tests
// Tests HTTP.get, HTTP.post, HTTP.put, HTTP.delete, HTTP.request

describe("HTTP", fn() {
    test("HTTP.get fetches a URL", fn() {
        let response = HTTP.get("https://httpbin.org/get");
        print("HTTP.get response:", response);
        assert(response.len() > 0);
    });

    test("HTTP.post sends data", fn() {
        let response = HTTP.post("https://httpbin.org/post", {"key" => "value"});
        print("HTTP.post response:", response);
        assert(response.len() > 0);
    });

    test("HTTP.put updates data", fn() {
        let response = HTTP.put("https://httpbin.org/put", {"data" => "test"});
        print("HTTP.put response:", response);
        assert(response.len() > 0);
    });

    test("HTTP.delete removes resource", fn() {
        let response = HTTP.delete("https://httpbin.org/delete");
        print("HTTP.delete response:", response);
        assert(response.len() > 0);
    });

    test("HTTP.request with custom method", fn() {
        let response = HTTP.request("GET", "https://httpbin.org/get", {});
        print("HTTP.request response:", response);
        assert(response.len() > 0);
    });
});
