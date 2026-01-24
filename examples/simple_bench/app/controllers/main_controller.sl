fn health(req: Any) -> Any {
    return {
        "status": 200,
        "headers": {"Content-Type": "text/plain"},
        "body": "OK"
    };
}

http_server_get("/health", "health");
