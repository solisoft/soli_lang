# WebSocket demo controller.
#
# Showcases the *server-side helper functions* — `ws_send`, `ws_broadcast` —
# instead of the return-action pattern. The handler returns {} every time and
# dispatches messages imperatively. This is the preferred style when you need
# conditional sends, multiple targets, or non-WebSocket logic mixed in.
#
# `ws_send` and `ws_broadcast` accept a hash directly and JSON-serialize it
# automatically, so no `.to_json` or `JSON.stringify` is needed.
def chat_handler(event)
  conn_id = event["connection_id"]

  if event["type"] == "connect"
    # Greet just this new connection.
    ws_send(conn_id, { "type": "welcome", "id": conn_id })
    # And tell everyone else someone joined.
    ws_broadcast({ "type": "join", "user": conn_id })
    return {}
  end

  if event["type"] == "disconnect"
    ws_broadcast({ "type": "leave", "user": conn_id })
    return {}
  end

  if event["type"] == "message"
    data = event["message"].to_h
    ws_broadcast({
      "type": "message",
      "text": data["text"],
      "from": conn_id
    })
    return {}
  end

  {}
end

def demo
  render("websocket/demo", { "title": "WebSocket Demo" })
end
