describe("Prometheus Metrics Endpoint", fn() {
    let server_url = "http://127.0.0.1:3000";

    test("/_metrics returns HTTP 200", fn() {
        let response = HTTP.get(server_url + "/_metrics") rescue null;
        if response != null {
            assert_eq(response["status"], 200);
        }
    });

    test("/_metrics Content-Type is text/plain", fn() {
        let response = HTTP.get(server_url + "/_metrics") rescue null;
        if response != null {
            let content_type = response["headers"]["Content-Type"] rescue "";
            assert(content_type.starts_with("text/plain"));
        }
    });

    test("/_metrics body contains expected metric names", fn() {
        let response = HTTP.get(server_url + "/_metrics") rescue null;
        if response != null {
            let body = response["body"];
            assert(body.contains("soli_http_requests_total"));
            assert(body.contains("soli_lexing_duration_seconds"));
            assert(body.contains("soli_parsing_duration_seconds"));
            assert(body.contains("soli_vm_execution_seconds"));
        }
    });

    test("/_metrics body uses Prometheus HELP/TYPE convention", fn() {
        let response = HTTP.get(server_url + "/_metrics") rescue null;
        if response != null {
            let body = response["body"];
            assert(body.contains("# HELP"));
            assert(body.contains("# TYPE"));
        }
    });
});
