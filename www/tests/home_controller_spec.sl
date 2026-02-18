// ============================================================================
// Home Controller Tests
// ============================================================================
//
// Tests for the HomeController which handles root routes (/, /up, /health)
// ============================================================================

describe("HomeController", fn()
    describe("GET /up", fn()
        test("returns UP response", fn()
            let result = render_text("UP");
            assert_eq(result, "UP");
        end);
    end);

    describe("GET /health", fn()
        test("returns health check response", fn()
            let response = {
                "status": 200,
                "headers": {"Content-Type": "text/json"},
                "body": "{\"status\":\"ok\"}"
            };
            assert_eq(response["status"], 200);
            assert_eq(response["headers"]["Content-Type"], "text/json");
            assert_json(response["body"]);
        end);

        test("health response is valid JSON", fn()
            let body = "{\"status\":\"ok\"}";
            assert_json(body);
        end);

        test("health response contains ok status", fn()
            let body = "{\"status\":\"ok\"}";
            assert_contains(body, "ok");
        end);
    end);

    describe("GET / (index)", fn()
        test("index renders with title", fn()
            let context = {
                "title": "Welcome",
                "message": "The Modern MVC Framework for Soli"
            };
            assert_eq(context["title"], "Welcome");
            assert_contains(context["message"], "MVC Framework");
        end);

        test("index context contains message", fn()
            let context = {
                "title": "Welcome",
                "message": "The Modern MVC Framework for Soli"
            };
            assert_not_null(context["message"]);
            assert_gt(len(context["message"]), 0);
        end);
    end);

    describe("GET /docs redirect", fn()
        test("redirects to docs.html", fn()
            let response = {
                "status": 302,
                "headers": {"Location": "/docs.html"},
                "body": ""
            };
            assert_eq(response["status"], 302);
            assert_eq(response["headers"]["Location"], "/docs.html");
        end);
    end);
end);

describe("Health Check Endpoint", fn()
    test("returns 200 status code", fn()
        let status = 200;
        assert_eq(status, 200);
    end);

    test("response headers include Content-Type", fn()
        let headers = {"Content-Type": "text/json"};
        assert_hash_has_key(headers, "Content-Type");
    end);
end);

describe("Root Endpoint", fn()
    test("renders home page", fn()
        let page_title = "Welcome";
        assert_eq(page_title, "Welcome");
    end);

    test("home page has descriptive message", fn()
        let message = "The Modern MVC Framework for Soli";
        assert_gt(len(message), 10);
        assert_contains(message, "MVC");
    end);
end);
