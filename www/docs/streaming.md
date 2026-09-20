# Streaming & SSE

Stream a response as it is produced — Server-Sent Events for live updates, or a chunked body
for a large export — instead of buffering the whole thing first.

A controller returns `sse(req)` or `stream(req, content_type)` with a block. The server holds
the connection open and sends each chunk over `Transfer-Encoding: chunked` as the block calls
`out.emit(...)` / `out.write(...)`. That suits AI token streaming, progress reporting, live
dashboards, and CSV or JSON exports too large to hold in memory.

## Server-Sent Events

`sse(req) do |out| … end` sends `text/event-stream`. Call `out.emit(data, event?)` once per
event; the browser's `EventSource` receives them live.

```soli
def stream(req)
  sse(req) do |out|
    for item in Notification.live()
      out.emit(item.to_json, "notice")   # event: notice\n data: {...}
    end
    out.emit("done")                      # a plain data: event
  end
end
```

```javascript
const es = new EventSource("/feed/stream")
es.addEventListener("notice", e => console.log(JSON.parse(e.data)))
```

`out.emit` returns `false` once the client has disconnected. Check it to leave an open-ended
loop early rather than producing events nobody is reading.

## Chunked bodies (large exports)

`stream(req, content_type) do |out| … end` sends a raw chunked body. Use `out.write(chunk)` —
no SSE framing — to stream a file or a report without assembling it first.

```soli
def export(req)
  stream(req, "text/csv") do |out|
    out.write("name,score\n")
    for row in Player.order("score desc").each_row()
      out.write(row.name + "," + str(row.score) + "\n")
    end
  end
end
```

## The `out` emitter

| Call | What it does |
|---|---|
| `out.emit(data, event?)` | One SSE event. Multi-line data is split into several `data:` lines. Returns `Bool` — `false` means the client is gone. |
| `out.write(data)` | One raw body chunk, no framing. For `stream`. |
| `out.llm_stream(system, user)` | Streams an LLM completion token by token into this response, emitting each delta as it arrives. Returns the whole answer, so you can persist it. Stops early if the client disconnects. |

It is `emit` and not `send` because `send` is the universal metaprogramming method and taking
that name here would shadow it.

```soli
# Stream an answer over SSE — tokens reach the browser as they generate.
def ask(req)
  sse(req) do |out|
    answer = out.llm_stream("You are concise.", req["query"]["q"])
    ChatLog.create({ "q": req["query"]["q"], "a": answer })
  end
end
```

`out.llm_stream` needs an LLM configured (`SOLI_LLM_API_KEY` / `SOLI_LLM_URL`). To stream an
answer grounded in your own data, retrieve the context first with `Model.rag` or
`Model.similar`, build the prompt from it, and then call `out.llm_stream`.

## Many connections: pub/sub

**A `sse` or `stream` block holds a worker thread for as long as it runs.** That is the right
trade for a finite job, and the wrong one for a dashboard with thousands of viewers who are
mostly idle — one worker per idle connection exhausts the pool.

For that shape, subscribe instead. `sse_subscribe(req, topic)` registers the connection and
returns immediately, holding no worker, and `sse_broadcast(topic, data, event?)` fans an event
out to every subscriber from any controller, job, or model callback.

```soli
# Each browser holds a cheap async connection — not a worker.
def subscribe(req)
  sse_subscribe(req, "user:#{current_user.id}")
end

# Push from anywhere — a controller action, a background job, a callback.
def notify(req)
  reached = sse_broadcast("user:#{params["id"]}", params["msg"], "alert")
  render_json({ "delivered": reached })
end
```

```javascript
const es = new EventSource("/notifications/subscribe")
es.addEventListener("alert", e => toast(e.data))
```

A subscription costs an async task rather than a thread, so one worker can hold thousands of
them. Disconnected clients are pruned on the next broadcast, and `sse_subscribers(topic)`
returns the current count.

## One call, both transports: `broadcast`

`broadcast(channel, payload)` fans `payload` out to both the WebSocket channel and the SSE
topic of the same name, so a page listens over whichever transport it uses and you publish
once. Non-string payloads serialize to JSON; the return value is the SSE subscriber count.

```soli
def create(req)
  post = Post.create(permit(params, {"title": true}))
  # Reaches WS clients in room "posts" AND SSE subscribers of topic "posts".
  broadcast("posts", { "event": "created", "id": post.id, "title": post.title })
  redirect("/posts/#{post.id}")
end
```

Models carry a shortcut: `Post.broadcast(payload)` publishes to the model's own collection
channel (`"posts"`), which suits an `after_save` callback so that every write pushes a change
event to subscribed clients. This is a general pub/sub primitive; for a LiveView that
re-renders itself on writes, prefer [reactive live queries](liveview.md).

## When to use what

| | Use it for |
|---|---|
| **SSE** | One-way server → browser updates: notifications, token streams, progress. Simpler than WebSockets, and reconnects on its own. |
| **WebSockets** | Two-way and low-latency: chat, presence, games. Documented at `/docs/core-concepts/websockets` — it has no markdown reference yet. |
| **[Live View](liveview.md)** | Server-rendered reactive UI, with no client JavaScript to write. |

**Pick the path by lifetime, not by feature.** A `sse` / `stream` block holds one worker thread
until it finishes, so it fits finite, active streams — an agent run, an export — and the pool
should be sized for how many run at once. For many long-lived, mostly-idle connections, use
`sse_subscribe` / `sse_broadcast`, which are async and hold no worker per connection.

Backpressure is automatic either way: a slow client pauses a block, and for a broadcast a full
client drops that message but keeps its subscription.

## See also

- [`liveview.md`](liveview.md) — reactive server-rendered UI over the same connection machinery
- [`jobs.md`](jobs.md) — where a long-running producer usually belongs
- Rendered page: `/docs/core-concepts/streaming`
