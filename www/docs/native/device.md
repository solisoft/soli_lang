# Device Capabilities

Vibration, sharing, badges, biometrics, NFC, printing — the rest of what a shell can reach.

Each follows the same rule as the camera: **use the web API where the host has one**, and only cross
the bridge where it does not. One call per capability hides that difference:

```js
await soli.nativeBridge.vibrate(200)
await soli.nativeBridge.share({ title: "Bonfire", text: "Look at this", url: location.href })
await soli.nativeBridge.badge(3)
await soli.nativeBridge.keepAwake(true)
const ok  = await soli.nativeBridge.authenticate("Confirm the payment")
const tag = await soli.nativeBridge.readTag()
```

Feature-detect first — a capability is listed only where it actually works:

```js
if (soli.nativeBridge.supports("nfc")) { ... }
```

## What each does

| Call | macOS shell | Android shell | Browser fallback |
|---|---|---|---|
| `vibrate(ms \| pattern)` | trackpad haptic | `Vibrator` | `navigator.vibrate` |
| `share({title, text, url})` | `NSSharingServicePicker` | `ACTION_SEND` chooser | `navigator.share` |
| `badge(count)` | dock tile | notification badge | Badging API |
| `keepAwake(on)` | power assertion | `FLAG_KEEP_SCREEN_ON` | — |
| `authenticate(reason)` | Touch ID / password | `BiometricPrompt` | — (use WebAuthn) |
| `readTag()` | ✗ no hardware | NFC reader mode | — |
| `print()` | `NSPrintOperation` | `PrintManager` | `window.print()` |

Every call returns a promise. Ones that need a human — biometrics, NFC, sharing — resolve when they
answer and **reject when they cancel**, so a page never waits forever:

```js
try {
  await soli.nativeBridge.authenticate("Confirm the transfer")
  submit()
} catch (err) {
  if (err.name === "TimeoutError") retry()
  else showMessage(err.message)      // "cancelled", "NFC is switched off", …
}
```

## The honest limits

Four rows above are not oversights, and each fails for a different reason:

- **Android has no *arbitrary* badge counter.** iOS, macOS and the web let you set the icon to any
  number at any time; Android does not — a badge there is a byproduct of a notification. `badge(n)`
  posts a silent, minimum-importance carrier notification carrying `setNumber(n)`: launchers that
  render a count (Samsung and others) show the number, stock/Pixel shows a dot, and the carrier sits
  collapsed at the bottom of the shade. `badge(0)` cancels it. There is no honest way to badge the
  icon without a notification behind it.
- **Macs have no NFC radio.** Not a missing binding — the hardware is absent.
- **macOS haptics mean the trackpad**, and only Force Touch ones. A no-op elsewhere rather than an
  error, since a page asking to buzz should not have to care.
- **Biometrics do not authenticate anything to your server.** They confirm the person holding the
  device, locally. For an actual credential use WebAuthn — this is a lock on a UI action, not proof
  of identity.

## Badges, open and closed

A badge has two cases, and Android and Apple differ in both.

**While the app is open**, `badge(count)` sets it — the dock tile on macOS, `navigator.setAppBadge`
in a browser, the carrier notification on Android.

**While the app is closed**, the count has to ride the push, because nothing local is running to set
it:

```soli
# iOS / macOS — aps.badge sets the icon number directly
Apns.send(token, { "title": "3 new", "badge": 3 }, apns_options)

# Android — notification_count on the pushed notification
Fcm.send(token, { "title": "3 new", "badge": 3 }, fcm_options)

# or let Push.deliver route it to whichever the user has
Push.deliver("user:#{str(user.id)}", { "title": "3 new", "badge": 3 }, options)
```

`badge` is pulled out of the payload and placed correctly for each transport — `aps.badge` for APNs,
`android.notification.notification_count` for FCM — so the same call badges a closed app on either
platform. On the web, the count arrives as push data and your service worker calls
`navigator.setAppBadge` (that part is app-side, in `sw.js`).

## Vibration patterns

A number is a duration; an array alternates on and off, in milliseconds:

```js
soli.nativeBridge.vibrate(50)              // a tick
soli.nativeBridge.vibrate([0, 100, 50, 100])  // buzz, pause, buzz
```

WebKit has no Vibration API at all, so on macOS this is the bridge or nothing.

## NFC

Android only, and it reads the tag's id:

```js
const id = await soli.nativeBridge.readTag()   // "04a2b3c4d5e6f7"
```

The shell uses **reader mode**, not foreground dispatch: it does not bounce through a new intent,
and it suppresses the system's discovery sound and activity switch — which is what makes an in-app
scanner feel in-app rather than like the OS interrupting.

Web NFC exists only in Chrome for Android, never in a WebView, so there is no fallback to reach for.

## Printing

`print()` uses the OS print service and prints the current page:

```js
await soli.nativeBridge.print()
```

Android needs the bridge — a WebView ignores `window.print()` entirely. Everywhere else this is
`window.print()`, which is why the call exists at all rather than leaving pages to branch.

## Related

- [Camera & Microphone](/docs/native/camera) · [Scanning](/docs/native/scanning) · [Geolocation](/docs/native/geolocation)
- [Native Bridge](/docs/development-tools/native-bridge) — the capability table
