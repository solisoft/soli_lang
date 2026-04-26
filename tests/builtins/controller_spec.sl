// ============================================================================
// Controller Test Suite
// ============================================================================

describe("Controller Base Class", fn() {
    test("controller can be extended", fn() {
        class TestController extends Controller {
            static {
                this.layout = "application";
            }
        }
        assert(true);
    });

    test("before_action can be set with function", fn() {
        class TestController extends Controller {
            static {
                this.before_action = fn(req) {
                    return req;
                };
            }
        }
        assert(true);
    });

    test("after_action can be set with function", fn() {
        class TestController extends Controller {
            static {
                this.after_action = fn(req) {
                    return req;
                };
            }
        }
        assert(true);
    });

    test("layout can be set via static block", fn() {
        class TestController extends Controller {
            static {
                this.layout = "custom_layout";
            }
        }
        assert(true);
    });

    test("child controller inherits from parent", fn() {
        class ParentController extends Controller {
            static {
                this.layout = "parent";
            }
        }
        class ChildController extends ParentController {
        }
        assert(true);
    });

    test("halt(status, message) builds a response hash the short-circuit logic accepts", fn() {
        // Hooks use `return halt(403, "Forbidden")` to abort a request.
        // The return value must be a hash with a `status` field so
        // `check_for_response` in serve/mod.rs treats it as a response, not as
        // a modified request hash. Regression guard against the builtin going
        // missing (it was undefined for a while, causing hooks to silently
        // no-op and let requests through). Named `halt` rather than `error`
        // because `error` collides with the common local name in form partials.
        let r = halt(403, "Forbidden");
        assert_eq(r["status"], 403);
        assert_eq(r["body"], "Forbidden");
        assert_eq(r["headers"]["Content-Type"], "text/plain; charset=utf-8");

        let r2 = halt(422, "Bad payload");
        assert_eq(r2["status"], 422);
        assert_eq(r2["body"], "Bad payload");
    });

    test("filtered before_action DSL parses at the language level", fn() {
        // `this.before_action(:show) = fn(req) {...}` must actually parse — the
        // parser desugars call-assignment to `this.before_action(:show, fn(req) {...})`
        // and Controller provides a no-op static method so the call resolves.
        // Real hook registration happens via the registry's textual scanner at
        // `soli serve` startup; this test just proves the DSL doesn't raise.
        class FilteredController extends Controller {
            static {
                this.before_action(:show, :edit) = fn(req) {
                    return req;
                };
                this.after_action(:create) = fn(req, response) {
                    return response;
                };
            }
        }
        assert(true);
    });
});

// ============================================================================
// respond_to — Rails-style content negotiation
// ============================================================================

fn _make_req(headers, path, query) {
    return {"headers": headers, "path": path, "query": query};
}

describe("respond_to (DSL form)", fn() {
    test("html-only handler matches Accept: text/html", fn() {
        let req = _make_req({"accept": "text/html"}, "/posts/1", {});
        let res = respond_to(req, fn(format) {
            format.html(fn() {"status": 200, "headers": {}, "body": "html"});
        });
        assert_eq(res["status"], 200);
        assert_eq(res["body"], "html");
    });

    test("json wins over html when Accept asks for json", fn() {
        let req = _make_req({"accept": "application/json"}, "/posts/1", {});
        let res = respond_to(req, fn(format) {
            format.html(fn() {"status": 200, "headers": {}, "body": "html"});
            format.json(fn() {"status": 200, "headers": {}, "body": "json"});
        });
        assert_eq(res["body"], "json");
    });

    test("q-values: json q=0.9 beats html q=0.5", fn() {
        let req = _make_req(
            {"accept": "text/html;q=0.5,application/json;q=0.9"},
            "/posts/1", {}
        );
        let res = respond_to(req, fn(format) {
            format.html(fn() {"status": 200, "headers": {}, "body": "html"});
            format.json(fn() {"status": 200, "headers": {}, "body": "json"});
        });
        assert_eq(res["body"], "json");
    });

    test("URL extension .json beats Accept: text/html", fn() {
        let req = _make_req({"accept": "text/html"}, "/posts/1.json", {});
        let res = respond_to(req, fn(format) {
            format.html(fn() {"status": 200, "headers": {}, "body": "html"});
            format.json(fn() {"status": 200, "headers": {}, "body": "json"});
        });
        assert_eq(res["body"], "json");
    });

    test("?format=xml beats Accept: text/html", fn() {
        let req = _make_req({"accept": "text/html"}, "/posts/1", {"format": "xml"});
        let res = respond_to(req, fn(format) {
            format.html(fn() {"status": 200, "headers": {}, "body": "html"});
            format.xml(fn() {"status": 200, "headers": {}, "body": "xml"});
        });
        assert_eq(res["body"], "xml");
    });

    test("HX-Request: true picks htmx branch over html", fn() {
        let req = _make_req(
            {"hx-request": "true", "accept": "text/html"}, "/posts/1", {}
        );
        let res = respond_to(req, fn(format) {
            format.html(fn() {"status": 200, "headers": {}, "body": "full"});
            format.htmx(fn() {"status": 200, "headers": {}, "body": "partial"});
        });
        assert_eq(res["body"], "partial");
    });

    test("X-Requested-With: XMLHttpRequest picks xhr branch", fn() {
        let req = _make_req(
            {"x-requested-with": "XMLHttpRequest", "accept": "text/html"},
            "/posts/1", {}
        );
        let res = respond_to(req, fn(format) {
            format.html(fn() {"status": 200, "headers": {}, "body": "full"});
            format.xhr(fn()  {"status": 200, "headers": {}, "body": "xhr"});
        });
        assert_eq(res["body"], "xhr");
    });

    test("Accept: */* falls through to first registered handler", fn() {
        let req = _make_req({"accept": "*/*"}, "/posts/1", {});
        let res = respond_to(req, fn(format) {
            format.html(fn() {"status": 200, "headers": {}, "body": "html-first"});
            format.json(fn() {"status": 200, "headers": {}, "body": "json"});
        });
        assert_eq(res["body"], "html-first");
    });

    test("unmatched Accept (pdf) returns 406 when only html registered", fn() {
        let req = _make_req({"accept": "application/pdf"}, "/posts/1", {});
        let res = respond_to(req, fn(format) {
            format.html(fn() {"status": 200, "headers": {}, "body": "html"});
        });
        assert_eq(res["status"], 406);
    });

    test("any catch-all fires when no other format matches", fn() {
        let req = _make_req({"accept": "application/pdf"}, "/posts/1", {});
        let res = respond_to(req, fn(format) {
            format.html(fn() {"status": 200, "headers": {}, "body": "html"});
            format.any(fn()  {"status": 200, "headers": {}, "body": "fallback"});
        });
        assert_eq(res["body"], "fallback");
    });

    test("last-write-wins on duplicate format registration", fn() {
        let req = _make_req({"accept": "application/json"}, "/posts/1", {});
        let res = respond_to(req, fn(format) {
            format.json(fn() {"status": 200, "headers": {}, "body": "first"});
            format.json(fn() {"status": 200, "headers": {}, "body": "second"});
        });
        assert_eq(res["body"], "second");
    });

    test("nested respond_to does not clobber outer state", fn() {
        let req_outer = _make_req({"accept": "text/html"}, "/p", {});
        let req_inner = _make_req({"accept": "application/json"}, "/p", {});
        let res = respond_to(req_outer, fn(format) {
            format.html(fn() {
                let inner = respond_to(req_inner, fn(f) {
                    f.json(fn() {"status": 200, "headers": {}, "body": "inner-json"});
                });
                return {"status": 200, "headers": {}, "body": "outer:" + inner["body"]};
            });
        });
        assert_eq(res["body"], "outer:inner-json");
    });

    test("excel format matches xlsx URL extension", fn() {
        let req = _make_req({"accept": "text/html"}, "/reports/q1.xlsx", {});
        let res = respond_to(req, fn(format) {
            format.html(fn()  {"status": 200, "headers": {}, "body": "html"});
            format.excel(fn() {"status": 200, "headers": {}, "body": "xlsx"});
        });
        assert_eq(res["body"], "xlsx");
    });

    test("csv format matches text/csv Accept header", fn() {
        let req = _make_req({"accept": "text/csv"}, "/exports", {});
        let res = respond_to(req, fn(format) {
            format.html(fn() {"status": 200, "headers": {}, "body": "html"});
            format.csv(fn()  {"status": 200, "headers": {}, "body": "csv"});
        });
        assert_eq(res["body"], "csv");
    });

    test("pdf format matches application/pdf Accept header", fn() {
        let req = _make_req({"accept": "application/pdf"}, "/invoices/1", {});
        let res = respond_to(req, fn(format) {
            format.html(fn() {"status": 200, "headers": {}, "body": "html"});
            format.pdf(fn()  {"status": 200, "headers": {}, "body": "pdf"});
        });
        assert_eq(res["body"], "pdf");
    });
});

describe("respond_to (hash form)", fn() {
    test("hash form dispatches like DSL", fn() {
        let req = _make_req({"accept": "application/json"}, "/posts/1", {});
        let res = respond_to(req, {
            "html": fn() {"status": 200, "headers": {}, "body": "html"},
            "json": fn() {"status": 200, "headers": {}, "body": "json"}
        });
        assert_eq(res["body"], "json");
    });

    test("hash form returns 406 on no match", fn() {
        let req = _make_req({"accept": "application/pdf"}, "/posts/1", {});
        let res = respond_to(req, {
            "html": fn() {"status": 200, "headers": {}, "body": "html"}
        });
        assert_eq(res["status"], 406);
    });

    test("hash form respects insertion order for wildcard fallback", fn() {
        let req = _make_req({"accept": "*/*"}, "/posts/1", {});
        let res = respond_to(req, {
            "html": fn() {"status": 200, "headers": {}, "body": "html-first"},
            "json": fn() {"status": 200, "headers": {}, "body": "json"}
        });
        assert_eq(res["body"], "html-first");
    });
});
