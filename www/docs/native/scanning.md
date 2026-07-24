# Barcode & QR Scanning

Read a code from the live camera feed, in a view.

```erb
<%- camera_preview({"facing": "environment", "scan": "qr_code"}) %>
```

```js
document.addEventListener("soli:scan", event => {
  console.log(event.detail.value)     // the decoded text
  console.log(event.detail.format)    // "qr_code", where the platform reports it
})
```

That is the whole integration. Scanning stops on the first hit unless you ask otherwise, and the
camera is released when the element goes away.

## Why not decode on the server

Because continuous scanning means ~10 frames a second. Uploading those is latency and bandwidth
nobody wants, and the round trip makes the viewfinder feel broken. Server-side decoding is the right
answer for **a photo the user captured** — a file input, one image, one request — not for a live
feed.

## Support, and the gap

`BarcodeDetector` is still **Chromium-only** — WebKit (Safari, iOS WKWebView, macOS
shell) does not implement it. Soli closes that gap for pages that use `scan=`:

| Host | How live scanning works |
|---|---|
| Android shell | ✅ native `BarcodeDetector` |
| Windows / Linux desktop | ✅ native (artifact opens Chrome) |
| Chromium browsers | ✅ native |
| **macOS shell** (`WKWebView`) | ✅ auto-loads Soli decoder + jsQR (not `BarcodeDetector`) |
| **Safari / iOS shell** | ✅ same WebKit path |

What “auto-loads” means (only when `scan=` is set and there is no native detector):

1. Inject `/__soli/barcode-decoder.js`.
2. That script tries **same-origin** jsQR first (`/js/jsQR.min.js`, then
   `/vendor/jsQR.min.js`), then a public CDN if CSP allows.
3. If the page already set `window.soli.camera.decoder`, that wins.

Chromium never downloads the optional script.

**Production / strict CSP:** vendor [jsQR](https://github.com/cozmo/jsQR) yourself — do not rely on the CDN:

```bash
# from your app root
mkdir -p public/js
curl -L -o public/js/jsQR.min.js \
  https://cdn.jsdelivr.net/npm/jsqr@1.4.0/dist/jsQR.min.js
```

```js
await soli.camera.loadJsQR()                   # search path above
await soli.camera.loadJsQR("/js/jsQR.min.js")  # explicit
# or set your own decoder:
window.soli.camera.decoder = async (video) => { /* string | null */ }
```

A full WASM reader (~200 KB) is deliberately **not** embedded in every Soli binary.

If neither a detector nor a working decoder is available, the element fires
`soli:scan-unsupported` (e.g. CSP blocked CDN and no vendored jsQR):

```js
video.addEventListener("soli:scan-unsupported", () => {
  document.querySelector("#upload-a-photo").hidden = false
})
```

## Options

| Option | |
|---|---|
| `scan` | Formats to look for: `"qr_code"`, or several — `"qr_code,ean_13,code_128"`. |
| `continuous` | Keep scanning after a hit. Default: stop on the first one. |
| `interval` | Milliseconds between frames. Default `100`. |
| `facing` | `"environment"` for the rear camera — almost always what you want for scanning. |
| `fallback` | A selector revealed when the camera fails or scanning is unsupported. |

**100 ms is deliberate.** A `requestAnimationFrame` loop decodes 60 times a second, drains a phone
battery and finds codes no faster: a code held in frame is still there 100 ms later.

## A complete example

Scanning a ticket at the door, posting each code as it is found:

```erb
<div class="scanner">
  <%- camera_preview({
    "facing":     "environment",
    "scan":       "qr_code",
    "continuous": true,
    "class":      "w-full rounded-xl",
    "fallback":   "#manual-entry"
  }) %>

  <form id="manual-entry" hidden action="/tickets/check" method="post">
    <%- csrf_field() %>
    <input name="code" placeholder="Type the ticket code">
    <button>Check in</button>
  </form>

  <ul id="checked-in"></ul>
</div>

<script>
  const seen = new Set()

  document.addEventListener("soli:scan", async (event) => {
    const code = event.detail.value
    if (seen.has(code)) return          // continuous mode re-reads the same code
    seen.add(code)

    const response = await fetch("/tickets/check", {
      method:  "POST",
      headers: { "Content-Type": "application/json",
                 "X-CSRF-Token": document.querySelector("meta[name=csrf-token]").content },
      body:    JSON.stringify({ code })
    })

    const ticket = await response.json()
    const item = document.createElement("li")
    item.textContent = ticket.valid ? `✅ ${ticket.holder}` : `❌ ${ticket.reason}`
    document.querySelector("#checked-in").prepend(item)
  })
</script>
```

The `seen` set matters in continuous mode: a code stays in frame for many hundred-millisecond ticks,
and without it one ticket checks in a dozen times.

## Capturing a still instead

If you want the photo rather than the code — a receipt, a document — take a frame and post it:

```js
const dataUrl = window.soli.camera.snapshot(video)   // JPEG data URL
```

Front-camera frames are un-mirrored on the way out, or text in the shot comes out backwards.

## Formats

Whatever the host detector supports. Chromium reports its list:

```js
const formats = await BarcodeDetector.getSupportedFormats()
// ["aztec", "code_128", "code_39", "data_matrix", "ean_13", "ean_8", "itf",
//  "pdf417", "qr_code", "upc_a", "upc_e", ...]
```

Ask only for what you need — a detector constrained to `qr_code` is faster than one trying every
format on every frame.

## Related

- [Camera & Microphone](/docs/native/camera) — the preview itself, and the permissions each shell needs
- [Native Bridge](/docs/development-tools/native-bridge) — the capability table
