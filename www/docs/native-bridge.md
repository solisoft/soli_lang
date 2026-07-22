# Native Bridge

Reach the shell your app is being viewed in.

While the app is **open**, server-side Soli code raises a real OS notification with no push service,
no certificates and no keys. For an app that is **closed**, see
[APNs](#reaching-a-closed-app-apns) further down — which needs all three, and a paid Apple
Developer account.

A Soli app packaged with [`soli desktop build`](/docs/development-tools/desktop), or wrapped in a
WebView on a phone, renders inside an embedded web view — and **neither `WKWebView` nor Android's
`WebView` implements the Push API or the Notifications API**. Both platforms reserve those for the
browser proper. So an app that ships web push reaches browsers and installed PWAs, and silently
reaches nothing at all inside its own native shell.

The native bridge is the missing channel: server-side Soli code addressing the client that is
currently looking at the page.

```soli
Native.notify("user:42", {
  "title": "New ping",
  "body":  "Ana replied to your comment",
  "url":   "/pings/3"
})
```

In a shell that raises a real OS notification. In a browser it raises a Web Notification. Where
neither is available it does nothing.

## Per feature

| Page | |
|---|---|
| [Notifications](/docs/native/notifications) | `Native.notify` reaches a client with the app open — no push service, no keys. |
| [Camera & Microphone](/docs/native/camera) | `getUserMedia`, once the shell stops denying it silently. |
| [Apple Push (APNs)](/docs/native/push-apple) | `Apns.send` reaches a macOS or iOS app that is closed. |
| [Android Push (FCM)](/docs/native/push-android) | `Fcm.send` reaches a closed Android app, where Doze kills connections. |

## What reaches whom

The bridge covers the app-is-open case, and nothing else. That is a deliberate boundary rather than
a shortcoming — it is what lets it work with no push service, no certificates and no keys — but you
need the whole picture to choose transports:

| Client | App open | App closed |
|---|---|---|
| Browser | bridge (or Web Notification) | web push (VAPID) |
| Installed PWA | bridge | web push (VAPID) |
| macOS / iOS shell | **bridge** | [APNs](#reaching-a-closed-app-apns) |
| Android shell | **bridge** | FCM (not yet in soli) |

The bridge is the only thing that reaches the two shell rows on the left, because an embedded web
view has neither the Push API nor the Notifications API. Everything on the right needs a push
service, because a closed app is not executing and something else has to be listening.

`notify` returns how many clients it reached, which makes the two compose without a branch on
platform:

```soli
reached = Native.notify("user:#{str(user_id)}", payload)
if reached == 0
  Apns.send(device_token, payload, apns_options)   # or WebPush, for a browser
end
```

One case where the right-hand column is simply empty: a desktop app carrying its **own** database
has nothing writing to it while closed, so there is nothing to announce. There the bridge is not a
partial answer — it is the whole one.

## Turning it on

One helper in your layout names the channel this page listens to:

```erb
<% user_id = session_get("user_id") %>
<%- native_channel("user:#{str(user_id)}") rescue "" unless user_id.nil? %>
```

That emits a `<meta name="soli-native">` tag whose presence is what switches the bridge on. A page
that never calls it downloads no script and opens no connection.

The channel travels as a **signed token**, not as plain text. Subscribing is a `GET` the browser
makes, so an unsigned `?channel=user:42` would let anyone listen to anyone. The token is keyed by
HKDF-SHA256 from `SOLI_SESSION_SECRET` (32+ characters, the same secret sealed cookies use, with its
own domain-separating label), carries a 12-hour expiry, and is verified before any subscription is
accepted. Rotating the secret invalidates outstanding tokens, exactly as it does sealed cookies.

## API

| Call | Returns | |
|---|---|---|
| `Native.notify(channel, payload)` | `Int` | Clients reached. `0` means nobody has the app open. |
| `Native.subscribers(channel)` | `Int` | Live listeners, without sending anything. |
| `Native.channel_token(channel)` | `String` | A raw token, for a client that is not a rendered page. |
| `native_channel(channel)` | `String` | The meta tag, for a view. The usual way in. |

Payload keys the shells understand: `title`, `body`, `url` (opened on click), `tag` (a stable id, so
an update replaces its predecessor rather than stacking), `icon`.

Channel names are yours to choose — `user:42`, `room:7`, `deploy:prod`. They may not contain `.`,
`|` or control characters, and are namespaced internally so they can never collide with, or be
reached through, an app's own `sse_broadcast` topics.

## Capabilities

A shell declares what it can do, and the page can branch on it without sniffing user agents:

```js
window.soli.nativeBridge   // { available: true, platform: "android", capabilities: ["notify", "camera"] }
```

Two rules explain the shape of this table:

1. **Prefer the web API when the host already has one.** `getUserMedia` needs no bridge call — it
   needs the shell to stop denying it, which is permission wiring rather than an API.
2. **Only embedded web views need a bridge at all.** On Windows and Linux a `soli desktop build`
   artifact opens the user's *real browser* (chrome-less, but Chrome), so every web API is already
   there — notifications, push, camera, geolocation. There is nothing to bridge until you replace
   that browser with a native window.

| Capability | Browser / PWA | Windows | Linux | macOS shell | Android shell |
|---|---|---|---|---|---|
| Notifications, app open | ✅ | ✅ browser | ✅ browser | ✅ shipped | ✅ shipped |
| Notifications, app closed | ✅ web push | ✅ web push | ✅ web push | ✅ [APNs](#reaching-a-closed-app-apns) | ✅ [FCM](#reaching-a-closed-android-app-fcm) sender¹ |
| Camera / microphone | ✅ | ✅ browser | ✅ browser | ✅ shipped | ✅ shipped |
| File upload / capture | ✅ | ✅ | ✅ | ✅ | ✅ shipped |
| Clipboard | ✅ | ✅ | ✅ | ✅ | ✅ |
| Geolocation | ✅ | ✅ browser | ✅ browser | 🔜 bridge | 🔜 bridge |
| Vibration / haptics | ✅ Android | ✗ | ✗ | 🔜 bridge | 🔜 needs `VIBRATE` |
| Deep links into the app | — | 🔜 | 🔜 | 🔜 | ✅ shipped |
| NFC | Chrome Android only | ✗ | ✗ | ✗ no hardware API | 🔜 bridge |
| [Barcode / QR scan](/docs/native/scanning) | ✅ native | ✅ native | ✅ native | ✅ decoder needed | ✅ native |
| Biometric unlock | ✅ WebAuthn | ✅ browser | ✅ browser | 🔜 bridge | 🔜 bridge |
| Badge count | ✅ Badging API | 🔜 | 🔜 | 🔜 bridge | 🔜 bridge |
| Share sheet | ✅ Web Share | ✅ browser | 🔜 | 🔜 bridge | 🔜 bridge |
| Keep screen awake | ✅ Wake Lock | ✅ browser | ✅ browser | 🔜 bridge | 🔜 bridge |
| Printing | ✅ | ✅ | ✅ | ✅ | 🔜 bridge |

✅ shipped · 🔜 planned · ✗ not possible on the platform · "browser" = provided by the browser the
artifact opens, not by a shell

¹ The **sender** ships; the Android app still needs the Firebase SDK to obtain a device token, which
means a Gradle build (see [What it costs](#what-it-costs-1)).

### Where the shells stand

| Host | Window today | Native shell |
|---|---|---|
| **Windows** | the user's browser, chrome-less | none — would be **WebView2** |
| **Linux** | the user's browser, chrome-less | none — would be **WebKitGTK** |
| **macOS** | native, frameless | ✅ AppKit + `WKWebView` |
| **Android** | native | ✅ `WebView` |
| **iOS** | — | none — would be UIKit + `WKWebView` |

A Windows or Linux shell is a real option, not a missing feature: it buys a native frameless window
and an icon, and costs you the web APIs the browser was providing for free. Both embed a web view
that suppresses notifications by default — WebView2 raises `NotificationReceived` for the host to
handle, WebKitGTK emits `show-notification` — so both would implement the same bridge contract the
macOS and Android shells do, and both would need their own permission wiring for camera.

**"macOS shell" means AppKit, not Apple in general.** There is no iOS shell yet. `WKWebView`,
`WKScriptMessageHandler`, `UNUserNotificationCenter` and the capture-permission delegate are shared
between the platforms, so the bridge code ports directly — but the window layer (`NSWindow`, menus,
the frameless chrome) is AppKit and has no UIKit equivalent. APNs, by contrast, is already
platform-neutral: the same `Apns.send` reaches both once a device registers.

Rows marked 🔜 are not implemented yet. A capability only appears in a shell's `capabilities` list
once it actually works there, so feature-detection stays honest:

```js
if (window.soli.nativeBridge.capabilities.includes("camera")) {
  // safe to offer the in-app scanner
}
```

## Writing a shell

A shell injects an object the client script looks for. WebKit can define it at document start:

```swift
window.soli = window.soli || {};
window.soli.native = {
  platform: "macos",
  capabilities: ["notify"],
  notify: function (json) { window.webkit.messageHandlers.soliNative.postMessage(json); }
};
```

Android binds a Java object by name instead, which the script also accepts — `addJavascriptInterface`
is how the platform injects, and evaluating a wrapper script early enough to dress it up races page
load:

```java
webView.addJavascriptInterface(new SoliNativeBridge(), "soliNativeHost");
```

Either way `notify` receives one JSON string. Working examples of both live in the Bonfire clients
(`clients/macos`, `clients/android`).

Pages can branch on what the host supports without sniffing user agents:

```js
window.soli.nativeBridge   // { available: true, platform: "android", capabilities: ["notify"] }
```

## Connection behaviour

The client subscribes over SSE and reconnects with exponential backoff, up to 30 seconds. It drops
the connection while the tab is hidden — an idle backgrounded stream costs the server a task for
nothing — and reconnects when it comes back. Thousands of idle subscribers cost async tasks, not
worker threads.

## Requirements

- `SOLI_SESSION_SECRET`, 32+ characters. Without it `native_channel` raises rather than emitting an
  unsigned tag.
- Nothing else. No push service, no keys, no certificates.
