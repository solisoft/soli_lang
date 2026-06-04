# WebSocket echo handler. Regression coverage for the HTTP/1.1 101 upgrade
# path: `serve_connection_with_upgrades` must hand the connection over after
# the 101, or no frame ever reaches the handler and nothing comes back.
fn handle(event: Any) -> Any {
    if event["type"] == "message" {
        let body = event["message"] ?? "";
        return { "send": "echo:" + body };
    }
    return {};
}
