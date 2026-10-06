# Server-sent events outside a server: the in-memory pub/sub reaches nobody,
# and the request-bound helpers refuse calls they cannot serve.

describe("SSE pub/sub") do
  test("a broadcast to a topic nobody follows reaches 0 clients") do
    assert_eq(sse_broadcast("spec_quiet_topic", "hello"), 0)
  end

  test("a broadcast with a custom event name reaches 0 clients too") do
    assert_eq(sse_broadcast("spec_other_topic", "data", "custom-event"), 0)
  end

  test("a topic nobody follows has 0 subscribers, before and after a broadcast") do
    assert_eq(sse_subscribers("spec_counted_topic"), 0)
    sse_broadcast("spec_counted_topic", "data")
    assert_eq(sse_subscribers("spec_counted_topic"), 0)
  end

  test("sse_subscribers refuses a topic that is not a string") do
    assert_raises("sse_subscribers(topic) requires a topic string") do
      sse_subscribers(42)
    end
  end

  test("sse_broadcast refuses a nil topic") do
    assert_raises("sse_broadcast(topic, data, event?) requires a topic string") do
      sse_broadcast(nil, "data")
    end
  end
end

describe("SSE request helpers") do
  test("sse needs a block") do
    assert_raises("sse(req) requires a block") do
      sse({})
    end
  end

  test("sse_subscribe needs a topic") do
    assert_raises("sse_subscribe(req, topic) requires a topic string") do
      sse_subscribe({})
    end
  end

  test("stream needs a block") do
    assert_raises("stream(req, content_type) requires a block") do
      stream({}, "text/csv")
    end
  end
end
