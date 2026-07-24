# Motion Sensors

The gyroscope, accelerometer and device orientation — the three readings a phone
gives you about how it is being held and moved. All three ride the standard web
`DeviceMotionEvent` / `DeviceOrientationEvent`, which fire in mobile Safari,
Chrome, and both WebView shells, so this is a thin client helper rather than a
native bridge.

What it adds over a raw `addEventListener` is the part hand-written code reliably
forgets:

1. the **iOS 13+ permission gesture** — `DeviceMotionEvent.requestPermission()`
   must be called from a user gesture, and rejects otherwise;
2. **stopping the listener** — instant navigation swaps the body without a page
   unload, so a subscription from a page that has been replaced keeps the sensor
   (and the battery drain) running;
3. one **shared listener per event** fanned out to every subscriber;
4. a **normalized reading** in each sensor's conventional unit.

## Enabling it

Referencing `soli.sensors` in an inline `<script>` is enough — the server sees
it and injects the client. When your sensor code lives in an **external** `.js`
file the server can't see, call `motion_sensors()` once (it emits the marker that
turns injection on):

```erb
<%- motion_sensors() %>   <%# only needed for external-JS sensor code %>
```

A page that references neither downloads nothing.

## The API

`window.soli.sensors` exposes three subscriptions. Each returns a
`Promise<{ stop() }>` — a Promise because permission may have to be asked first.

```javascript
// Angular velocity — x/y/z in rad/s
const gyro = await soli.sensors.gyroscope(r => {
  console.log(r.x, r.y, r.z, r.interval)
})

// Linear acceleration — x/y/z in m/s² (gravity removed by default)
const accel = await soli.sensors.accelerometer(r => {
  console.log(r.x, r.y, r.z)
}, { includeGravity: false })

// Device attitude — degrees
const tilt = await soli.sensors.orientation(r => {
  console.log(r.alpha, r.beta, r.gamma, r.absolute)
})

// Later — always stop what you started
gyro.stop()
accel.stop()
tilt.stop()
```

| Method | Reading | Unit |
|--------|---------|------|
| `gyroscope(cb, opts)` | `{ x, y, z, interval }` — angular velocity | rad/s |
| `accelerometer(cb, opts)` | `{ x, y, z, interval }` — linear acceleration | m/s² |
| `orientation(cb, opts)` | `{ alpha, beta, gamma, absolute }` — attitude | degrees |
| `supported(kind)` | whether the event exists | — |
| `requestPermission()` | ask motion + orientation in one gesture | — |
| `stopAll()` | stop every active subscription | — |

Orientation axes follow the web convention: `alpha` is the compass rotation
(0–360, `absolute: true` when it is true north), `beta` is front-to-back tilt
(−180 to 180), `gamma` is left-to-right tilt (−90 to 90).

### Options

- **`frequency`** (Hz) or **`interval`** (ms) — throttle the callback. Sensors
  can fire 60+ times a second; a level indicator or a compass needs far less, and
  throttling saves the battery. `{ frequency: 10 }` caps it at ten readings a
  second.
- **`includeGravity`** (accelerometer only) — by default the reading has gravity
  removed (`DeviceMotionEvent.acceleration`). Set `true` for the raw
  with-gravity value. On a device that only reports the with-gravity reading, the
  gravity-free request falls back to it.

## iOS needs a gesture

On iOS 13+, motion and orientation are gated behind a permission prompt that
**only appears if you ask from a user gesture**. Call from a click, not on load:

```html
<button id="enable">Enable motion</button>
<script>
  document.getElementById("enable").addEventListener("click", async () => {
    // one gesture grants both
    const state = await soli.sensors.requestPermission()   // {motion, orientation}
    if (state.motion === "granted") {
      soli.sensors.gyroscope(r => level(r))
    }
  })
</script>
```

Calling `gyroscope()` directly works too — it requests permission itself — but
only if that call is inside the gesture. Outside one, the promise rejects with a
`NotAllowedError`. Android and desktop browsers have no such prompt.

## Per-platform reality

| Platform | Behaviour |
|----------|-----------|
| **Mobile Safari / iOS shell** | Works, after the per-gesture `requestPermission()` prompt. |
| **Chrome Android / Android shell** | Works with no prompt. |
| **Desktop browsers, macOS / Linux shells** | The events exist, so `supported()` is `true`, but a machine with no gyroscope never emits — your callback simply never fires. Design for readings that may not come. |

`supported(kind)` reports whether the API exists, which is not the same as
whether the hardware is present — the honest signal is that a callback never
arrives, so don't block your UI waiting for the first reading.

## Cleanup is automatic across instant navigation

You must call `.stop()` when a subscription is no longer needed. The one case the
helper handles for you is **instant navigation** — on the `soli:visit` event
(and on `pagehide`) every active subscription is stopped, because the body swap
that instant-nav performs would otherwise leave a listener from the previous page
running against a gyroscope nobody is watching. Re-subscribe in `soli:load` if the
next page needs the sensor.
