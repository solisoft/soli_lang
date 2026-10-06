# WebSocket helpers outside a server: no connections, and every helper
# refuses arguments of the wrong type with a message naming the parameter.

describe("WebSocket helpers outside a server") do
  test("ws_count is 0") do
    assert_eq(ws_count(), 0)
  end

  test("ws_clients is an empty hash") do
    assert_eq(ws_clients(), {})
  end

  test("ws_clients_in a channel nobody joined is an empty hash") do
    assert_eq(ws_clients_in("spec_room"), {})
  end

  test("presence queries on a channel nobody joined answer empty") do
    pending("bug: ws_list_presence/ws_presence_count/ws_get_presence/ws_broadcast_room panic without an async runtime")
    assert_eq(ws_presence_count("spec_room"), 0)
    assert_eq(ws_list_presence("spec_room"), [])
    assert_null(ws_get_presence("spec_room", "user_1"))
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
