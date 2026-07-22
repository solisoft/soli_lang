// Soli camera preview — the client half.
//
// Injected only into pages carrying a `[data-soli-camera]` element, so a page
// without one pays nothing.
//
// What it does: starts a stream into every such <video>, keeps it alive across
// instant-nav swaps, and — the part hand-written code usually forgets — stops
// the tracks when the element goes away, so the camera indicator does not stay
// lit after the user has navigated on.
(function () {
  "use strict";

  if (window.__soliCameraStarted) return;
  window.__soliCameraStarted = true;

  var active = new WeakMap();

  function constraintsFor(element) {
    var facing = element.getAttribute("data-facing") || "user";
    var video = { facingMode: facing };

    var width = parseInt(element.getAttribute("data-width"), 10);
    var height = parseInt(element.getAttribute("data-height"), 10);
    // `ideal`, not `exact`: an exact constraint that no camera satisfies fails
    // the whole request rather than picking the nearest mode.
    if (width) video.width = { ideal: width };
    if (height) video.height = { ideal: height };

    return { video: video, audio: element.hasAttribute("data-audio") };
  }

  function fail(element, error) {
    element.setAttribute("data-camera-state", "error");
    // The page decides what to show; this only reports. `NotAllowedError` means
    // denied, `NotFoundError` means no camera, `NotReadableError` means another
    // app holds it.
    element.dispatchEvent(
      new CustomEvent("soli:camera-error", {
        bubbles: true,
        detail: { name: error && error.name, message: error && error.message }
      })
    );

    var fallback = element.getAttribute("data-fallback");
    if (fallback) {
      var node = document.querySelector(fallback);
      if (node) node.hidden = false;
    }
  }

  function start(element) {
    if (active.has(element)) return;
    if (!navigator.mediaDevices || !navigator.mediaDevices.getUserMedia) {
      fail(element, { name: "NotSupportedError", message: "getUserMedia is unavailable" });
      return;
    }

    element.setAttribute("data-camera-state", "starting");
    navigator.mediaDevices
      .getUserMedia(constraintsFor(element))
      .then(function (stream) {
        // The element may have been swapped out while the user was answering
        // the permission prompt; do not leave an orphan stream running.
        if (!element.isConnected) {
          stop(stream);
          return;
        }
        active.set(element, stream);
        element.srcObject = stream;
        element.setAttribute("data-camera-state", "live");
        element.dispatchEvent(new CustomEvent("soli:camera-ready", { bubbles: true }));
        if (element.hasAttribute("data-soli-scan")) startScanning(element);
        var playing = element.play();
        if (playing && playing.catch) playing.catch(function () {});
      })
      .catch(function (error) {
        fail(element, error);
      });
  }

  function stop(streamOrElement) {
    var stream = streamOrElement;
    if (streamOrElement instanceof Element) {
      stopScanning(streamOrElement);
      stream = active.get(streamOrElement);
      active.delete(streamOrElement);
      streamOrElement.srcObject = null;
      streamOrElement.setAttribute("data-camera-state", "stopped");
    }
    if (!stream || !stream.getTracks) return;
    stream.getTracks().forEach(function (track) {
      track.stop();
    });
  }

  /// A still frame as a data URL, for upload or preview.
  function snapshot(element, type, quality) {
    if (!element || !element.videoWidth) return null;
    var canvas = document.createElement("canvas");
    canvas.width = element.videoWidth;
    canvas.height = element.videoHeight;
    var context = canvas.getContext("2d");
    // A front-facing preview is mirrored for the user's benefit; the capture
    // should not be, or text in frame comes out backwards.
    if (element.getAttribute("data-facing") === "user" && !element.hasAttribute("data-no-unmirror")) {
      context.translate(canvas.width, 0);
      context.scale(-1, 1);
    }
    context.drawImage(element, 0, 0);
    return canvas.toDataURL(type || "image/jpeg", quality || 0.9);
  }

  // -------------------------------------------------------------------
  // Barcode / QR scanning
  // -------------------------------------------------------------------
  //
  // `BarcodeDetector` is native in Chromium — the Android shell, and Chrome on
  // Windows/Linux — and absent from WebKit, so the macOS shell and Safari need
  // a decoder supplied by the page. Soli deliberately ships the loop, not the
  // decoder: a WASM barcode reader is ~200 KB, and putting it in every soli
  // binary to serve the pages that scan would be the wrong trade.
  //
  //   window.soli.camera.decoder = async (video) => { ... return "text" | null }
  //
  // The loop, the throttling and the lifecycle are the fiddly parts, and those
  // are here.

  var scanners = new WeakMap();

  function detectorFor(formats) {
    if (typeof window.BarcodeDetector !== "function") return null;
    try {
      return new window.BarcodeDetector({ formats: formats });
    } catch (e) {
      // Thrown when none of the requested formats is supported.
      return null;
    }
  }

  function startScanning(element) {
    if (scanners.has(element)) return;

    var formats = (element.getAttribute("data-soli-scan") || "qr_code")
      .split(",")
      .map(function (f) { return f.trim(); })
      .filter(Boolean);
    var interval = parseInt(element.getAttribute("data-scan-interval"), 10) || 100;
    var continuous = element.hasAttribute("data-scan-continuous");

    var detector = detectorFor(formats);
    if (!detector && typeof (window.soli.camera || {}).decoder !== "function") {
      element.dispatchEvent(
        new CustomEvent("soli:scan-unsupported", {
          bubbles: true,
          detail: { formats: formats }
        })
      );
      return;
    }

    var state = { stopped: false };
    scanners.set(element, state);

    function found(value, format) {
      element.dispatchEvent(
        new CustomEvent("soli:scan", {
          bubbles: true,
          detail: { value: value, format: format || null }
        })
      );
      if (!continuous) stopScanning(element);
    }

    function tick() {
      if (state.stopped || !element.isConnected) return;
      // A video that is not playing yet has no frame to read.
      if (element.readyState < 2) {
        setTimeout(tick, interval);
        return;
      }

      var attempt = detector
        ? detector.detect(element).then(function (codes) {
            return codes && codes.length ? { value: codes[0].rawValue, format: codes[0].format } : null;
          })
        : Promise.resolve(window.soli.camera.decoder(element)).then(function (value) {
            return value ? { value: value, format: null } : null;
          });

      attempt
        .then(function (hit) {
          if (state.stopped) return;
          if (hit) found(hit.value, hit.format);
          if (!state.stopped) setTimeout(tick, interval);
        })
        .catch(function () {
          // A single bad frame is not a reason to stop scanning.
          if (!state.stopped) setTimeout(tick, interval);
        });
    }

    tick();
  }

  function stopScanning(element) {
    var state = scanners.get(element);
    if (state) state.stopped = true;
    scanners.delete(element);
  }

  function scan(root) {
    (root || document).querySelectorAll("[data-soli-camera]").forEach(function (element) {
      if (element.hasAttribute("data-manual")) return;
      start(element);
    });
  }

  // Release cameras whose element left the DOM — an instant-nav body swap does
  // not fire unload, so nothing else would.
  var observer = new MutationObserver(function (mutations) {
    mutations.forEach(function (mutation) {
      mutation.removedNodes.forEach(function (node) {
        if (node.nodeType !== 1) return;
        if (node.matches && node.matches("[data-soli-camera]")) stop(node);
        if (node.querySelectorAll) {
          node.querySelectorAll("[data-soli-camera]").forEach(stop);
        }
      });
    });
  });

  function boot() {
    scan(document);
    observer.observe(document.documentElement, { childList: true, subtree: true });
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", boot);
  } else {
    boot();
  }

  // Instant navigation replaces the body without a page load.
  document.addEventListener("soli:load", function () {
    scan(document);
  });

  window.addEventListener("pagehide", function () {
    document.querySelectorAll("[data-soli-camera]").forEach(stop);
  });

  window.soli = window.soli || {};
  window.soli.camera = {
    start: start,
    stop: stop,
    snapshot: snapshot,
    startScanning: startScanning,
    stopScanning: stopScanning,
    /// Whether a code can be read without the page supplying a decoder.
    scanningIsNative: typeof window.BarcodeDetector === "function",
    /// Whether this host can offer a camera at all, for deciding what to render.
    available: !!(navigator.mediaDevices && navigator.mediaDevices.getUserMedia)
  };
})();
