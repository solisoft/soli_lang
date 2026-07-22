// Soli native bridge — client half.
//
// Injected only into pages that called `native_channel(...)`, so a page that
// wants nothing from the shell pays nothing.
//
// What it does:
//   1. subscribes to the page's signed channel over SSE
//   2. routes each event to the native shell if one injected `window.soli.native`,
//      otherwise to a Web Notification, otherwise drops it
//   3. inside a shell, replaces `window.Notification` so page code that already
//      calls `new Notification(...)` keeps working — a WebView has no such
//      global at all, so without this that code throws.
(function () {
  "use strict";

  if (window.__soliNativeStarted) return;
  window.__soliNativeStarted = true;

  var meta = document.querySelector('meta[name="soli-native"]');
  if (!meta || !meta.content) return;
  var token = meta.content;

  // Two shapes, because the two platforms inject differently. WKWebView can
  // define a real object at document start (WKUserScript); Android's
  // `addJavascriptInterface` binds a Java object under a global name instead,
  // and evaluating JS early enough to wrap it races page load. So accept
  // either, rather than making the Android shell fight its own API.
  var bridge = null;
  if (window.soli && window.soli.native && typeof window.soli.native.notify === "function") {
    bridge = window.soli.native;
  } else if (window.soliNativeHost && typeof window.soliNativeHost.notify === "function") {
    var host = window.soliNativeHost;
    // A Java interface can only hand back primitives, so the capability list
    // crosses as a comma-separated string.
    var declared = typeof host.capabilities === "function" ? host.capabilities() : "notify";
    bridge = {
      platform: typeof host.platform === "function" ? host.platform() : "android",
      capabilities: String(declared || "notify").split(",").filter(Boolean),
      notify: function (json) {
        host.notify(json);
      }
    };
  }

  // ---------------------------------------------------------------------
  // Delivery
  // ---------------------------------------------------------------------

  function deliver(payload) {
    if (!payload || typeof payload !== "object") return;

    if (bridge && typeof bridge.notify === "function") {
      try {
        bridge.notify(JSON.stringify(payload));
        return;
      } catch (e) {
        // Fall through: a shell that fails is not a reason to lose the
        // notification when the page can still show one itself.
      }
    }

    if (typeof window.Notification !== "function") return;
    if (window.Notification.permission === "granted") {
      show(payload);
    } else if (window.Notification.permission !== "denied") {
      // Never prompt on an incoming event — a permission dialog the user did
      // not ask for is worse than a missed notification. Pages that want the
      // browser path call Notification.requestPermission() from a gesture.
      return;
    }
  }

  function show(payload) {
    try {
      var note = new window.Notification(payload.title || "", {
        body: payload.body || "",
        icon: payload.icon || undefined,
        tag: payload.tag || undefined
      });
      if (payload.url) {
        note.onclick = function () {
          window.focus();
          window.location.href = payload.url;
        };
      }
    } catch (e) {
      /* A page in a context that forbids notifications: nothing to do. */
    }
  }

  // ---------------------------------------------------------------------
  // Notification polyfill (shell only)
  // ---------------------------------------------------------------------
  //
  // Deliberately only when a bridge exists. In a browser the real Notification
  // API is better than anything shimmed on top of it, and shadowing a standard
  // global for no gain would only confuse whoever debugs it next.
  if (bridge && typeof bridge.notify === "function") {
    var ShimNotification = function (title, options) {
      options = options || {};
      this.title = title;
      this.body = options.body;
      try {
        bridge.notify(
          JSON.stringify({
            title: title,
            body: options.body || "",
            icon: options.icon,
            tag: options.tag,
            url: options.data && options.data.url ? options.data.url : undefined
          })
        );
      } catch (e) {
        /* nothing sensible to do from a constructor */
      }
    };
    ShimNotification.permission = "granted";
    ShimNotification.requestPermission = function (cb) {
      // The shell owns the real OS permission; the page's own prompt would be
      // a second, meaningless one.
      if (typeof cb === "function") cb("granted");
      return Promise.resolve("granted");
    };
    ShimNotification.prototype.close = function () {};
    window.Notification = ShimNotification;
  }

  // ---------------------------------------------------------------------
  // Transport
  // ---------------------------------------------------------------------

  var source = null;
  var retry = 1000;
  var MAX_RETRY = 30000;

  function connect() {
    if (typeof window.EventSource !== "function") return;

    source = new EventSource("/__soli/native/stream?token=" + encodeURIComponent(token));

    source.addEventListener("soli-native", function (event) {
      var message;
      try {
        message = JSON.parse(event.data);
      } catch (e) {
        return;
      }
      if (message && message.type === "notify") deliver(message.payload);
    });

    source.onopen = function () {
      retry = 1000;
    };

    source.onerror = function () {
      // EventSource reconnects on its own, but not after the server closes the
      // stream deliberately (an expired token, a restart). Back off and rebuild
      // rather than spin.
      if (source) source.close();
      source = null;
      setTimeout(connect, retry);
      retry = Math.min(retry * 2, MAX_RETRY);
    };
  }

  // Drop the connection while hidden: a backgrounded tab holding an idle SSE
  // stream costs the server a task for nothing, and the shell shows
  // notifications natively anyway.
  document.addEventListener("visibilitychange", function () {
    if (document.hidden) {
      if (source) {
        source.close();
        source = null;
      }
    } else if (!source) {
      connect();
    }
  });

  window.addEventListener("pagehide", function () {
    if (source) source.close();
  });

  connect();

  // Expose what the page can rely on, so app code can branch without sniffing
  // user agents.
  window.soli = window.soli || {};
  window.soli.nativeBridge = {
    available: !!bridge,
    platform: bridge && bridge.platform ? bridge.platform : "web",
    capabilities: bridge && bridge.capabilities ? bridge.capabilities : ["notify"]
  };
})();
