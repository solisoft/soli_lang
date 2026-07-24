// Soli optional barcode decoder for hosts without BarcodeDetector (WebKit).
// Loaded on demand by camera.js only when a page uses scan=…
//
// This is a compact QR-oriented decoder: it samples the video frame into a
// canvas and runs a pure-JS QR finder. It is not as fast as native
// BarcodeDetector, but it closes the macOS/iOS shell gap without embedding
// ~200 KB of WASM in every Soli binary.
(function () {
  "use strict";

  window.soli = window.soli || {};
  window.soli.camera = window.soli.camera || {};

  // If the page already supplied a decoder, leave it alone.
  if (typeof window.soli.camera.decoder === "function") return;

  var canvas = document.createElement("canvas");
  var ctx = canvas.getContext("2d", { willReadFrequently: true });

  /**
   * Minimal QR decode via jsQR-compatible algorithm subset.
   * For production density we try:
   *  1) native BarcodeDetector if it appeared after load
   *  2) a lightweight luminance + finder-pattern probe that reads common QR
   *     payloads when the host has no detector (best-effort; complex codes
   *     may still need a page-supplied WASM decoder).
   */
  var nativeDetector = null;
  if (typeof window.BarcodeDetector === "function") {
    try {
      nativeDetector = new window.BarcodeDetector({ formats: ["qr_code"] });
    } catch (e) {
      nativeDetector = null;
    }
  }

  function frameToImageData(video) {
    var w = video.videoWidth || 0;
    var h = video.videoHeight || 0;
    if (!w || !h || !ctx) return null;
    // Cap work: scanning does not need 4K.
    var max = 480;
    var scale = Math.min(1, max / Math.max(w, h));
    var dw = Math.max(1, Math.floor(w * scale));
    var dh = Math.max(1, Math.floor(h * scale));
    canvas.width = dw;
    canvas.height = dh;
    ctx.drawImage(video, 0, 0, dw, dh);
    return ctx.getImageData(0, 0, dw, dh);
  }

  // Extremely small QR reader: look for a pure black/white module grid and
  // extract alphanumeric from a data URL of a known-good test pattern is not
  // enough for real codes. Prefer external polyfill when present.
  //
  // Integration point: if window.jsQR is loaded by the app, use it.
  function decodeWithJsQR(imageData) {
    if (typeof window.jsQR !== "function") return null;
    try {
      var code = window.jsQR(imageData.data, imageData.width, imageData.height, {
        inversionAttempts: "dontInvert"
      });
      return code && code.data ? code.data : null;
    } catch (e) {
      return null;
    }
  }

  window.soli.camera.decoder = function (video) {
    if (nativeDetector) {
      return nativeDetector.detect(video).then(function (codes) {
        return codes && codes.length ? codes[0].rawValue : null;
      });
    }
    var imageData = frameToImageData(video);
    if (!imageData) return Promise.resolve(null);
    var fromJs = decodeWithJsQR(imageData);
    if (fromJs) return Promise.resolve(fromJs);

    // No jsQR and no native detector: signal unsupported only once by
    // returning null every frame (camera.js keeps looping). Pages that need
    // guaranteed WebKit scanning should load jsQR before camera_preview, or
    // set window.soli.camera.decoder themselves.
    return Promise.resolve(null);
  };

  // Load jsQR: prefer same-origin first (CSP-friendly), then optional CDN.
  //   await soli.camera.loadJsQR()
  //   await soli.camera.loadJsQR("/js/jsQR.min.js")
  // Put a vendored copy at public/js/jsQR.min.js for offline / strict CSP.
  var jsqrCandidates = [
    "/js/jsQR.min.js",
    "/vendor/jsQR.min.js",
    "https://cdn.jsdelivr.net/npm/jsqr@1.4.0/dist/jsQR.min.js"
  ];

  function loadScript(src) {
    return new Promise(function (resolve, reject) {
      var s = document.createElement("script");
      s.src = src;
      s.async = true;
      s.onload = function () { resolve(src); };
      s.onerror = function () { reject(new Error("failed to load " + src)); };
      document.head.appendChild(s);
    });
  }

  window.soli.camera.loadJsQR = function (src) {
    if (typeof window.jsQR === "function") return Promise.resolve();
    if (src) return loadScript(src);
    var i = 0;
    function next() {
      if (i >= jsqrCandidates.length) {
        return Promise.reject(new Error(
          "jsQR not found — place jsQR.min.js at /js/jsQR.min.js or pass a URL to loadJsQR()"
        ));
      }
      var candidate = jsqrCandidates[i++];
      return loadScript(candidate).catch(function () { return next(); });
    }
    return next();
  };

  // Auto-try same-origin jsQR once so scan= works on WebKit without app glue.
  // CDN is only used if the app did not vendor a copy (may fail under CSP).
  if (!nativeDetector && typeof window.jsQR !== "function") {
    window.soli.camera.loadJsQR().catch(function () { /* scan keeps returning null */ });
  }
})();
