# WebSockets

Native WebSocket support with Phoenix-inspired presence tracking — for chat,
notifications, dashboards and live collaboration.

## Quick start

Three pieces: a route, a handler, and the browser's own `WebSocket`.

```soli
# config/routes.sl
router_websocket("/ws/echo", "echo#handle")
```

```soli
# app/controllers/echo_controller.sl
def handle(event)
  return { "send": "echo: " + event["message"] } if event["type"] == "message"
  {}
end
```

```javascript
const ws = new WebSocket(`ws://${location.host}/ws/echo`)
ws.onmessage = (e) => console.log(e.data)
ws.onopen = () => ws.send("hello")
// → "echo: hello"
```

That is the whole loop.

## How handlers work

A handler is a plain function taking an event hash and returning a hash of
actions. The runtime dispatches the actions and sends them down the right
sockets.

```soli
def handle(event)
  type = event["type"]
  id   = event["connection_id"]

  return { "broadcast": { "type": "join", "user": id } } if type == "connect"
  return { "broadcast": event["message"] }               if type == "message"

  {}
end
```

### The event

| Key | Meaning |
|---|---|
| `type` | `"connect"`, `"message"` or `"disconnect"` |
| `connection_id` | a UUID identifying this connection |
| `message` | the client's text payload — set only when `type == "message"` |
| `params` | dynamic route segments (`:room_id`) |
| `query` | the parsed query string of the upgrade request |
| `headers` | HTTP headers from the original upgrade |

**A handler must always return a hash.** `{}` means "no action" — right for a
disconnect that only needs server-side cleanup, or a message you want to drop.

### Response actions

Every key in the returned hash triggers an action, and keys combine.

| Key | Effect |
|---|---|
| `send` | reply only to the client that triggered the event |
| `broadcast` | send to every connected client, the sender included |
| `join` | subscribe this connection to a channel |
| `leave` | unsubscribe — automatic on disconnect |
| `broadcast_room` | send to everyone in the connection's most recently joined room |
| `track` | start presence tracking, with metadata |
| `set_presence` | update presence state (typing, away, online) |
| `untrack` | stop tracking — automatic on disconnect |

## A chat room, in three increments

**Broadcast to everyone.** The simplest useful handler:

```soli
def handle(event)
  return { "broadcast": event["message"] } if event["type"] == "message"
  {}
end
```

**Scope to a room.** Broadcasting globally does not survive a second chatroom.
A dynamic segment plus `join` + `broadcast_room` isolates the traffic:

```soli
router_websocket("/ws/room/:room_id", "chat#handle")
```

```soli
def handle(event)
  room = "room:" + event["params"]["room_id"]

  return { "join": room }                       if event["type"] == "connect"
  return { "broadcast_room": event["message"] } if event["type"] == "message"
  # leave is automatic on disconnect
  {}
end
```

**Add presence and typing.** One more action — `track` on connect — and clients
receive `presence_state` / `presence_diff` whenever the user list changes:

```soli
def handle(event)
  room = "room:" + event["params"]["room_id"]
  user = get_current_user()

  if event["type"] == "connect"
    return {
      "join":  room,
      "track": {
        "channel": room,
        "user_id": user["id"],    # required — groups multi-device connections
        "name":    user["name"],
        "avatar":  user["avatar"]
      }
    }
  end

  if event["type"] == "message"
    data = event["message"].to_h

    return { "set_presence": { "channel": room, "state": "typing" } } if data["event"] == "typing"
    return { "set_presence": { "channel": room, "state": "online" } } if data["event"] == "stop_typing"

    return { "broadcast_room": event["message"] }
  end

  # disconnect auto-untracks and emits the leave diff
  {}
end
```

## Presence

**Presence is grouped by `user_id`, not by connection.** Someone with three tabs
appears once. A join fires only when their *first* connection arrives; a leave
only when their *last* one exits.

Clients receive two message shapes. `presence_state` is the initial sync —
replace everything you hold:

```json
{
  "event": "presence_state",
  "payload": {
    "user_123": {
      "metas": [
        { "phx_ref": "1", "state": "online", "name": "Alice" },
        { "phx_ref": "2", "state": "typing", "name": "Alice" }
      ]
    },
    "user_456": { "metas": [{ "phx_ref": "3", "state": "online", "name": "Bob" }] }
  }
}
```

`presence_diff` is every change after that:

```json
{
  "event": "presence_diff",
  "payload": {
    "joins":  { "user_789": { "metas": [{ "phx_ref": "4", "state": "online", "name": "Carol" }] } },
    "leaves": { "user_456": { "metas": [{ "phx_ref": "3", "state": "online", "name": "Bob"   }] } }
  }
}
```

## From the browser

No SDK — the built-in `WebSocket` object.

```javascript
const protocol = location.protocol === 'https:' ? 'wss:' : 'ws:'
const ws = new WebSocket(`${protocol}//${location.host}/ws/room/general`)

ws.send(JSON.stringify({ text: 'Hello!' }))
ws.send(JSON.stringify({ event: 'typing' }))
ws.send(JSON.stringify({ event: 'stop_typing' }))
```

Applying presence:

```javascript
let presences = {}

ws.onmessage = (event) => {
  const data = JSON.parse(event.data)

  if (data.event === 'presence_state') {
    presences = data.payload              // initial sync — replace everything
    renderUserList()
  } else if (data.event === 'presence_diff') {
    Object.entries(data.payload.joins).forEach(([id, p]) => { presences[id] = p })
    Object.entries(data.payload.leaves).forEach(([id])   => { delete presences[id] })
    renderUserList()
  }
}

function renderUserList() {
  const users = Object.entries(presences).map(([userId, { metas }]) => ({
    userId,
    ...metas[0],                          // the first meta drives display
    connectionCount: metas.length
  }))
  // …
}
```

## Reaching the runtime from elsewhere

These builtins let an HTTP route, a background job or a model callback push into
the WebSocket runtime without being a handler itself.

| Call | Answers | What it does |
|---|---|---|
| `ws_send(connection_id, message)` | `null` | one specific connection |
| `ws_broadcast(message)` | `null` | every connected client |
| `ws_broadcast_room(channel, message)` | `null` | everyone subscribed to `channel` |
| `broadcast(channel, payload)` | SSE subscriber count | **both** the WS channel and the SSE topic of that name |
| `ws_close(connection_id, reason)` | `null` | closes with a reason |
| `ws_join(channel)` / `ws_leave(channel)` | `null` | subscribe / unsubscribe the current connection |
| `ws_clients()` | `Array<String>` | every connected client id |
| `ws_clients_in(channel)` | `Array<String>` | ids subscribed to `channel` |
| `ws_count()` | `Int` | total active connections |
| `ws_list_presence(channel)` | `Array<Hash>` | every user in the channel, with metadata |
| `ws_presence_count(channel)` | `Int` | unique **users**, not connections |
| `ws_get_presence(channel, user_id)` | `Hash \| null` | one user's presence |

`broadcast` is the one to reach for when a page might be on either transport:
non-string payloads serialize to JSON, and `Model.broadcast(payload)` is the
shortcut that publishes to a model's own collection channel.

```soli
broadcast("room:lobby", { "event": "joined", "user": name })
```

## Performance

The runtime is async Rust — `async-channel` fan-out, no per-message allocations
on the hot path.

- Prefer `send` over `broadcast` whenever you know the recipient.
- Use rooms to scope fan-out: `broadcast_room` is far cheaper than a global
  `broadcast`.
- Presence diffs fire on join, leave and state change only — they never poll.
- Pre-serialize a broadcast payload once with `hash.to_json`; do not recompute
  it per recipient.

## Which transport

| | Use it for |
|---|---|
| **WebSockets** | two-way and low-latency: chat, presence, games |
| [**SSE / streaming**](streaming.md) | one-way server → browser: notifications, token streams, progress |
| [**Live View**](liveview.md) | server-rendered reactive UI, with no client JavaScript to write |

## See also

- [`streaming.md`](streaming.md) — SSE, chunked bodies, and `broadcast` across both transports
- [`liveview.md`](liveview.md) — reactive UI over the same machinery
- Rendered page: `/docs/core-concepts/websockets`
