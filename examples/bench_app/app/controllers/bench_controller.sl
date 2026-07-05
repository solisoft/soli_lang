// Benchmark controller - minimal routing
def health(req: Any) -> Any {
    return {
        "status": 200,
        "headers": {"Content-Type": "text/plain"},
        "body": "OK"
    };
}

def hello(req: Any) -> Any {
    return {
        "status": 200,
        "headers": {"Content-Type": "application/json"},
        "body": "{\"message\": \"Hello\"}"
    };
}
