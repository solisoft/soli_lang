# Mobile Preview Design

**Status:** Proposal — not implemented. Nothing below exists yet unless marked *existing*.
**Related:** `soli generate client` (`src/scaffold/client_generator.rs`), operator pages (`src/serve/admin_auth.rs`, `src/serve/operator_shell.rs`), attachments (`src/interpreter/builtins/attachments.rs`), auto-update (`src/update.rs`).

## Problem

`soli generate client android|ios` stops at a source tree. A developer who wants a
client to try the app on a phone has to work out two things alone:

1. **Build.** Android has the generated `build.sh`; iOS has a README line saying
   "run xcodegen, open Xcode". Nothing archives or exports an installable IPA,
   and nothing stamps the app's version into either shell.
2. **Distribute.** There is no place to put the APK/IPA, no install page, no QR
   code, no iOS over-the-air manifest. Developers fall back to e-mailing APKs,
   Diawi, or Firebase App Distribution.

Expo solves the same problems for React Native with EAS Build and internal
distribution. Soli's shells are thinner, so the equivalent is much smaller.

## Goals

1. `soli mobile build android|ios` produces an installable, versioned artifact.
2. `soli mobile publish` sends it to a distribution endpoint from a laptop or CI.
3. Every Soli app can *be* that endpoint: an operator page at `/__soli/mobile`,
   **off unless configured**, gated like `/__soli/errors`, listing builds with
   install pages and QR codes.

## Non-goals

- Store submission (App Store Connect, Play Console, TestFlight upload).
- Code signing management beyond what Xcode's automatic signing already does.
- OTA content updates: the WebView already loads the live deployment.
- A hosted, multi-tenant distribution service. Every app distributes its own
  builds.
- A generic launcher app (an Expo Go equivalent). It is sketched under
  *Future: the Soli Go launcher* below, but no phase of this proposal builds
  it.

## Overview

```
 laptop / CI                         the app's own server (staging)
 ┌──────────────────────────┐        ┌──────────────────────────────────┐
 │ soli mobile build android │        │ /__soli/mobile          (gated)  │
 │   → dist/mobile/*.apk     │ ─────▶ │   POST /builds  (token, stream)  │
 │ soli mobile publish       │ upload │   list · delete · QR             │
 └──────────────────────────┘        │ /__soli/mobile/i/<install token> │
                                     │   page · download · manifest     │
 phone                               └──────────────────────────────────┘
 ┌──────────────────────────┐                 ▲
 │ camera → QR → install    │─────────────────┘
 └──────────────────────────┘
```

## 1. Configuration

The app already knows almost everything a shell needs. A new
`config/mobile.toml` holds the rest, so `generate client`, `mobile build`, and
`mobile publish` stop taking the same flags three times:

```toml
# config/mobile.toml
[mobile]
url = "https://staging.shop.example.com"
package = "com.example.shop"        # Android package and iOS bundle id
scheme = "shop"
name = "Shop"
team_id = "ABCDE12345"              # iOS only
fcm = false                         # Android: build clients/android-fcm (Gradle) instead

[mobile.publish]
url = "https://staging.shop.example.com"   # where `soli mobile publish` uploads
```

The version is not repeated here. The shell's `versionName` /
`MARKETING_VERSION` is read from the *existing* `[package].version` in
`soli.toml`, which stays the single source of truth for it.

**Why not `soli.toml`.** `Package::parse` (`src/module/package.rs`) rejects
any section other than `[package]` and `[dependencies]`, and any unknown
`[package]` key, with a hard error. A `[mobile]` section, or `mobile_*` keys
under `[package]`, would make every older soli refuse the project, `soli add`
and the `soli_version` pin-and-switch path included. `[dependencies]` is no
option either: each key there is resolved as a package. A separate file is read
only by the commands that know about it, so no released binary breaks. It
follows the `deploy.toml` precedent, whose parser lives apart from the command
in `src/module/deploy_config.rs`, and `soli.toml` stays a package manifest
for the registry.

The file is optional. When it is missing, every command still works from
flags. CLI flags always override the file, and `generate client` keeps
accepting them, so existing scripts keep working. Unknown keys in
`config/mobile.toml` produce a warning, not an error. That way a key added
later does not break the first releases that read the file, which is the
mistake `Package::parse` made.

| Variable | Read by | Meaning |
|---|---|---|
| `SOLI_MOBILE_TOKEN` | server, CLI | Bearer token for uploads and the operator page. On the CLI it authenticates `publish`. |
| `SOLI_MOBILE_USER` / `SOLI_MOBILE_PASSWORD` | server | Basic auth for the operator page (browser). |
| `SOLI_ADMIN_*` | server | *Existing* — unlocks every operator page, this one included. |
| `SOLI_MOBILE_PATH` | server | Artifact directory. Default `./storage/mobile`. |
| `SOLI_MOBILE_MAX_SIZE` | server | Upload cap in bytes. Default 512 MiB. Independent of `SOLI_MAX_BODY_SIZE`. |
| `SOLI_MOBILE_KEEP` | server | Builds kept per platform. Older ones are deleted with their files. Default 20. |
| `SOLI_MOBILE_PUBLIC_INSTALL` | server | `0` disables install links even when the page is enabled. Default on. |

## 2. `soli mobile build android|ios`

Nested under `mobile` because a top-level `publish` already exists (the package
registry, `Command::Publish`). Parsed like the `desktop` block in
`src/cli/args.rs`, implemented in a new `src/cli/commands/mobile.rs`.

```bash
soli mobile build android                 # uses clients/android, generating it first if missing
soli mobile build ios --export ad-hoc     # macOS only
soli mobile build android --build-number 42 --out dist/mobile
```

Steps:

1. **Source.** Use `clients/<platform>/` when present (the developer may have
   customised it). Otherwise run the existing generator into a temp dir from
   `config/mobile.toml`.
   - Android has two shells. `generate client android --fcm` writes a Gradle
     project to `clients/android-fcm/` instead of the `build.sh` tree in
     `clients/android/`. `mobile build android` takes `--fcm`, or
     `fcm = true` in `config/mobile.toml`, to pick it. Without either, it uses
     whichever of the two directories exists, and refuses with a message
     naming both when both exist.
2. **Version.** Copy the source to a temp dir and stamp `versionName` /
   `versionCode` (the `android:` attributes in `AndroidManifest.xml` for the
   plain shell, the `defaultConfig` lines in `app/build.gradle` for the FCM
   shell) or `CFBundleShortVersionString` / `CFBundleVersion` (in the
   app's `Info.plist`; the template hard-codes `1.0` / `1`, so passing
   `MARKETING_VERSION` to `xcodebuild` alone would not reach it). The build number comes
   from `--build-number`, otherwise `GITHUB_RUN_NUMBER` / `CI_PIPELINE_IID`,
   otherwise a timestamp. The developer's tree is never modified.
3. **Compile.**
   - Android runs the generated `build.sh`, which needs `ANDROID_HOME`. The
     command checks build-tools 35 and platform 34 up front and names what is
     missing, instead of letting `aapt2` fail mid-way. A release keystore can be
     given with `--keystore` / `SOLI_ANDROID_KEYSTORE`; the default stays the
     generated debug keystore, which is enough for test installs.
   - The FCM shell runs Gradle (`./gradlew` when the developer has added a
     wrapper, `gradle` otherwise; the template ships none). Its
     `app/build.gradle` has no `signingConfig`, so `assembleRelease` would
     produce an unsigned APK that Android refuses to install. The default is
     therefore `assembleDebug`, which is signed with Gradle's debug key and
     matches the plain shell's test-install story. With `--keystore`, the
     command passes the keystore through `-Pandroid.injected.signing.*`
     properties and runs `assembleRelease`, so the developer's
     `build.gradle` stays untouched. It is refused up front, with the
     README's instructions, when `app/google-services.json` is missing,
     since only the `.example` file is generated.
   - iOS runs `xcodegen generate`, `xcodebuild archive`, and
     `xcodebuild -exportArchive` with an `ExportOptions.plist`. It uses Xcode's
     `release-testing` method (ad hoc) by default and `enterprise` on request,
     with automatic signing and `-allowProvisioningUpdates`. It is refused with a
     clear message off macOS, and when `xcodegen` or the team id is missing.
4. **Output.** `dist/mobile/<name>-<version>-<build>.{apk,ipa}`, plus a
   `.mobile.json` stub with `platform, version, build_number, sha256, size`. It
   reuses `desktop::container::sha256_hex` and the `emit_update_stub` pattern
   from `src/cli/commands/mod.rs`.

## 3. `soli mobile publish`

```bash
soli mobile publish dist/mobile/shop-1.4.0-42.apk --notes "Fixes the cart"
soli mobile publish --latest android             # newest artifact in dist/mobile
soli mobile build android --publish              # build, then publish
```

It does a multipart `POST <publish.url>/__soli/mobile/builds` with
`Authorization: Bearer $SOLI_MOBILE_TOKEN` and the fields of the upload
contract below, using the blocking `reqwest` client already used by
`src/module/registry.rs`. It prints the install URL and exits non-zero on any
status other than 201, so CI fails visibly.

### Upload contract

The contract:

```
POST /__soli/mobile/builds
Authorization: Bearer <token>
Content-Type: multipart/form-data

file          the .apk or .ipa (required)
platform      android | ios   (optional — inferred from the extension)
version       string          (optional — default "0.0.0")
build_number  string          (optional)
notes         string          (optional, ≤ 10 kB)

201 {"id", "platform", "version", "build_number", "size", "sha256",
     "install_url", "download_url"}
401 missing / wrong token        413 over SOLI_MOBILE_MAX_SIZE
422 {"error": "..."} wrong extension for the platform, empty file
```

## 4. The in-app page: `/__soli/mobile`

An operator page alongside `/__soli/errors`, `/__soli/jobs`, and
`/__soli/slow_queries`, built the same way.

**Gate.** `admin_auth::authorize(headers, dev_mode, peer_ip, "MOBILE")`.

- Under `--dev`, a request from the machine itself is allowed with no
  configuration: a trusted dev peer *and* a local `Host` header, so a
  DNS-rebound name cannot reach it. Anyone else on the LAN falls through to
  the credential check.
- Elsewhere, Basic auth (`SOLI_MOBILE_USER`/`_PASSWORD`) or Bearer
  (`SOLI_MOBILE_TOKEN`) is required, or the shared `SOLI_ADMIN_*` credentials.
- With nothing configured the page answers **404**. It is therefore off in
  production by default and is enabled per environment by setting a token on
  staging.

**Dispatch.** A new `src/serve/dev_mobile.rs`, with a dispatch stage in
`handle_hyper_request`. The upload rules out the `dev_errors` /
`dev_slow_queries` shape: those are sync `pub(crate) fn dispatch` functions
that see only the request head. `dev_mobile::dispatch` follows
`dev_routes::dispatch` instead, as a `pub(super) async fn` that takes
ownership of `req`, so the upload body is **streamed to a temp file** in the
async handler. It never reaches the worker, whose buffered body path
caps requests at 8 MiB. The size cap is enforced while streaming.

**Storage.**

- Metadata lives in a framework-owned `_soli_mobile_builds` collection, through
  `internal_store`, so SoliDB and the SQL adapters both work. This is how
  `_soli_errors` is stored.
- Files live under `SOLI_MOBILE_PATH/<id>/`. The temp file is moved into place
  with `store_disk_from_path` in `attachments.rs`.
- Retention (`SOLI_MOBILE_KEEP`) prunes after each upload.

**Pages** (rendered with `operator_shell::page`, new `Section::Mobile`):

| Route | What |
|---|---|
| `GET /__soli/mobile` | Latest build per platform with its QR code, the build history, and a copy-paste CI snippet. |
| `POST /__soli/mobile/builds` | The upload contract. Bearer only, never a cookie. |
| `POST /__soli/mobile/builds/:id/delete` | Delete a build. It must be added to `csrf::is_operator_dashboard_path` so it keeps the Origin check. |

**Install links (public by secret).**

- Routes:
  - `GET /__soli/mobile/i/:token`: a mobile-friendly install page.
  - `GET /__soli/mobile/i/:token/download`: the binary, streamed with Range
    support. `static_files::stream_file` is exposed for this rather than going
    through `read_attachment`, which base64-encodes the whole file in memory.
  - `GET /__soli/mobile/i/:token/manifest.plist`: the iOS `itms-services`
    manifest.
- These cannot sit behind the operator gate. iOS fetches the manifest and the
  IPA itself, outside Safari's cookie jar, and a QR code is scanned on a device
  that never signed in.
- So each build gets its own random token: 24 bytes, unguessable, unique, and
  revoked by deleting the build. `SOLI_MOBILE_PUBLIC_INSTALL=0` turns the links
  off entirely.

**Response headers.**

- Downloads are sent with `Content-Disposition: attachment; filename="…"`,
  `X-Content-Type-Options: nosniff`, and the right type
  (`application/vnd.android.package-archive`, `application/octet-stream`).
- Install pages carry `noindex`.

**Visibility changes.** Four private helpers become `pub(crate)`. None
changes behaviour:

| Helper | Today | Why the page needs it |
|---|---|---|
| `store_disk_from_path` (`src/interpreter/builtins/attachments.rs`) | private | Moves the streamed temp file into `SOLI_MOBILE_PATH`. |
| `sanitize_part` (`src/interpreter/builtins/attachments.rs`) | private | The `..`-blocking path sanitizer, applied to every path segment built from an upload. |
| `stream_file` (`src/serve/static_files.rs`) | private | Range downloads without loading the binary into memory. |
| `is_operator_dashboard_path` (`src/serve/csrf.rs`) | private | Not called from `dev_mobile`; it gains the `/__soli/mobile` prefix in place. Listed so the CSRF change is not missed. |

`admin_auth::authorize` is `pub(super)`, which is already enough for a
module in `src/serve/`. `desktop::container::sha256_hex` and
`emit_update_stub` (`pub(crate)`) are already reachable from
`src/cli/commands/`.

**QR codes** are rendered as SVG on the server, with no JavaScript, through a
direct `qrcode` dependency. That crate is already in the tree, but only behind
the `pdf` feature (`pdf/src/qr.rs`). A `qr_svg(text)` builtin could fall out of
this for free; the docs currently say "No QR builtin".

## Security summary

- The operator page and the upload endpoint are **off unless a credential is
  configured**. They reuse the existing gate, with no new auth code.
- Uploads are Bearer-only, so they carry no cookie and pass the Origin gate
  without opening a CSRF hole. Only HTML actions (delete) are added to the
  operator CSRF list.
- The body is streamed with a hard cap. Extensions are checked against the
  platform, file names are generated rather than taken from the client, and
  the `..`-blocking path sanitizer from `attachments.rs` is reused.
- Install tokens are per build, random, and revocable. They grant read access
  to one binary and its notes, nothing else.
- The CLI reads the token from the environment and never accepts it on argv by
  default, so it does not show up in `ps` output or CI logs.
- iOS over-the-air installs require HTTPS. The page warns when the request
  scheme is `http` and hides the iOS button.

## Phases

Each phase ships with tests and every documentation surface listed in
`CLAUDE.md` → *Documentation Policy*:

- `www/docs` and the hand-written `.slv` pages;
- both changelogs;
- the comparison page;
- the `soli-lang` skill.

| Phase | Content | Tests |
|---|---|---|
| **1 — build** | `config/mobile.toml` and its parser; `soli mobile build android` for both the `build.sh` and the FCM Gradle shell; version stamping from `[package].version`; `.mobile.json` stub; `generate client` reads `config/mobile.toml`. | `args.rs` parse tests; parser tests for `config/mobile.toml` (missing file, unknown keys warn, flags override); a `tests/mobile_build_test.rs` that stamps both generated Android trees (manifest and `app/build.gradle`) and checks the shell choice when one, the other, or both directories exist (the compile step is skipped without `ANDROID_HOME`, and required under a CI flag). |
| **2 — distribute** | `/__soli/mobile` (gate, streamed upload, `_soli_mobile_builds`, retention, install pages, Range download, QR); `soli mobile publish`. | Unit tests for the gate decisions, as in `admin_auth.rs`. An e2e test starts `soli serve`, uploads with and without the token, checks the 404 when unconfigured, the 413 over the cap, a download that matches byte for byte, and a Range request. Plus a publish → install round trip. |
| **3 — iOS** | `soli mobile build ios` (archive and export); `manifest.plist`. | Manifest rendering unit tests; the build is macOS-gated in CI. |

Phases 1 and 2 deliver most of the value: together they already replace
"e-mail me the APK". Phase 3 adds iOS on the same server side.

## Future: the Soli Go launcher

> **Not part of this proposal.** Nothing in phases 1–3 builds, depends on, or
> links to the launcher. It is written down here as a possible next step, so
> the distribution page can grow into it later. It needs a decision from the
> maintainers first: the project would have to publish an app in the stores.

Because a Soli shell is a WebView on a deployed URL, most changes need **no new
build at all**. Today, though, there is no way to open "my staging app" inside a
native shell without building one for it. Expo Go solves this for React Native.
The Soli equivalent would be **one app, published by the project in the
stores**:

- It would ship as a new client template, `src/scaffold/templates/clients/launcher/{android,ios}`,
  next to the existing `android`, `android_fcm`, `ios`, `linux`, and `windows`
  ones, and derived from the plain `android` and `ios` shells.
- It would keep the same bridge contract (`soliNativeHost` / `window.soli.native`),
  so camera, geolocation, sharing, and haptics behave as they do in a real
  shell.
- `IN_APP_HOSTS` would no longer be a constant. The host the user chose to open
  becomes in-app for that session; other hosts open externally, as today.
- It would register a `soligo://open?url=<https-url>` deep link. An "Open in
  Soli Go" QR code on `/__soli/mobile` would encode it, and the phone's camera
  app would hand it to the launcher.
- Its home screen would be native: recently opened apps (stored on the
  device), a "Scan" button, and a "Paste URL" field.
- A floating ⌂ control, or a shake, would return to it from inside an app.
- It would need no server of its own.

What a launcher cannot do is anything that needs per-app native configuration:
a custom URL scheme, push notifications under the app's own bundle id,
Universal Links, or an app icon. Those always need a real build, which is what
phases 1–3 provide, so the launcher would complement them, not replace them.

**Store review.** A WebView launcher that opens arbitrary HTTPS sites is a
browser-like app. Expo Go is the precedent, but the listing must present it as a
developer tool, and iOS forbids downloading executable code. The launcher only
renders web content, which is allowed.

**Questions it would raise:** would the project publish the launcher in the
stores, and under which name and account?

## Open questions for the maintainers

1. Is `/__soli/mobile` the right home, as an operator page beside errors and
   jobs, or should distribution stay out of the framework, leaving only build
   and publish in the CLI?
2. Should QR rendering become a public builtin (`qr_svg`) while the dependency
   is being added?
3. Is a default retention of 20 builds per platform, with files under
   `storage/mobile`, acceptable for the typical staging box?
