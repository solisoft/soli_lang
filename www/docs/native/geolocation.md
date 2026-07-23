# Geolocation

Where the user is, and what to do with it once you know.

The browser half is one line and needs no framework:

```js
navigator.geolocation.getCurrentPosition(
  pos => console.log(pos.coords.latitude, pos.coords.longitude),
  err => console.log(err.code)      // 1 PERMISSION_DENIED · 2 POSITION_UNAVAILABLE · 3 TIMEOUT
)
```

What soli adds is the two ends that line does not cover: **shells have to permit it**, and
**everything after the coordinates is server-side arithmetic**.

## The silent failure

Like the camera, an unwired web view denies location with no prompt and nothing in the log. The
page's error callback fires with `PERMISSION_DENIED` for a permission the user was never asked
about. Both shells now handle it:

| | What it does |
|---|---|
| **Android** | `onGeolocationPermissionsShowPrompt` in the `WebChromeClient`, plus `ACCESS_FINE_LOCATION` / `ACCESS_COARSE_LOCATION` granted at runtime |
| **macOS** | `NSLocationWhenInUseUsageDescription` — WKWebView can only serve geolocation if the host app is itself authorised for Core Location |
| **Windows / Linux** | nothing — the artifact opens the real browser |

Two details, both of which produce a hung page if you get them wrong when writing your own shell:

- Android asks **per origin** (the web view) *and* **per app** (the OS). Both have to be satisfied,
  so the shell holds the page's request while Android asks the user, then replays it.
- An unanswered `GeolocationPermissions.Callback` leaves `getCurrentPosition` waiting **forever**
  rather than calling its error handler. Denial has to be answered explicitly.

Only the app's own origin is granted. A third-party frame asking for the user's position is refused
without a prompt.

## Server-side: `Geo`

```soli
metres = Geo.distance(48.8566, 2.3522, 51.5074, -0.1278)   # 343516.4 — Paris to London
box    = Geo.bounding_box(48.8566, 2.3522, 5000)           # 5 km around Paris
hash   = Geo.geohash(48.8566, 2.3522, 9)                   # "u09tvw0f6"
```

| Call | Returns | |
|---|---|---|
| `Geo.distance(lat1, lng1, lat2, lng2)` | `Float` | Great-circle metres (haversine). |
| `Geo.bearing(lat1, lng1, lat2, lng2)` | `Float` | Degrees clockwise from north. |
| `Geo.bounding_box(lat, lng, radius_m)` | `Hash` | `{min_lat, max_lat, min_lng, max_lng}`. |
| `Geo.geohash(lat, lng, precision?)` | `String` | Default precision 9 (~5 m). |
| `Geo.geohash_decode(hash)` | `Hash` | `{lat, lng, lat_error, lng_error}`. |

Coordinates outside ±90 / ±180 raise rather than wrap — a wrapped coordinate produces an answer that
looks plausible and is wrong.

## Finding what is nearby

The important part, because the obvious version does not scale. **Distance is a trigonometric
function of every row**, so a query that filters or sorts by it cannot use an index — that is a full
scan. Use the box as a cheap indexed pre-filter, then measure only what survives:

```soli
class Place < Model
  # Places within `radius_m`, nearest first.
  static def near(lat, lng, radius_m)
    box = Geo.bounding_box(lat, lng, radius_m)

    # Indexed, and cheap: four comparisons on two fields.
    candidates = Place.where(
      "lat >= @min_lat AND lat <= @max_lat AND lng >= @min_lng AND lng <= @max_lng",
      box
    )

    # Exact, and only over what the box already narrowed to. The box is a
    # square around a circle, so its corners are outside the radius.
    within = []
    for place in candidates
      metres = Geo.distance(lat, lng, place["lat"], place["lng"])
      within.push({ "place": place, "metres": metres }) if metres <= radius_m
    end

    within.sort_by(fn(entry) entry["metres"])
  end
end
```

Index `lat` and `lng` and this stays fast as the collection grows. Without the box it is `O(rows)`
trigonometry per request.

## Geohashes

A geohash turns a position into a string whose **prefix means proximity** — neighbours share a
prefix, so a `LIKE 'u09tv%'` finds a neighbourhood with no trigonometry at all, on an ordinary text
index.

| Precision | Cell size |
|---|---|
| 5 | ~5 km |
| 6 | ~1 km |
| 7 | ~150 m |
| 9 | ~5 m |

```soli
class Place < Model
  before_save fn() { this.geohash = Geo.geohash(this.lat, this.lng, 9) }

  static def in_neighbourhood(lat, lng)
    Place.where("geohash LIKE @prefix", { "prefix": "#{Geo.geohash(lat, lng, 6)}%" })
  end
end
```

One caveat worth knowing before relying on it: **cell edges**. Two points a metre apart can sit
either side of a boundary and share no prefix at all. For a "roughly here" lookup that is fine; for
correctness, use the bounding box.

## Watching a position

For a moving user, `watchPosition` streams updates — and **must be cleared**, or it keeps the GPS
awake and drains the battery:

```js
const watch = navigator.geolocation.watchPosition(
  pos => send(pos.coords),
  err => console.log(err.code),
  { enableHighAccuracy: true, maximumAge: 10000, timeout: 15000 }
)

// Instant navigation swaps the body without a page unload, so this is the hook
// that actually fires.
document.addEventListener("soli:visit", () => navigator.geolocation.clearWatch(watch), { once: true })
```

`enableHighAccuracy: true` turns on GPS rather than network positioning: markedly more accurate
outdoors, markedly more expensive. Leave it off unless you are drawing a track.

## Privacy

A position is personal data. Ask at the moment it is needed rather than on page load — a prompt the
user did not expect is usually denied, and a denial is remembered. `pos.coords.accuracy` (metres) is
worth storing alongside the coordinates: it is the difference between "in this building" and "in
this city", and without it you cannot tell them apart later.

## Related

- [Camera & Microphone](/docs/native/camera) — the same permission-wiring story
- [Native Bridge](/docs/development-tools/native-bridge) — the capability table
