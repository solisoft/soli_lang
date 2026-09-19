# EUI — events

`"on": {"click": "toggle"}` names a server event. When it fires, the handler's
`params` carry the node id, the event kind, its payload, and — because the
server kept the tree — the node's `props`. That is how a row in a list says
which row it is:

```soli
checkbox(item["title"], item["done"], "toggle", {"id": item["id"]})
# … in the handler:
id = params["props"]["id"]
```

An event on a node that has no such handler in the tree the client was sent
is refused and ends the session. Nothing a client sends is trusted before
that check.

## Every event kind

29 of them. A view declares a handler by name under the node's `"on"` hash;
a kind the view has not declared cannot arrive.

| Kind | Fires when |
|------|-----------|
| `click` | A press and release on the node. |
| `double_click` | Two clicks inside the double-click interval. |
| `pointer_down` | The pointer went down on the node. |
| `pointer_up` | …and came back up. |
| `pointer_move` | The pointer moved while over the node. |
| `pointer_enter` | The pointer entered the node. |
| `pointer_leave` | The pointer left it. |
| `key_down` | A key went down while the node had focus. |
| `key_up` | …and came back up. |
| `text_input` | Text was typed or composed into the node. |
| `focus` | The node took keyboard focus. |
| `blur` | It lost focus. |
| `change` | A field's value settled. |
| `submit` | A form-shaped group was submitted. |
| `scroll` | The node was scrolled. |
| `resize` | The node's box changed size. |
| `context_menu` | The secondary (right) button, or its platform equivalent. |
| `drag_start` | A drag began on the node. |
| `drag_over` | A drag passed over it. |
| `drop` | A drag was released on it. |
| `long_press` | A press held past the platform's threshold. |
| `window` | The window itself changed — moved, resized, closed. |
| `ended` | Media finished playing. |
| `time_update` | Media playback position advanced. |
| `wake` | The session's own clock fired. See below. |
| `file_pick` | A file dialog returned a choice. |
| `file_save` | A save dialog returned a destination. |
| `location` | A position fix from the device. Phone only. |
| `nfc_tag` | An NFC tag came within range. Phone only. |

`file_pick`, `file_save`, `location` and `nfc_tag` are answered by the client rather
than the server, and the last two are compiled only for a phone: a desktop has no tag
reader and no positioning it can reach, so the code behind them is not built for it.

## Waking a session

A component may ask to be woken on a clock — that is the `wake` event. On its own that
makes every window a poller: it learns what someone else did only when its own clock
next fires, up to a whole period late, and spends a render per period discovering that
nothing happened.

`eui_wake(component)` is the other half. It renders every *other* live session of a
component immediately, from wherever the change was written:

```soli
def send(state, params)
  Message.create({ "room": state["room"], "body": params["value"] })
  eui_wake("chat")        # every other window showing this component, now
  state
end
```

Nothing in the protocol required the polling shape: a `Batch` is server→client, the
send side is a queue drained by its own task, and a rendered batch goes to the
instance's senders. Call it where the change is written, not on a timer.

## Notifying the person

`eui_notify(title, body, opts)` raises a real notification on the machine the window is
on — the desktop's own, the one every other application uses. It is the one thing a
session can do that reaches somebody who has stopped looking at it.

```soli
def mail_arrived(event)
  msg = Mail.latest()
  eui_notify("Nouveau message", "#{msg.from} : #{msg.subject}", { "tag": "thread-#{msg.thread_id}" })
  { "state": { "unseen": Mail.unseen() } }
end
```

A call and not a node: a notification is something an application *does* once, where
something happened, so it belongs in a handler — a view that calls it says the same
thing again on every render. The title is required; `opts["tag"]` is an identity, and a
second notification carrying the tag of one still on screen replaces it rather than
stacking beside it. Long strings are cut to the protocol's limits rather than refused.

It reaches **the session whose handler is running, and no other**, and returns whether
it was *sent* — never whether it was seen. The client shows it only if the person
granted `notifications` (ask in `config/routes.sl` with `eui_capabilities`), and nothing
comes back over the socket: not shown, not clicked, not dismissed. Called where no
session is rendering — a controller, a job, `soli run` — it does nothing and returns
`false`.

So the pairing with `eui_wake` is the shape to reach for: the job that took the delivery
wakes the component, and each woken session decides for itself whether its own machine
should say anything.
