// WebSocket controller - handles WebSocket demo

// WebSocket handler - receives events: {type, connection_id, message?, channel?}
// Returns: {broadcast: "msg"} to broadcast to all, {send: "msg"} to reply to sender
fn chat_handler(event: Any) -> Any {
    let event_type = event["type"];
    let connection_id = event["connection_id"];

    if (event_type == "connect") {
        return {
            "broadcast": "{\"type\":\"join\",\"user\":\"" + connection_id + "\"}"
        };
    }

    if (event_type == "disconnect") {
        return {
            "broadcast": "{\"type\":\"leave\",\"user\":\"" + connection_id + "\"}"
        };
    }

    if (event_type == "message") {
        let message = event["message"];
        let parsed = json_parse(message);
        let text = parsed["text"];
        // Use "send" for echo (single client) instead of "broadcast" (all clients)
        return {
            "send": "{\"type\":\"echo\",\"text\":\"" + text + "\"}"
        };
    }

    return {};
}

// Page to display the WebSocket demo
fn demo(req: Any) -> Any {
    return render("websocket/demo", {
        "title": "WebSocket Demo"
    });
}
