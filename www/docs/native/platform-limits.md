# Platform limits & vertical stubs

What the native bridge deliberately does **not** pretend to own.

The bridge prefers the web API when the host has one, and only crosses into the
shell when the embedded web view cannot. Some product asks — continuous
background location, store billing, home-screen widgets — sit outside that
contract. The client helpers exist so feature-detection stays honest.

## Background location

```js
if (soli.nativeBridge.supports("background_location")) {
  await soli.nativeBridge.startBackgroundLocation({ distanceFilter: 25 })
  // shell posts updates to your app (e.g. POST /locations)
} else {
  // fall back to navigator.geolocation while the page is open
}
```

| Host | Status |
|------|--------|
| Browser / PWA | Foreground `navigator.geolocation` only |
| Generated shells (default) | **Not** in `capabilities` — helpers reject with `NotSupportedError` |
| Custom shell | Declare `background_location` and implement `background_location_start` / `_stop` |

iOS “Always” location and Android foreground services need entitlements, privacy
strings, and store review narratives that no WebView-first stack can invent for
you. Wire them in the shell when the product requires it.

## In-app purchases

```js
try {
  await soli.nativeBridge.purchase("sku.premium.monthly")
} catch (err) {
  if (err.name === "NotSupportedError") {
    // open a web checkout, or hide the button
  }
}
```

Soli does **not** ship StoreKit or Play Billing. A shell may list `iap` and
handle `iap_purchase`; until then `purchase(...)` rejects. Server-side receipt
verification stays your domain.

## Widgets / Live Activities / App Clips

Not part of the WebView thesis. There is no `soli.nativeBridge.widget` API.
Ship a native extension next to the generated shell if you need a home-screen
glance; keep product data on the Soli server.

## What *is* first-class

| Area | Entry |
|------|--------|
| Open/closed notifications | [Notifications](/docs/native/notifications) |
| Device token store | [Devices](/docs/native/devices) |
| Shell generation | [Clients](/docs/native/clients) |
| Deep links | [Deep Links](/docs/native/deep-links) |
| Offline outbox | [Offline mobile](/docs/native/offline) |
| Desktop local product | [Desktop](/docs/development-tools/desktop) |
| Product narrative | [Native mobile blog](/docs/blog/native-mobile) |

## Related

- [Device Capabilities](/docs/native/device)
- [Native Bridge](/docs/development-tools/native-bridge) — hub for all native pages
