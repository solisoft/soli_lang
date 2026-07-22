# Notifications

Raise a real OS notification from server-side Soli, inside a packaged desktop or mobile app.

An embedded web view has **no Notifications API and no Push API** — Android's `WebView` and
`WKWebView` both reserve them for the browser. So an app that ships web push reaches browsers and
installed PWAs, and silently reaches nothing inside its own shell. This closes that gap for a client
that has the app open; [Apple push](/docs/native/push-apple) and
[Android push](/docs/native/push-android) cover one that does not.

## One line in your layout

```erb
<% user_id = session_get("user_id") %>
<%- native_channel("user:#{str(user_id)}") rescue "" unless user_id.nil? %>
```

That emits a `<meta name="soli-native">` tag naming the channel this page listens to. Its presence is
what switches the bridge on — a page that never calls it downloads no script and opens no
connection.

## Sending

```soli
Native.notify("user:42", {
  "title": "New ping",
  "body":  "Ana replied to your comment",
  "url":   "/pings/3"
})
```

| Key | |
|---|---|
| `title` | Required in practice — a notification with no title shows as blank. |
| `body` | The line under the title. |
| `url` | Opened when the notification is clicked. |
| `tag` | A stable id: a second notification with the same tag *replaces* the first rather than stacking. |
| `icon` | Overrides the app icon, where the platform supports it. |

## A real example

A comment reply, notifying everyone on the thread except its author:

```soli
class Comment < Model
  def notify_thread
    for participant in this.thread_participants()
      next if participant["_key"] == this.author_id

      reached = Native.notify("user:#{participant["_key"]}", {
        "title": "#{this.author_name} replied",
        "body":  this.body.truncate(120),
        "url":   "/threads/#{this.thread_id}#comment-#{this._key}",
        "tag":   "thread-#{this.thread_id}"
      })

      # Nobody looking: fall through to a push service.
      WebPush.deliver_to_user(participant["_key"], "New reply", this.body, "/threads/#{this.thread_id}") if reached == 0
    end
  end
end
```

The `tag` matters here: ten replies to one thread collapse into one notification rather than ten.

## Knowing whether anyone is listening

`notify` returns the number of clients it reached, and `Native.subscribers(channel)` asks without
sending. That return value is what lets an app choose a transport without branching on platform:

```soli
if Native.subscribers("user:#{str(user_id)}") == 0
  # They are away — a digest email is more use than a notification nobody sees.
  DigestMailer.queue(user_id)
end
```

## Existing `Notification` code keeps working

Inside a shell the client script replaces `window.Notification` with one routed through the bridge:

```js
new Notification("Saved", { body: "Your changes are live" })
```

That keeps working in the shell — where the global does not otherwise exist — and keeps using real
Web Notifications in a browser. `Notification.requestPermission()` resolves to `"granted"`, because
the shell owns the OS-level permission and a second in-page prompt would be meaningless.

## Channel names and their tokens

Channels are yours to choose: `user:42`, `room:7`, `deploy:prod`. They may not contain `.`, `|` or
control characters, and are namespaced internally so they can never collide with — or be reached
through — an app's own `sse_broadcast` topics.

The channel travels as a **signed token**, not as plain text. Subscribing is a `GET` the browser
makes, so an unsigned `?channel=user:42` would let anyone listen to anyone. The token is keyed by
HKDF-SHA256 from `SOLI_SESSION_SECRET` (32+ characters, the same secret sealed cookies use, with its
own domain-separating label), carries a 12-hour expiry, and is verified before any subscription is
accepted. Rotating the secret invalidates outstanding tokens.

`Native.channel_token(channel)` mints one directly, for a client that is not a rendered page.

## Connection behaviour

The client subscribes over SSE and reconnects with exponential backoff up to 30 seconds. It drops
the connection while the tab is hidden — an idle backgrounded stream costs the server a task for
nothing — and reconnects when it returns. Thousands of idle subscribers cost async tasks, not worker
threads.

## Requirements

`SOLI_SESSION_SECRET`, 32+ characters. Without it `native_channel` raises rather than emitting an
unsigned tag. Nothing else — no push service, no certificates, no keys.
