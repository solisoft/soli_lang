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

Each flag falls back to `config/mobile.toml` (below), then to a default: app
name and package derived from the project folder.

Output: `clients/<platform>/`, or `clients/android-fcm/` with `--fcm`.

## `config/mobile.toml`

`soli generate client`, `soli mobile build` and `soli mobile publish` read their
shared settings from one optional file, so the same flags are not repeated three
times. Flags always win over it.

```toml
# config/mobile.toml
[mobile]
url = "https://staging.shop.example.com"   # what the WebView loads
package = "com.example.shop"               # Android package and iOS bundle id
scheme = "shop"
name = "Shop"
team_id = "ABCDE12345"                     # iOS only
fcm = false                                # Android: build clients/android-fcm instead

[mobile.publish]
url = "https://staging.shop.example.com"   # where `soli mobile publish` uploads
```

The shell's version is not repeated here: it is `[package].version` in
`soli.toml`. An unknown key is a warning, not an error. It lives apart from
`soli.toml` because that manifest refuses unknown sections, so a `[mobile]` there
would break every older soli on the project.

## Build: `soli mobile build`

```bash
soli mobile build android                  # dist/mobile/shop-1.4.0-<build>.apk
soli mobile build android --build-number 42 --out dist/mobile
soli mobile build android --fcm            # the Gradle + Firebase shell
soli mobile build ios --team-id ABCDE12345 # macOS only: an ad hoc IPA
soli mobile build ios --export enterprise
soli mobile build android --publish --notes "Fixes the cart"
```

The shell in `clients/<platform>/` is copied to a temporary directory and
stamped there with the version from `soli.toml` and a build number, so your tree
is never edited by a build. A missing shell is generated first from
`config/mobile.toml`, the way `soli generate client` would.

| | |
|---|---|
| Version | `[package].version` in `soli.toml` (`0.0.0` without one) → `versionName` / `CFBundleShortVersionString` |
| Build number | `--build-number`, else `GITHUB_RUN_NUMBER` / `CI_PIPELINE_IID`, else minutes since the Unix epoch → `versionCode` / `CFBundleVersion` |
| Output | `dist/mobile/<name>-<version>-<build>.{apk,ipa}`, plus a `.mobile.json` stub (platform, version, build number, bundle id, sha256, size) that `publish` reads |

**Android, plain shell.** Runs the generated `build.sh`. The SDK pieces it needs
(`ANDROID_HOME`, build-tools 35, platform 34, a JDK, `zip`) are checked first and
named when missing. The APK is signed with `clients/android/debug.keystore`,
created on the first build and kept there: a new key per build would make Android
refuse to install an update over the previous one. With `--keystore PATH` (or
`SOLI_ANDROID_KEYSTORE`) it is signed again with that key; the passwords come from
`SOLI_ANDROID_KEYSTORE_PASSWORD`, `SOLI_ANDROID_KEY_PASSWORD` and
`SOLI_ANDROID_KEY_ALIAS`, never from the command line.

**Android, FCM shell.** Picked with `--fcm` or `fcm = true`; without either,
whichever of `clients/android` and `clients/android-fcm` exists is built, and
having both is refused until you choose. Runs `./gradlew` when you have added a
wrapper, `gradle` otherwise, and needs `app/google-services.json`. Without a
keystore it builds `assembleDebug`, signed with Gradle's debug key, because an
unsigned release APK does not install. With `--keystore` it builds
`assembleRelease`, passing the key through `ORG_GRADLE_PROJECT_*` variables so
your `build.gradle` is untouched.

**iOS.** macOS only, with `xcodegen` and Xcode. Runs `xcodegen generate`,
`xcodebuild archive` and `xcodebuild -exportArchive` with automatic signing and
`-allowProvisioningUpdates`. `--export ad-hoc` (the default: devices registered
on your profile) or `enterprise`. It needs your team id: `--team-id`, or
`team_id` in `config/mobile.toml`.

## Distribute: `/__soli/mobile`

Every app can host its own builds. `/__soli/mobile` is an operator page beside
`/__soli/errors` and `/__soli/jobs`: it lists the builds, shows a QR code for the
latest build of each app (one per bundle id and platform, captioned with the app's
name, so a customer app and a merchant app each get theirs), and gives each build an
install page.

It is **off unless configured**, behind the same gate as the other operator
pages:

| Variable | Meaning |
|---|---|
| `SOLI_MOBILE_TOKEN` | Bearer token for uploads and the page. Set it on the server and in CI. |
| `SOLI_MOBILE_USER` / `SOLI_MOBILE_PASSWORD` | Basic auth for the page in a browser |
| `SOLI_ADMIN_*` | The shared operator credentials also open it |
| `SOLI_MOBILE_PATH` | Where builds are stored. Default `./storage/mobile` |
| `SOLI_MOBILE_MAX_SIZE` | Upload cap in bytes. Default 512 MiB, independent of `SOLI_MAX_BODY_SIZE` |
| `SOLI_MOBILE_KEEP` | Builds kept per app (bundle id and platform); older ones are deleted with their files. Default 20 |
| `SOLI_MOBILE_PUBLIC_INSTALL` | `0` turns the install links off |

With none of the credentials set, the page answers 404, so production does not
advertise it. Under `--dev` it is open to a request from the machine itself,
and its install links and QR codes use this machine's LAN address instead of
`localhost`, so a phone on the same network can open them.

Upload from a laptop or CI:

```bash
export SOLI_MOBILE_TOKEN=…                      # the server's value
soli mobile publish dist/mobile/shop-1.4.0-42.apk --notes "Fixes the cart"
soli mobile publish --latest android            # the newest .apk in dist/mobile
```

The URL comes from `--url`, else `[mobile.publish] url`, else `[mobile] url`. The
token is read only from the environment: `--token` is refused, so it never shows
in `ps` or a CI log. The command prints the install URL and exits non-zero on any
refusal.

The upload is a multipart `POST /__soli/mobile/builds` with
`Authorization: Bearer`, streamed to disk with a hard cap; it never reaches a
worker. Fields: `file` (required), `platform`, `version`, `build_number`,
`notes` (≤ 10 kB), `name`, and `bundle_id` (required for iOS). It answers `201`
with `install_url` and `download_url`, `401`, `413` over the cap, or `422`.

**Install links.** Each build gets a random, unguessable token:
`/__soli/mobile/i/<token>` is a phone-friendly page with the download (Android)
or install (iOS) button, and the QR code on the operator page points at it.
These links skip the gate on purpose, because iOS fetches the manifest and the IPA
outside Safari's cookies and a phone scanning a QR code never signed in. Deleting
the build revokes its link. Downloads support `Range` and are sent as
attachments with `nosniff`, and every install response carries `noindex`.

iOS installs only over HTTPS. Behind a TLS-terminating proxy, set
`SOLI_TRUST_PROXY=1` so the page builds `https://` links; reached over plain
`http`, it says so and shows no iOS button.

## Platforms

| Platform | Output | Build | Closed-app push |
|----------|--------|--------|-----------------|
| `android` | No-Gradle WebView APK | `soli mobile build android` (or `./build.sh`; `ANDROID_HOME`, build-tools 35, platform 34) | Bridge only until you add FCM |
| `android --fcm` | Gradle app + FCM service | `google-services.json` + `soli mobile build android --fcm` | Token → `POST /devices` |
| `ios` | XcodeGen project | `soli mobile build ios` (or `xcodegen generate` then Xcode) | APNs token → `POST /devices` |
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
| Command | `soli desktop build` | `soli generate client …`, `soli mobile build` |

See the [native mobile blog post](/docs/blog/native-mobile) for the product story.

## Related

- [Device registration](/docs/native/devices)
- [Notifications](/docs/native/notifications)
- [Deep Links](/docs/native/deep-links)
- [Platform limits](/docs/native/platform-limits)
- [Desktop Applications](/docs/development-tools/desktop)
- [Native Bridge](/docs/development-tools/native-bridge)
