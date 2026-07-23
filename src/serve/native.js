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
  // Request/response calls
  // ---------------------------------------------------------------------
  //
  // `notify` is fire-and-forget, but biometrics, NFC and the share sheet all
  // have to answer — the user accepts or cancels. Both platforms can only push
  // strings across, so a call carries an id and the shell replies by invoking
  // `window.__soliNativeReply` with it.
  //
  // Every pending call is rejected if the shell never answers: a promise that
  // hangs forever is worse than a rejection a page can handle.

  var pending = {};
  var nextCallId = 0;
  var CALL_TIMEOUT_MS = 120000; // biometrics and NFC wait on a human

  window.__soliNativeReply = function (payload) {
    var message = typeof payload === "string" ? JSON.parse(payload) : payload;
    var call = pending[message.id];
    if (!call) return;
    delete pending[message.id];
    clearTimeout(call.timer);
    if (message.ok) {
      call.resolve(message.value);
    } else {
      var error = new Error(message.error || "the native call failed");
      error.name = message.name || "NativeError";
      call.reject(error);
    }
  };

  function call(method, args) {
    return new Promise(function (resolve, reject) {
      if (!bridge || typeof bridge.call !== "function") {
        var error = new Error("no native host for '" + method + "'");
        error.name = "NotSupportedError";
        reject(error);
        return;
      }
      var id = ++nextCallId;
      pending[id] = {
        resolve: resolve,
        reject: reject,
        timer: setTimeout(function () {
          delete pending[id];
          var timeout = new Error("'" + method + "' did not answer");
          timeout.name = "TimeoutError";
          reject(timeout);
        }, CALL_TIMEOUT_MS)
      };
      bridge.call(JSON.stringify({ id: id, method: method, args: args || {} }));
    });
  }

  function supports(capability) {
    return !!bridge && (bridge.capabilities || []).indexOf(capability) !== -1;
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
    capabilities: bridge && bridge.capabilities ? bridge.capabilities : ["notify"],
    supports: supports,
    call: call,

    /// Buzz. Falls back to the Vibration API, which Android's WebView has and
    /// WebKit does not.
    vibrate: function (pattern) {
      if (supports("vibrate")) return call("vibrate", { pattern: [].concat(pattern || 200) });
      if (navigator.vibrate) return Promise.resolve(navigator.vibrate(pattern || 200));
      return Promise.resolve(false);
    },

    /// The OS share sheet. `navigator.share` where it exists, native otherwise.
    share: function (data) {
      if (navigator.share) return navigator.share(data);
      if (supports("share")) return call("share", data || {});
      return Promise.reject(Object.assign(new Error("sharing is unavailable"), {
        name: "NotSupportedError"
      }));
    },

    /// An unread count on the app icon.
    badge: function (count) {
      if (supports("badge")) return call("badge", { count: count || 0 });
      if (navigator.setAppBadge) {
        return count ? navigator.setAppBadge(count) : navigator.clearAppBadge();
      }
      return Promise.resolve(false);
    },

    /// Keep the screen on — a scanner or a recipe should not dim mid-task.
    keepAwake: function (on) {
      if (supports("keep_awake")) return call("keep_awake", { on: on !== false });
      return Promise.resolve(false);
    },

    /// Face ID / Touch ID / fingerprint. Resolves true only on a real success.
    authenticate: function (reason) {
      if (!supports("biometric")) {
        return Promise.reject(Object.assign(new Error("no biometric hardware exposed"), {
          name: "NotSupportedError"
        }));
      }
      return call("biometric", { reason: reason || "Confirm it is you" });
    },

    /// Read one NFC tag. Android only — WebKit has no API and macOS no hardware.
    readTag: function () {
      if (!supports("nfc")) {
        return Promise.reject(Object.assign(new Error("NFC is unavailable here"), {
          name: "NotSupportedError"
        }));
      }
      return call("nfc_read", {});
    },

    /// Print the current page through the OS print service.
    print: function () {
      if (supports("print")) return call("print", {});
      window.print();
      return Promise.resolve(true);
    }
  };
})();
