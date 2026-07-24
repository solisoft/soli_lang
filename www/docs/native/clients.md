# Native clients

Generate thin OS shells that load your **deployed** Soli app in a WebView and
speak the [native bridge](/docs/development-tools/native-bridge).

This is **not** a second UI rewrite. The product stays Soli (models, templates,
LiveView, jobs). The shell is distribution chrome: icon, permissions, OS
notifications when closed, deep links, store packaging.

For a local offline product with an embedded database, use
[`soli desktop build`](/docs/development-tools/desktop) instead.

## End-to-end recipe

```bash
# 1. App (once)
soli new myapp && cd myapp
soli generate auth                 # optional but typical
# set SOLI_SESSION_SECRET (32+ chars) in .env

# 2. Server pieces for push + links
soli generate devices
soli generate app_links \
  --android-package net.example.myapp \
  --apple-app-id TEAMID.net.example.myapp \
  --paths "/pings/*,/threads/*"
soli db:migrate up

# 3. Shells
soli generate client android --url https://app.example.com \
  --package net.example.myapp --scheme myapp
soli generate client android --fcm --url https://app.example.com \
  --package net.example.myapp --scheme myapp
soli generate client ios --url https://app.example.com \
  --bundle-id net.example.myapp --team-id TEAMID --scheme myapp
```

In the authenticated layout:

```erb
<%- csrf_meta_tag() %>
<% user_id = session_get("user_id") %>
<%- native_channel("user:#{str(user_id)}") rescue "" unless user_id.nil? %>
```

After login (page or shell):

```js
// Web Push
await soli.nativeBridge.registerDevice({
  platform: "web",
  subscription: subscriptionJson
})
// Shells usually POST /devices themselves once the session cookie exists
```

Notify:

```soli
deliver_to_user(user.id, {
  "title": "New ping",
  "body":  "Ana replied",
  "url":   "/pings/3"
}, {
  "apns": apns_options,
  "fcm":  fcm_options
})
```

## Generate

```bash
soli generate client <platform> [options] [folder]
```

| Flag | Meaning |
|------|---------|
| `--url` | Deployment origin the WebView loads (include trailing `/` or not — normalized) |
| `--package` / `--bundle-id` | Android package or iOS bundle id |
| `--scheme` | Custom URL scheme (`myapp://…`) |
| `--name` / `--app-name` | Display name |
| `--team-id` | Apple team id (iOS project + entitlements) |
| `--fcm` | Android only: Gradle + Firebase Messaging template |

Defaults: app name and package derived from the project folder when omitted.

Output: `clients/<platform>/`, or `clients/android-fcm/` with `--fcm`.

## Platforms

| Platform | Output | Build | Closed-app push |
|----------|--------|--------|-----------------|
| `android` | No-Gradle WebView APK | `ANDROID_HOME` + `./build.sh` (build-tools 35, platform 34) | Bridge only until you add FCM |
| `android --fcm` | Gradle app + FCM service | `google-services.json` + `./gradlew :app:assembleRelease` | Token → `POST /devices` |
| `ios` | XcodeGen project | `xcodegen generate` then Xcode | APNs token → `POST /devices` |
| `linux` | GTK + WebKitGTK crate | `cargo build --release` | Prefer bridge; use web push if needed |
| `windows` | WebView2 (.NET 8) | `dotnet build` on Windows | Same |

**macOS local products** use [`soli desktop build`](/docs/development-tools/desktop)
or embed with `SOLI_DESKTOP_NO_WINDOW`. There is no `generate client macos` for a
remote WebView shell yet — start from the iOS template if you need one.

## What each shell does

1. Loads `START_URL` (and deep-link paths) in a system WebView / WKWebView.
2. Injects the bridge contract expected by `src/serve/native.js`
   (`soliNativeHost` or `window.soli.native`).
3. Handles OS permissions the web view would otherwise deny silently (camera, location, notifications).
4. Routes custom schemes and App/Universal Links into the WebView.
5. (FCM / APNs builds) Obtains a device token and POSTs `/devices` with the session cookie when the user is logged in.

## Free vs paid (Apple)

| Capability | Free provisioning | Paid account |
|------------|-------------------|--------------|
| Custom scheme deep links | ✅ | ✅ |
| Camera, geo, haptics, share, biometrics | ✅ | ✅ |
| APNs push | | ✅ `aps-environment` |
| Universal Links | | ✅ associated domains |
| Core NFC | | ✅ entitlement |

## Server scaffolds to pair

| Command | Why |
|---------|-----|
| [`soli generate devices`](/docs/native/devices) | Token store + `deliver_to_user` + prune |
| [`soli generate app_links`](/docs/native/deep-links) | Host proof files for https deep links |
| [`soli generate offline`](/docs/native/offline) | Optional outbox for flaky radio |

## Desktop vs mobile

| | Desktop artifact | Mobile shell |
|--|------------------|--------------|
| Process | Bundled Soli + SolidB | Thin WebView |
| Data | Local private DB | Your server |
| Updates | Signed OTA channel | Deploy server; store for shell binary |
| Command | `soli desktop build` | `soli generate client …` |

See the [native mobile blog post](/docs/blog/native-mobile) for the product story.

## Related

- [Device registration](/docs/native/devices)
- [Notifications](/docs/native/notifications)
- [Deep Links](/docs/native/deep-links)
- [Platform limits](/docs/native/platform-limits)
- [Desktop Applications](/docs/development-tools/desktop)
- [Native Bridge](/docs/development-tools/native-bridge)
