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
| `ws_send(connection_id, message)` | `nil` | one specific connection — on another instance too, through the realtime bus |
| `ws_broadcast(message, opts?)` | `nil` | every connected client |
| `ws_broadcast_room(channel, message, opts?)` | `nil` | everyone subscribed to `channel` |
| `broadcast(channel, payload, opts?)` | SSE subscriber count | **both** the WS channel and the SSE topic of that name |
| `ws_close(connection_id, reason)` | `nil` | closes a connection, with a reason — on another instance too |
| `ws_join(channel)` / `ws_leave(channel)` | `nil` | subscribe / unsubscribe the connection whose handler is running |
| `ws_clients()` | `Array<String>` | the id of every connection on this process |
| `ws_clients_in(channel)` | `Array<String>` | ids on this process subscribed to `channel` |
| `ws_count(opts?)` | `Int` | active connections on this process; `{"cluster": true}` adds the other instances' |
| `ws_list_presence(channel)` | `Array<Hash>` | every user in the channel, with metadata, across instances |
| `ws_presence_count(channel)` | `Int` | unique **users**, not connections |
| `ws_get_presence(channel, user_id)` | `Hash \| nil` | one user's presence |
| `realtime_bus()` | `Hash` | which bus carries broadcasts to other processes, and how it is doing |

`ws_join` and `ws_leave` act on the connection whose event the handler is
handling, and return once the join has landed — a `ws_clients_in` or a
broadcast on the next line already sees it. Called anywhere else (an HTTP
action, a job) they raise; return `{"join": channel}` from the handler instead.

```soli
def handle(event)
  if event["message"] == "join"
    ws_join("room:lobby")
    return {"send": "#{ws_clients_in("room:lobby").length} here"}
  end
  {}
end
```

Every helper works outside `soli serve` too — in a script, a spec, the
standalone `soli worker`. There are no local sockets there: the senders skip
the local delivery (and still publish to a [realtime bus](#running-more-than-one-instance)
when one is configured), the readers answer `[]`, `0` or `nil`.

`broadcast` is the one to reach for when a page might be on either transport:
non-string payloads serialize to JSON, and `Model.broadcast(payload, opts?)` is
the shortcut that publishes to a model's own collection channel.

```soli
broadcast("room:lobby", { "event": "joined", "user": name })
```

## Running more than one instance

By default every room lives in one process: a `broadcast` reaches the sockets
and SSE streams of the process that made the call, and nothing else. Run two
`soli serve` instances behind a load balancer — or broadcast from a job in the
standalone `soli worker` — and the other process's clients hear nothing.

`SOLI_REALTIME_BUS` connects them:

| Value | What it does |
|---|---|
| `local` (default) | no bus; everything stays in the process |
| `solidb` | each broadcast is also written to the engine-owned `_rt_bus` collection, and every `soli serve` process delivers what the others wrote, read off its changefeed |
| `solikv` (also `redis`, `valkey`) | each broadcast is `PUBLISH`ed on a channel of the SoliKV — or Redis, or Valkey — named by `SOLIKV_RESP_HOST` / `SOLIKV_RESP_PORT` / `SOLIKV_TOKEN`; every `soli serve` process holds one `SUBSCRIBE` connection to it. No database needed |

```bash
# .env, on every instance and on the job worker
SOLI_REALTIME_BUS=solidb

# or, through SoliKV / Redis / Valkey
SOLI_REALTIME_BUS=solikv
SOLIKV_RESP_HOST=kv.internal
SOLIKV_RESP_PORT=6380
SOLIKV_TOKEN=…
```

With a bus, `broadcast`, `Model.broadcast`, `ws_broadcast`,
`ws_broadcast_room`, `sse_broadcast` and the handler `broadcast`,
`broadcast_room` and `broadcast_channel` actions reach every instance. Each call
delivers to its own process first, then hands the message to a background
publisher and returns — the caller never waits on the bus. A process
ignores its own messages, and never re-publishes what it received.

- **Opt in per call to stay home:** `broadcast(channel, payload, {"local": true})`
  (also on `Model.broadcast`, `ws_broadcast`, `ws_broadcast_room`).
- **Fire and forget.** The changefeed drops events for a lagging subscriber and
  does not cross SoliDB cluster nodes; a `solikv` subscriber that is
  reconnecting misses what was published meanwhile. A message can be lost — the
  contract of a Phoenix PubSub broadcast. Keep anything that must not be missed in the
  database, and let the client re-read it.
- **Targeted sends cross too.** `ws_send(id, …)` and `ws_close(id, …)` for a
  connection another instance holds go over the bus, and that instance
  delivers them — from an action, a job in `soli worker`, anywhere.
- **Presence spans instances.** `track` on one instance shows up in
  `ws_list_presence`, `ws_presence_count` and `ws_get_presence` on all of
  them, and in their clients' `presence_diff`s. A join goes out for a user's
  first connection anywhere, a leave for the last. Each instance owns its own
  connections' metas, announces every change, and restates its whole presence
  every 10 s while it has any, so a lost message heals at the next beat; a new
  instance asks the others and has their state within a second or two. An
  instance that dies without a word is forgotten after 30 s, and its users'
  leaves go out then (expect 30–40 s). Eventually consistent, like
  Phoenix.Presence. `phx_ref`s carry a random per-process tag
  (`"9f2c41ab.12"`), never a host name.
- **Counts are local unless asked.** `broadcast` returns the SSE subscribers
  *of this process*; `ws_clients` and `ws_clients_in` list this process's
  connections. `ws_count({"cluster": true})` adds what the other instances
  reported at their last heartbeat (up to 10 s old); `ws_count()` stays
  this process's.
- **On `solidb`, every broadcast is a database write.** For a very chatty
  room, prefer `solikv`, keep the traffic local with `{"local": true}`, or pin
  its clients to one instance.
- The `solidb` bus lives in the application's **base database**
  (`SOLIDB_DATABASE`), even inside a request that called `switch_tenant`.
- The `solikv` bus publishes on the channel `soli:rt`, or on
  `SOLI_REALTIME_CHANNEL` when set: two apps sharing one SoliKV must set
  distinct channels. In a multi-app (vhost) server, every app but the first
  appends its directory name. Its `PUBLISH` stays out of the dev bar's KV
  panel. Each subscriber pings every 15 s and reconnects, with backoff, after
  45 s of silence.
- Messages over 256 KB are delivered locally but not published.
- `soli worker` publishes but does not subscribe. A script (`soli app.sl`)
  publishes from a background thread: one that exits right after its
  `broadcast` can exit before the message is written.

`realtime_bus()` reports the state of the bus:

```soli
realtime_bus()
# {"backend": "solidb", "connected": true, "published": 42, "received": 17,
#  "dropped": 0, "last_rx_at": 1793366400}
```

`connected` is `true` while this process's subscription is established;
`dropped` counts messages that could not be published (database down, queue
full, too large). Any other `SOLI_REALTIME_BUS` value is reported once at boot
and treated as `local`; a `solidb` bus on an app whose database is SQL does the
same — use `solikv` there.

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
- [`deploy.md`](deploy.md) — running several instances
- [`liveview.md`](liveview.md) — reactive UI over the same machinery
- Rendered page: `/docs/core-concepts/websockets`
