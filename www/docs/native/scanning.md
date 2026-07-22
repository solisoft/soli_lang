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

`BarcodeDetector` is native in Chromium and absent from WebKit:

| Host | Live scanning |
|---|---|
| Android shell | ✅ native `BarcodeDetector` |
| Windows / Linux | ✅ native (the artifact opens Chrome) |
| Chromium browsers | ✅ native |
| **macOS shell** (`WKWebView`) | ❌ needs a decoder |
| **Safari / iOS** | ❌ needs a decoder |

Soli ships the **loop**, not the decoder. A WASM barcode reader is ~200 KB, and putting one in every
soli binary to serve the pages that scan would be the wrong trade. Where the platform has no
detector, supply one:

```js
import { readBarcodes } from "zxing-wasm"

window.soli.camera.decoder = async (video) => {
  const results = await readBarcodes(video, { formats: ["QRCode"] })
  return results.length ? results[0].text : null
}
```

Return the decoded string, or `null` for "nothing in this frame". The loop, throttling, lifecycle and
event are handled either way.

If neither a detector nor a decoder is available the element fires `soli:scan-unsupported`, so a page
can offer something else rather than showing a viewfinder that will never resolve:

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
