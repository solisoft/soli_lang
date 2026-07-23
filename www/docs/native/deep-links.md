# Deep Links

Open `https://your-app.com/pings/3` — or `yourapp://pings/3` — in the app instead of the browser.

A deep link has two halves that must agree: the **app** declares which URLs it handles, and the
**host** serves a file proving the app is allowed to. Get either wrong and the link silently falls
back to the browser, with no error anywhere — which is why this is fiddlier than it looks.

Soli owns the host half (`AppLinks`, which generates the proof files) and the shells own the app
half.

## Two kinds of link

| | Looks like | Verification | Works with |
|---|---|---|---|
| **Universal / App Link** | `https://app.com/pings/3` | host serves a file naming the app | any signing on Android; a **paid** Apple account on macOS/iOS |
| **Custom scheme** | `bonfire://pings/3` | none | any signing, both platforms |

The https link is the one worth having — it's an ordinary URL that opens the app when installed and
the site otherwise. The custom scheme needs no verification, so it works immediately (a QR code, an
email button) even before the https link is set up, or on an ad-hoc signed macOS build that can't
use the https form.

## The host half: `AppLinks`

The OS verifies an https link by fetching a file from your domain and checking your app is in it.
`AppLinks` generates that file — the part everyone gets wrong by hand:

```soli
# config/routes.sl
get("/.well-known/assetlinks.json",              "well_known#android")
get("/.well-known/apple-app-site-association",   "well_known#apple")
```

```soli
# app/controllers/well_known_controller.sl
def android(req)
  {
    "headers": { "Content-Type": "application/json" },
    "body": AppLinks.android("net.solisoft.bonfire", [ENV["ANDROID_CERT_SHA256"]])
  }
end

def apple(req)
  {
    "headers": { "Content-Type": "application/json" },
    "body": AppLinks.apple("ABCDE12345.net.solisoft.bonfire", ["/pings/*", "/threads/*"])
  }
end
```

Three details the OS is unforgiving about, all handled for you or worth knowing:

- **The Apple file has no `.json` extension** and must be served **as `application/json` with no
  redirect** — Apple's CDN fetches `/.well-known/apple-app-site-association` verbatim, and a 301 to a
  `.json` version fails verification.
- **The Android fingerprint** is your signing certificate's SHA-256, from `keytool -list -v -keystore
  …` or the Play Console. `AppLinks.android` accepts it as plain hex or colon-separated and normalizes
  to the upper-case colon form Google matches — a wrong-length one is rejected rather than silently
  never matching.
- **`AppLinks.apple`** takes `TEAMID.bundle.id` and emits both the modern `components` form and the
  legacy `paths`, so one file serves every OS version.

## The app half: what the shells declare

**Android** — the manifest declares both link types, and the app routes the incoming URL into the
web view (a deep link that only launched the app onto the home page would be pointless):

```xml
<intent-filter android:autoVerify="true">
  <action android:name="android.intent.action.VIEW" />
  <category android:name="android.intent.category.DEFAULT" />
  <category android:name="android.intent.category.BROWSABLE" />
  <data android:scheme="https" android:host="bonfire-app.pro" />
</intent-filter>
```

The shell handles the intent on both a cold launch (the launch intent's data) and a warm one
(`onNewIntent`), maps a `bonfire://host/path` onto `https://…/host/path`, and loads it. `autoVerify`
is what makes Android fetch your `assetlinks.json`; until it succeeds the https link shows a chooser,
and the custom scheme is the reliable fallback.

**iOS** (`clients/ios`) — the same `CFBundleURLTypes` custom scheme, handled in `application(_:open:)`, plus Universal Links via `application(_:continue:)` (with the associated-domains entitlement). **macOS** — a custom scheme in `CFBundleURLTypes`, handled through an Apple event. Because a launch
event can arrive before the web view has loaded, the shell **queues** the link and delivers it once
the first page is ready:

```swift
NSAppleEventManager.shared().setEventHandler(
    self, andSelector: #selector(handleURLEvent(_:withReplyEvent:)),
    forEventClass: AEEventClass(kInternetEventClass), andEventID: AEEventID(kAEGetURL))
```

Universal (https) links on macOS additionally need the **associated-domains entitlement**, which
comes from a provisioning profile and so a paid Apple Developer account. The custom scheme has no
such requirement, which is why the shell ships it.

## Testing

```bash
# Android — simulate either link type against a connected device
adb shell am start -a android.intent.action.VIEW -d "https://bonfire-app.pro/pings/3"
adb shell am start -a android.intent.action.VIEW -d "bonfire://pings/3"

# macOS — the custom scheme
open "bonfire://pings/3"

# Verify the host files before you rely on them
curl -s https://bonfire-app.pro/.well-known/assetlinks.json
curl -sI https://bonfire-app.pro/.well-known/apple-app-site-association   # must be application/json, 200, no redirect
```

Google's [Statement List Generator and Tester](https://developers.google.com/digital-asset-links/tools/generator)
and Apple's swcutil (`swcutil dl -d bonfire-app.pro`) check the files the way the OS does.

## Related

- [Notifications](/docs/native/notifications) — notification taps carry a URL, routed the same way
- [Native Bridge](/docs/development-tools/native-bridge) — the capability table
