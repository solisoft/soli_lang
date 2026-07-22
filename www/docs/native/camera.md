# Camera & Microphone

`getUserMedia` inside a packaged app — which works only once the shell stops denying it.

This is the clearest example of the bridge's first rule: **prefer the web API where the host has
one**. There is no `Native.camera(...)` call, because the web already has `getUserMedia`. What a
shell has to do is grant it.

## The failure this fixes

A web view denies capture **silently**. The page sees a rejected promise, the user sees no prompt,
and nothing appears in any log:

```js
navigator.mediaDevices.getUserMedia({ video: true })
  .catch(err => console.log(err.name))   // "NotAllowedError", with no prompt ever shown
```

That is why camera code that works in a browser "just doesn't work" in a wrapper. It is not the
page's fault and not a bug in `getUserMedia`.

## Using it

Nothing soli-specific — it is ordinary web code:

```js
async function startScanner(video) {
  if (!window.soli?.nativeBridge?.capabilities.includes("camera")) {
    return showFallback()   // a file input with capture=environment still works everywhere
  }

  const stream = await navigator.mediaDevices.getUserMedia({
    video: { facingMode: "environment" },   // rear camera on a phone
    audio: false
  })
  video.srcObject = stream
  await video.play()
  return stream
}

// Release the camera when you are done — the indicator light stays on otherwise.
function stop(stream) {
  stream.getTracks().forEach(track => track.stop())
}
```

Feature-detect through `capabilities` rather than the user agent: a capability only appears there
once it actually works in that shell.

## The `camera_preview` helper

Plain HTML is a perfectly good answer, and the helper exists only for what it leaves out:

```erb
<%- camera_preview({"facing": "environment", "class": "rounded-xl", "fallback": "#no-camera"}) %>
```

```html
<video data-soli-camera autoplay playsinline muted class="rounded-xl"
       data-facing="environment" data-fallback="#no-camera"></video>
```

| Option | |
|---|---|
| `facing` | `"user"` (default) or `"environment"` for the rear camera. |
| `width` / `height` | Requested as `ideal`, so an unsupported size picks the nearest mode rather than failing. |
| `audio` | `true` to capture the microphone too. |
| `scan` | Barcode formats — see [Scanning](/docs/native/scanning). |
| `fallback` | A selector revealed when the camera fails. |
| `class` / `id` | Passed through to the element. |
| `manual` | Render the element but do not start the stream; call `soli.camera.start(el)` yourself. |

What it buys over the six lines above:

- **The tracks are stopped when the element leaves the DOM.** Instant navigation swaps the body
  without a page unload, so a hand-rolled preview keeps its stream and the camera indicator stays
  lit after the user has moved on. This is the one that actually bites.
- `playsinline muted` always — without them iOS goes fullscreen and refuses to autoplay.
- Constraints requested as `ideal` rather than `exact`.
- `soli.camera.snapshot(video)` un-mirrors front-camera frames, so text in shot is not backwards.
- Events rather than callbacks: `soli:camera-ready`, `soli:camera-error` (with the `NotAllowedError` /
  `NotFoundError` / `NotReadableError` name), and `data-camera-state` on the element for CSS.

The script is injected **only** into pages that render such an element, so a page with no camera
downloads nothing.

## Without a live stream

If all you need is a photo, a file input costs nothing and works on every platform, including
browsers with no bridge at all:

```html
<input type="file" accept="image/*" capture="environment">
```

On Android that opens the camera app directly. The shell already handles the file chooser; a bare
WebView ignores every file input, which is its own silent failure.

## What each shell does

| | Grant | Also required |
|---|---|---|
| **macOS** | `requestMediaCapturePermissionFor` in the `WKUIDelegate` | `NSCameraUsageDescription` / `NSMicrophoneUsageDescription` in `Info.plist` |
| **Android** | `onPermissionRequest` in the `WebChromeClient` | `CAMERA` / `RECORD_AUDIO` permissions, granted at runtime from API 23 |
| **Windows / Linux** | nothing — the artifact opens the real browser | — |

Two platform details worth knowing, because both fail in confusing ways:

- **macOS terminates the process** if a usage description is missing. Not a denial, not an exception
  — the app disappears the moment the page calls `getUserMedia`. The string you write is what the
  user reads in the permission dialog, so it should explain the why.
- **Android has two gates**: the app's own runtime permission and the per-origin one the web view
  asks about. The shell holds the page's request while Android asks the user, then replays it. A
  `PermissionRequest` that is never answered leaves `getUserMedia` hanging forever rather than
  rejecting, so denial is answered explicitly too.

Reference implementations: `clients/macos/main.swift` and
`clients/android/src/net/solisoft/bonfire/MainActivity.java`.

## Secure contexts

`getUserMedia` requires one. HTTPS qualifies, and so does `http://127.0.0.1` — which is what a
`soli desktop build` app serves on, so a packaged desktop app is fine without a certificate.
