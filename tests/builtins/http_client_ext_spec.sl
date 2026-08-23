// ============================================================================
// HTTP client & test-infrastructure extensions Test Suite
// ============================================================================
// Covers APIs not exercised by tests/builtins/http_spec.sl:
//   - HTTP JSON verbs (get_json/post_json/put_json/patch_json), patch, head,
//     get_jsonp error path, json_parse/json_stringify, HTTP.parallel
//   - res_* response-construction/predicate helpers (plain hashes)
//   - test_server_* state builtins
//   - query-instrumentation assertions that work without a server
//   - Browser DSL + page assertions (skip-guarded: need --browser)
//
// All live HTTP traffic goes to the in-process mock_http server, which answers
// every method/path with 200 + {"ok":true}.
// ============================================================================

let port = mock_http_server_start();
let base = "http://127.0.0.1:" + str(port);

fn make_response(status) {
    return { "status": status };
}

describe("HTTP JSON verbs", fn() {
    test("HTTP.get_json parses the response body into a value", fn() {
        let result = HTTP.get_json(base + "/users");
        assert_eq(result["ok"], true);
    });

    test("HTTP.post_json posts JSON and parses the response", fn() {
        let result = HTTP.post_json(base + "/users", { "name": "Alice" });
        assert_eq(result["ok"], true);
    });

    test("HTTP.put_json puts JSON and parses the response", fn() {
        let result = HTTP.put_json(base + "/users/1", { "name": "Bob" });
        assert_eq(result["ok"], true);
    });

    test("HTTP.patch_json patches JSON and parses the response", fn() {
        let result = HTTP.patch_json(base + "/users/1", { "name": "Carol" });
        assert_eq(result["ok"], true);
    });

    test("HTTP.patch returns the raw response body as a string", fn() {
        // HTTP.patch resolves lazily like HTTP.get; str() forces the value.
        let body = str(HTTP.patch(base + "/users/1", { "name": "Dave" }));
        assert_eq(body, "{\"ok\":true}");
    });

    test("HTTP.head returns a status line string", fn() {
        let status_line = str(HTTP.head(base + "/ping"));
        assert_eq(status_line, "200 OK");
    });

    test("HTTP.get_jsonp raises on a non-JSONP response body", fn() {
        // The mock always answers plain JSON with no callback(...) padding,
        // which is exactly the malformed-JSONP case get_jsonp must reject.
        // The error surfaces when the lazy future is forced (str()).
        let raised = false;
        try {
            let payload = HTTP.get_jsonp(base + "/plain-json");
            str(payload);
        } catch e {
            raised = true;
            assert(str(e).contains("JSONP"));
        }
        assert(raised);
    });
});

describe("HTTP json helpers", fn() {
    test("HTTP.json_parse parses a JSON object", fn() {
        let parsed = HTTP.json_parse("{\"name\":\"Alice\",\"age\":30}");
        assert_eq(parsed["name"], "Alice");
        assert_eq(parsed["age"], 30);
    });

    test("HTTP.json_parse parses arrays and preserves types", fn() {
        let parsed = HTTP.json_parse("[1, 2, 3]");
        assert_eq(parsed.len(), 3);
        assert_eq(parsed[1], 2);

        let nested = HTTP.json_parse("{\"active\": true, \"score\": 9.5, \"note\": null}");
        assert_eq(nested["active"], true);
        assert_eq(nested["score"], 9.5);
        assert_eq(nested["note"], null);
    });

    test("HTTP.json_stringify serializes values to compact JSON", fn() {
        assert_eq(HTTP.json_stringify(42), "42");
        assert_eq(HTTP.json_stringify([1, 2, 3]), "[1,2,3]");
    });

    test("json_stringify and json_parse round-trip a hash", fn() {
        let person = { "name": "Bob", "tags": ["a", "b"], "count": 7 };
        let round_tripped = HTTP.json_parse(HTTP.json_stringify(person));
        assert_eq(round_tripped["name"], "Bob");
        assert_eq(round_tripped["tags"].len(), 2);
        assert_eq(round_tripped["count"], 7);
    });

    test("HTTP.json_parse raises on invalid JSON", fn() {
        let raised = false;
        try {
            HTTP.json_parse("{definitely not json");
        } catch e {
            raised = true;
        }
        assert(raised);
    });
});

describe("HTTP.parallel", fn() {
    test("runs multiple request configs concurrently and returns full responses", fn() {
        let results = HTTP.parallel([
            { "url": base + "/one" },
            { "url": base + "/two", "method": "GET" }
        ]);
        assert_eq(results.len(), 2);
        for response in results {
            assert_eq(response["status"], 200);
            assert_eq(response["body"], "{\"ok\":true}");
            assert_eq(response["headers"]["content-type"], "application/json");
        }
    });

    test("supports POST configs with headers and bodies", fn() {
        let results = HTTP.parallel([
            {
                "url": base + "/create",
                "method": "POST",
                "headers": { "X-Custom": "yes" },
                "body": { "key": "value" }
            }
        ]);
        assert_eq(results.len(), 1);
        assert_eq(results[0]["status"], 200);
        assert_eq(results[0]["status_text"], "OK");
    });

    test("returns an empty array for empty input", fn() {
        let results = HTTP.parallel([]);
        assert_eq(results.len(), 0);
    });
});

describe("res_* helpers on plain response hashes", fn() {
    test("res_status extracts the status field", fn() {
        let response = { "status": 201, "body": "created" };
        assert_eq(res_status(response), 201);
    });

    test("res_body extracts the body field", fn() {
        let response = { "status": 200, "body": "{\"ok\":true}" };
        assert_eq(res_body(response), "{\"ok\":true}");
    });

    test("res_json parses a string body into a value", fn() {
        let response = { "status": 200, "body": "{\"ok\":true,\"n\":3}" };
        let parsed = res_json(response);
        assert_eq(parsed["ok"], true);
        assert_eq(parsed["n"], 3);
    });

    test("res_header looks up headers case-insensitively and defaults to null", fn() {
        let response = {
            "status": 302,
            "headers": { "Location": "/dashboard", "X-Request-Id": "abc123" }
        };
        assert_eq(res_header(response, "location"), "/dashboard");
        assert_eq(res_header(response, "LOCATION"), "/dashboard");
        assert_eq(res_header(response, "X-Request-Id"), "abc123");
        assert_eq(res_header(response, "Missing"), null);
    });

    test("res_location extracts the Location header and res_headers returns them all", fn() {
        let response = { "status": 301, "headers": { "Location": "/new-home" } };
        assert_eq(res_location(response), "/new-home");

        let all = res_headers(response);
        assert_eq(all["Location"], "/new-home");
    });

    test("res_headers defaults to an empty hash when no headers are present", fn() {
        let all = res_headers({ "status": 200 });
        assert_eq(all.len(), 0);
    });

    test("res_status raises when the hash has no status field", fn() {
        let raised = false;
        try {
            res_status({ "body": "no status here" });
        } catch e {
            raised = true;
        }
        assert(raised);
    });
});

describe("res_* status predicates", fn() {
    test("success range is recognized by both ? and bare forms", fn() {
        assert(res_ok?(make_response(200)));
        assert(res_ok?(make_response(204)));
        assert(res_ok(make_response(299)));
        assert_not(res_ok?(make_response(300)));
        assert_not(res_ok?(make_response(404)));
    });

    test("redirect predicates cover 3xx", fn() {
        assert(res_redirect?(make_response(301)));
        assert(res_redirect?(make_response(302)));
        assert(res_redirect(make_response(307)));
        assert_not(res_redirect?(make_response(200)));
    });

    test("not_found / unauthorized / forbidden / unprocessable match exact codes", fn() {
        assert(res_not_found?(make_response(404)));
        assert_not(res_not_found?(make_response(400)));

        assert(res_unauthorized?(make_response(401)));
        assert(res_forbidden?(make_response(403)));
        assert(res_unprocessable?(make_response(422)));

        assert(res_not_found(make_response(404)));
        assert(res_unauthorized(make_response(401)));
        assert(res_forbidden(make_response(403)));
        assert(res_unprocessable(make_response(422)));
    });

    test("client_error covers 4xx and server_error covers 5xx", fn() {
        assert(res_client_error?(make_response(400)));
        assert(res_client_error(make_response(499)));
        assert_not(res_client_error?(make_response(500)));

        assert(res_server_error?(make_response(500)));
        assert(res_server_error(make_response(503)));
        assert_not(res_server_error?(make_response(404)));
    });
});

describe("test server state builtins", fn() {
    // NOTE: test_server_start only reserves a port and flips state flags in
    // this process; the app that serves requests runs as a separate subprocess
    // spawned by the `soli test` parent when an app project exists. There is no
    // app scaffolding under tests/, so these tests assert the observable state
    // machine (start -> running/url -> stop) and deliberately make no request
    // against the port.
    test("start reports a running server with a loopback URL", fn() {
        let started_port = test_server_start();
        assert(started_port > 0);
        assert(test_server_running());

        let url = test_server_url();
        assert(url.starts_with("http://127.0.0.1:"));
        assert(url.contains(str(started_port)));
    });

    test("stop resets the running state and clears the URL", fn() {
        test_server_start();
        assert(test_server_running());
        test_server_stop();
        assert_not(test_server_running());
        assert_eq(test_server_url(), "");
    });
});

describe("viewport readback (no browser required)", fn() {
    test("viewport() with no arguments reports the default viewport", fn() {
        let current = viewport();
        assert_eq(current["width"], 1280);
        assert_eq(current["height"], 800);
        assert_eq(current["mobile"], false);
    });
});

describe("query instrumentation assertions", fn() {
    test("assert_query_count accepts a bare Int count", fn() {
        assert_query_count(3, 3);

        let raised = false;
        try {
            assert_query_count(3, 4);
        } catch e {
            raised = true;
        }
        assert(raised);
    });

    test("assert_query_count reads query_count off a response hash", fn() {
        let response = { "query_count": 5 };
        assert_query_count(response, 5);

        let raised = false;
        try {
            assert_query_count(response, 2);
        } catch e {
            raised = true;
        }
        assert(raised);
    });

    test("assert_max_queries enforces an upper bound", fn() {
        assert_max_queries(5, 10);

        let raised = false;
        try {
            assert_max_queries(8, 5);
        } catch e {
            raised = true;
        }
        assert(raised);
    });

    test("assert_no_n_plus_one passes on a clean response and fails on repeated templates", fn() {
        assert_no_n_plus_one({
            "query_count": 3,
            "n_plus_one": []
        });

        let raised = false;
        try {
            assert_no_n_plus_one({
                "query_count": 11,
                "n_plus_one": [{ "query": "FOR doc IN posts RETURN doc", "count": 11 }]
            });
        } catch e {
            raised = true;
            assert(str(e).contains("N+1"));
        }
        assert(raised);
    });

    test("assert_no_ungrouped_reads passes on coalesced responses and flags stragglers", fn() {
        assert_no_ungrouped_reads({
            "query_count": 1,
            "ungrouped_reads": []
        });

        let raised = false;
        try {
            assert_no_ungrouped_reads({
                "query_count": 3,
                "ungrouped_reads": [
                    { "query": "FOR doc IN posts RETURN doc" },
                    { "query": "FOR doc IN accounts RETURN doc" },
                    { "query": "FOR doc IN tags RETURN doc" }
                ]
            });
        } catch e {
            raised = true;
            assert(str(e).contains("grouped"));
        }
        assert(raised);
    });

    test("dev_queries returns an empty log when no DB work has happened", fn() {
        let queries = dev_queries();
        assert_eq(queries.len(), 0);
    });
});

// ---------------------------------------------------------------------------
// Browser DSL + page-bound assertions.
//
// These require BOTH a browser driver (`soli test --browser`, backed by CDP +
// Chrome) AND a real app served by the test server. Neither exists in this
// environment: tests/ has no app scaffolding, so even with a browser there is
// nothing at the other end of the URL to visit. Probe availability once, like
// kv_spec.sl probes SoliKV, and skip every page-bound test when absent.
// ---------------------------------------------------------------------------
let __browser_ready = false;
try {
    visit("/__soli_browser_probe");
    __browser_ready = true;
    close_browser();
} catch e {
    str(e);
}

describe("Browser DSL", fn() {
    test("visit renders a page and page_* accessors report its state", fn() {
        if not __browser_ready
            return null
        end

        visit("/");
        assert_eq(page_path(), "/");
        assert(page_url().starts_with("http://127.0.0.1:"));
        assert(page_html().contains("<html"));
        assert(page_text().len() >= 0);
        assert(page_title().len() >= 0);
        close_browser();
    });

    test("page_errors starts empty on a healthy page", fn() {
        if not __browser_ready
            return null
        end

        visit("/");
        assert_eq(page_errors().len(), 0);
        assert_no_page_errors();
        close_browser();
    });

    test("wait_for waits until a selector appears", fn() {
        if not __browser_ready
            return null
        end

        visit("/");
        wait_for("body");
        wait_for_text("body");
        close_browser();
    });

    test("interaction verbs dispatch to elements", fn() {
        if not __browser_ready
            return null
        end

        visit("/");
        click("body");
        press("Escape");
        close_browser();
    });

    test("page assertion helpers verify selector/text/path presence", fn() {
        if not __browser_ready
            return null
        end

        visit("/");
        assert_selector("body");
        assert_no_selector("#definitely-missing-element");
        assert_page_path("/");
        close_browser();
    });
});
