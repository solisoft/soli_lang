// Minimal test
let counter = 0;

fn handler(req: Any) -> Any {
    counter = counter + 1;
    return {
        "status": 200,
        "headers": {"Content-Type": "text/plain"},
        "body": "OK " + str(counter)
    };
}

print("Starting server on port 4000...");
http_server_listen(4000);
