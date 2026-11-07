# WebSocket helpers outside a server: no connections, and every helper
# refuses arguments of the wrong type with a message naming the parameter.

describe("WebSocket helpers outside a server") do
  test("ws_count is 0") do
    assert_eq(ws_count(), 0)
  end

  test("ws_clients is an empty array of connection ids") do
    assert_eq(ws_clients(), [])
  end

  test("ws_clients_in a channel nobody joined is an empty array") do
    assert_eq(ws_clients_in("spec_room"), [])
  end

  test("presence queries on a channel nobody joined answer empty") do
    assert_eq(ws_presence_count("spec_room"), 0)
    assert_eq(ws_list_presence("spec_room"), [])
    assert_null(ws_get_presence("spec_room", "user_1"))
  end

  test("the senders do nothing instead of failing") do
    assert_null(ws_broadcast("hello"))
    assert_null(ws_broadcast({"kind": "ping"}, {"local": true}))
    assert_null(ws_broadcast_room("spec_room", "hello"))
    assert_null(ws_broadcast_room("spec_room", "hello", {"local": true}))
    assert_null(ws_send("6f1c1d1e-2b8a-4c55-9d0e-3a4f5b6c7d8e", "hello"))
    assert_null(ws_close("6f1c1d1e-2b8a-4c55-9d0e-3a4f5b6c7d8e", "bye"))
  end

  test("ws_join and ws_leave need a WebSocket handler to act on") do
    assert_raises("ws_join() only works inside a WebSocket handler") do
      ws_join("spec_room")
    end
    assert_raises("ws_leave() only works inside a WebSocket handler") do
      ws_leave("spec_room")
    end
  end
end

describe("broadcast across processes") do
  test("broadcast returns this process's SSE subscribers") do
    assert_eq(broadcast("spec_channel", {"id": 1}), 0)
  end

  test("broadcast takes {local: true} to stay in this process") do
    assert_eq(broadcast("spec_channel", "hello", {"local": true}), 0)
    assert_eq(broadcast("spec_channel", "hello", {"local": false}), 0)
  end

  test("broadcast refuses an unknown option") do
    assert_raises("broadcast() unknown option \"locale\" (local)") do
      broadcast("spec_channel", "hello", {"locale": true})
    end
  end

  test("broadcast refuses options that are not a hash") do
    assert_raises("broadcast() options must be a hash, got string") do
      broadcast("spec_channel", "hello", "local")
    end
  end

  test("realtime_bus reports the local backend by default") do
    bus = realtime_bus()
    assert_eq(bus["backend"], "local")
    assert_eq(bus["connected"], false)
    assert_eq(bus["published"], 0)
    assert_eq(bus["dropped"], 0)
    assert_null(bus["last_rx_at"])
  end
end

describe("WebSocket helper argument validation") do
  test("ws_join needs a string channel") do
    assert_raises("ws_join() expects string channel, got int") do
      ws_join(123)
    end
  end

  test("ws_leave needs a string channel") do
    assert_raises("ws_leave() expects string channel, got int") do
      ws_leave(123)
    end
  end

  test("ws_clients_in needs a string channel") do
    assert_raises("ws_clients_in() expects string channel, got int") do
      ws_clients_in(123)
    end
  end

  test("ws_send needs a string connection id") do
    assert_raises("ws_send() expects string connection_id, got int") do
      ws_send(42, "hello")
    end
  end

  test("ws_close needs a UUID connection id") do
    assert_raises("Invalid UUID format") do
      ws_close("definitely-not-a-uuid", "reason")
    end
  end

  test("ws_broadcast_room needs a string channel") do
    assert_raises("ws_broadcast_room() expects string channel, got int") do
      ws_broadcast_room(42, "message")
    end
  end

  test("ws_list_presence needs a string channel") do
    assert_raises("ws_list_presence() expects string channel, got null") do
      ws_list_presence(nil)
    end
  end

  test("ws_presence_count needs a string channel") do
    assert_raises("ws_presence_count() expects string channel, got array") do
      ws_presence_count([])
    end
  end

  test("ws_get_presence needs a string user id") do
    assert_raises("ws_get_presence() expects string user_id, got int") do
      ws_get_presence("room", 42)
    end
  end
end
