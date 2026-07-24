// Soli motion sensors — client half.
//
// Injected only into pages that opt in (a `soli-sensors` meta from
// `motion_sensors()`, or any inline use of `soli.sensors`), so a page that
// wants nothing pays nothing.
//
// Reading a motion sensor is a `devicemotion`/`deviceorientation` listener —
// a handful of lines that work in mobile Safari, Chrome, and both WebView
// shells. This exists for the parts hand-written code reliably forgets:
//
//   1. the iOS 13+ permission gate: DeviceMotionEvent.requestPermission() must
//      be called from a user gesture, and rejects otherwise;
//   2. stopping the listener — instant navigation swaps the body without a page
//      unload, so a subscription started by a page that has since been replaced
//      keeps the sensor (and the battery drain) running;
//   3. one shared listener per event fanned out to every subscriber, rather
//      than N listeners re-parsing the same event;
//   4. a normalized reading in each sensor's conventional unit.
//
// Exposes `window.soli.sensors`:
//   gyroscope(cb, opts)     -> Promise<{stop()}>   x/y/z in rad/s
//   accelerometer(cb, opts) -> Promise<{stop()}>   x/y/z in m/s^2
//   orientation(cb, opts)   -> Promise<{stop()}>   alpha/beta/gamma in degrees
//   supported(kind)         -> Bool
//   requestPermission()     -> Promise<{motion, orientation}>  (call from a gesture)
//   stopAll()
//
// opts: { frequency: Hz } or { interval: ms } to throttle; accelerometer also
// takes { includeGravity: true } (default false — gravity removed where the
// device can).
(function () {
  "use strict";

  if (window.__soliSensorsStarted) return;
  window.__soliSensorsStarted = true;

  var DEG2RAD = Math.PI / 180;

  function nowMs() {
    return new Date().getTime();
  }

  function sensorError(name, message) {
    var error = new Error(message);
    error.name = name;
    return error;
  }

  // Normalize throttle options to a minimum inter-callback interval in ms.
  function normalizeOpts(opts) {
    opts = opts || {};
    var minInterval = 0;
    if (opts.frequency && opts.frequency > 0) {
      minInterval = 1000 / opts.frequency;
    } else if (opts.interval && opts.interval > 0) {
      minInterval = opts.interval;
    }
    return { minInterval: minInterval, includeGravity: !!opts.includeGravity };
  }

  // ---------------------------------------------------------------------
  // Shared `devicemotion` fan-out (gyroscope + accelerometer)
  // ---------------------------------------------------------------------

  var motionSubs = [];
  var motionListening = false;

  function onDeviceMotion(event) {
    var now = nowMs();
    // Snapshot: a callback may stop() itself, which splices the live array.
    var subs = motionSubs.slice();
    for (var i = 0; i < subs.length; i++) {
      var sub = subs[i];
      if (sub.opts.minInterval && now - sub.last < sub.opts.minInterval) continue;

      var reading = null;
      if (sub.kind === "gyroscope") {
        var rate = event.rotationRate;
        if (!rate) continue; // no gyroscope on this device
        reading = {
          x: (rate.beta || 0) * DEG2RAD,
          y: (rate.gamma || 0) * DEG2RAD,
          z: (rate.alpha || 0) * DEG2RAD,
          interval: event.interval || 0
        };
      } else {
        // gravity-free acceleration by default; fall back to the with-gravity
        // reading on devices that only report that one.
        var accel = sub.opts.includeGravity
          ? event.accelerationIncludingGravity
          : event.acceleration || event.accelerationIncludingGravity;
        if (!accel) continue;
        reading = {
          x: accel.x || 0,
          y: accel.y || 0,
          z: accel.z || 0,
          interval: event.interval || 0
        };
      }

      sub.last = now;
      try {
        sub.cb(reading);
      } catch (e) {
        /* a throwing callback must not take down the other subscribers */
      }
    }
  }

  function startMotion() {
    if (!motionListening) {
      window.addEventListener("devicemotion", onDeviceMotion, { passive: true });
      motionListening = true;
    }
  }

  function stopMotionIfIdle() {
    if (motionListening && motionSubs.length === 0) {
      window.removeEventListener("devicemotion", onDeviceMotion);
      motionListening = false;
    }
  }

  // ---------------------------------------------------------------------
  // Shared `deviceorientation` fan-out
  // ---------------------------------------------------------------------

  var orientSubs = [];
  var orientListening = false;

  function onDeviceOrientation(event) {
    var now = nowMs();
    var subs = orientSubs.slice();
    for (var i = 0; i < subs.length; i++) {
      var sub = subs[i];
      if (sub.opts.minInterval && now - sub.last < sub.opts.minInterval) continue;
      sub.last = now;
      try {
        sub.cb({
          alpha: event.alpha,
          beta: event.beta,
          gamma: event.gamma,
          absolute: !!event.absolute
        });
      } catch (e) {
        /* isolate a throwing callback */
      }
    }
  }

  function startOrient() {
    if (!orientListening) {
      window.addEventListener("deviceorientation", onDeviceOrientation, { passive: true });
      orientListening = true;
    }
  }

  function stopOrientIfIdle() {
    if (orientListening && orientSubs.length === 0) {
      window.removeEventListener("deviceorientation", onDeviceOrientation);
      orientListening = false;
    }
  }

  // ---------------------------------------------------------------------
  // Permission (iOS 13+ gates motion behind a per-gesture prompt)
  // ---------------------------------------------------------------------

  function requestPermission(ctor) {
    if (ctor && typeof ctor.requestPermission === "function") {
      // Must be invoked from a user gesture on iOS; rejects otherwise.
      return ctor.requestPermission();
    }
    return Promise.resolve("granted");
  }

  // ---------------------------------------------------------------------
  // Subscribe
  // ---------------------------------------------------------------------

  function subscribe(kind, cb, userOpts) {
    if (typeof cb !== "function") {
      return Promise.reject(sensorError("TypeError", "soli.sensors." + kind + "() needs a callback"));
    }
    var isOrientation = kind === "orientation";
    var ctor = isOrientation ? window.DeviceOrientationEvent : window.DeviceMotionEvent;
    if (typeof ctor === "undefined") {
      return Promise.reject(sensorError("NotSupportedError", kind + " is not available in this browser"));
    }

    return requestPermission(ctor).then(function (state) {
      if (state && state !== "granted") {
        throw sensorError("NotAllowedError", "permission for " + kind + " was " + state);
      }
      var sub = { kind: kind, cb: cb, opts: normalizeOpts(userOpts), last: 0 };
      if (isOrientation) {
        orientSubs.push(sub);
        startOrient();
      } else {
        motionSubs.push(sub);
        startMotion();
      }

      var stopped = false;
      return {
        kind: kind,
        stop: function () {
          if (stopped) return;
          stopped = true;
          var arr = isOrientation ? orientSubs : motionSubs;
          var idx = arr.indexOf(sub);
          if (idx !== -1) arr.splice(idx, 1);
          if (isOrientation) stopOrientIfIdle();
          else stopMotionIfIdle();
        }
      };
    });
  }

  function stopAll() {
    motionSubs.length = 0;
    orientSubs.length = 0;
    stopMotionIfIdle();
    stopOrientIfIdle();
  }

  // Instant navigation swaps the body without a page unload; drop every
  // subscription so a sensor started on the old page does not outlive it.
  document.addEventListener("soli:visit", stopAll);
  window.addEventListener("pagehide", stopAll);

  // ---------------------------------------------------------------------
  // Public API
  // ---------------------------------------------------------------------

  window.soli = window.soli || {};
  window.soli.sensors = {
    /// Angular velocity, x/y/z in rad/s. Returns a Promise<{stop()}>.
    gyroscope: function (cb, opts) {
      return subscribe("gyroscope", cb, opts);
    },

    /// Linear acceleration, x/y/z in m/s^2 (gravity removed unless
    /// { includeGravity: true }). Returns a Promise<{stop()}>.
    accelerometer: function (cb, opts) {
      return subscribe("accelerometer", cb, opts);
    },

    /// Device attitude — alpha (0..360), beta (-180..180), gamma (-90..90),
    /// in degrees, plus `absolute`. Returns a Promise<{stop()}>.
    orientation: function (cb, opts) {
      return subscribe("orientation", cb, opts);
    },

    /// Whether the underlying event exists at all. It cannot tell whether the
    /// hardware is present — a desktop shell reports true but never emits.
    supported: function (kind) {
      if (kind === "orientation") return typeof window.DeviceOrientationEvent !== "undefined";
      return typeof window.DeviceMotionEvent !== "undefined";
    },

    /// Ask for motion + orientation permission in one gesture (iOS). Call from
    /// a click; resolves { motion, orientation } each "granted"/"denied".
    requestPermission: function () {
      return Promise.all([
        requestPermission(window.DeviceMotionEvent).catch(function () {
          return "denied";
        }),
        requestPermission(window.DeviceOrientationEvent).catch(function () {
          return "denied";
        })
      ]).then(function (states) {
        return { motion: states[0] || "granted", orientation: states[1] || "granted" };
      });
    },

    /// Stop every active subscription.
    stopAll: stopAll
  };
})();
