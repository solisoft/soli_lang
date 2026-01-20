---
title: WebSocket Support
description: WebSocket support in Soli MVC framework
---

# WebSocket Support

Soli's MVC framework includes WebSocket support for real-time, bidirectional communication between clients and the server.

## Overview

WebSockets provide persistent, full-duplex connections that remain open, allowing both the server and client to send messages at any time. This is ideal for:
- Real-time chat applications
- Live notifications
- Collaborative editing
- Live dashboards
- Gaming

## Registering WebSocket Routes

WebSocket routes are registered in `config/routes.soli` using the `websocket()` DSL function:

```soli
websocket("/path", "controller#handler")
```

**Parameters:**
- `path`: WebSocket endpoint path (e.g., `/chat`, `/notifications`)
- `handler`: Controller action in `"controller#action"` format

## WebSocket Handler

A WebSocket handler is a single function that handles all event types (connect, message, disconnect):

```soli
fn ws_handler(event: Any) -> Any {
    # Access event properties
    let event_type = event["type"];           # "connect", "message", or "disconnect"
    let connection_id = event["connection_id"];  # Unique ID for this client
    let message = event["message"];           # Only for "message" events

    # Return actions to control behavior
    return {
        # Optional: send a message to this client
        "send": "Welcome!",

        # Optional: broadcast to all connected clients
        "broadcast": "A new user connected",

        # Optional: broadcast to a channel/room
        "broadcast_room": "updates",

        # Optional: join a channel
        "join": "general",

        # Optional: leave a channel
        "leave": "old_channel",

        # Optional: close this connection
        "close": "Goodbye!"
    };
}

websocket("/chat", "chat#ws_handler");
```

## Event Types

WebSocket events are passed to your handler as a Hash with these fields:

| Field | Type | Description |
|-------|------|-------------|
| `type` | String | Event type: `"connect"`, `"message"`, or `"disconnect"` |
| `connection_id` | String | Unique identifier for this client connection |
| `message` | String? | Message content (only for `"message"` events) |
| `channel` | String? | Channel name (if applicable) |

## Handler Response Actions

The handler can return a Hash with these optional actions:

| Action | Type | Description |
|--------|------|-------------|
| `send` | String | Send a message to this specific client |
| `broadcast` | String | Broadcast a message to all connected clients |
| `broadcast_room` | String | Broadcast to all clients in a specific channel |
| `join` | String | Join this connection to a channel |
| `leave` | String | Remove this connection from a channel |
| `close` | String | Close the connection (with optional reason) |

## Example: Simple Chat

### app/controllers/chat_controller.soli

```soli
fn chat_connect(event: Any) -> Any {
    # New connection - welcome the user
    print("User connected: " + event["connection_id"]);

    return {
        "send": "Welcome to the chat!",
        "broadcast": "A new user has joined"
    };
}

fn chat_message(event: Any) -> Any {
    # Handle incoming message
    let connection_id = event["connection_id"];
    let message = event["message"];

    print("Received from " + connection_id + ": " + message);

    # Broadcast to all connected clients
    return {
        "broadcast": message
    };
}

fn chat_disconnect(event: Any) -> Any {
    # User disconnected
    print("User disconnected: " + event["connection_id"]);

    return {
        "broadcast": "A user has left the chat"
    };
}
```

### config/routes.soli

```soli
websocket("/chat", "chat#handle_websocket");

# Single handler that dispatches based on event type
fn handle_websocket(event: Any) -> Any {
    let event_type = event["type"];

    if event_type == "connect" {
        return chat_connect(event);
    } else if event_type == "message" {
        return chat_message(event);
    } else if event_type == "disconnect" {
        return chat_disconnect(event);
    }

    return null;
}
```

## Example: Channels/Rooms

```soli
fn channel_handler(event: Any) -> Any {
    let event_type = event["type"];
    let connection_id = event["connection_id"];

    if event_type == "connect" {
        # Auto-join default channel
        return {
            "join": "general",
            "send": "Connected! You are in the general channel."
        };
    }

    if event_type == "message" {
        let message = event["message"];

        # Parse channel command
        if message == "/join rooms" {
            return {
                "join": "rooms",
                "send": "Joined the rooms channel"
            };
        }

        if message == "/join general" {
            return {
                "join": "general",
                "send": "Switched to general channel"
            };
        }

        # Echo message back to sender
        return {
            "send": "You said: " + message
        };
    }

    return null;
}

websocket("/live", "live#channel_handler");
```

## Client-Side Connection

Connect to the WebSocket endpoint from a browser:

```javascript
const socket = new WebSocket('ws://localhost:3000/chat');

socket.onopen = function() {
    console.log('Connected to WebSocket');
    socket.send('Hello server!');
};

socket.onmessage = function(event) {
    console.log('Received:', event.data);
};

socket.onclose = function(event) {
    console.log('Disconnected:', event.reason);
};

socket.onerror = function(error) {
    console.error('WebSocket error:', error);
};
```

## Server-Side Broadcast

You can also trigger broadcasts from regular HTTP routes:

```soli
fn broadcast_notification(req: Any) -> Any {
    let message = req["body"];

    # Access the global WebSocket registry
    let registry = get_ws_registry();

    # Broadcast to all connected clients
    registry.broadcast_all(message);

    return {
        "status": 200,
        "body": "Notification sent to " + str(registry.connection_count()) + " clients"
    };
}

http_server_post("/admin/broadcast", "admin#broadcast_notification");
```

## API Reference

### WebSocketRegistry Methods

The global registry provides these methods:

| Method | Parameters | Description |
|--------|------------|-------------|
| `broadcast_all(message)` | String | Send message to all connected clients |
| `broadcast_to_channel(channel, message)` | String, String | Send message to all clients in a channel |
| `send_to(connection_id, message)` | String, String | Send message to a specific client |
| `join_channel(connection_id, channel)` | String, String | Add a connection to a channel |
| `leave_channel(connection_id, channel)` | String, String | Remove a connection from a channel |
| `close(connection_id, reason)` | String, String | Close a specific connection |
| `connection_count()` | - | Get number of active connections |
| `get_all_ids()` | - | Get all connection IDs |

### Global Registry Access

```soli
let registry = get_ws_registry();

# Broadcast to everyone
registry.broadcast_all("Server is restarting in 5 minutes");

# Get connection count
let count = registry.connection_count();
print("Active connections:", count);

# Send to specific client
registry.send_to(connection_id, "Your order is ready!");
```

## Hot Reload

WebSocket handlers support hot reload. When you modify a controller file, the new handler code will be used for subsequent events without restarting the server.

## Best Practices

1. **Keep handlers lightweight** - Complex operations should be deferred to background tasks
2. **Validate messages** - Always validate client input before processing
3. **Use channels for targeted messaging** - More efficient than filtering on the client
4. **Handle disconnections gracefully** - Clean up resources when clients disconnect
5. **Limit message sizes** - Set reasonable limits to prevent memory issues

## Limitations

- Currently supports text messages only
- No binary message support yet
- No automatic reconnection (client-side)
- No message queuing for offline clients
