# Auto-Update (OTA)

A `soli build --standalone` or `soli desktop build` artifact is a **frozen binary**.
Without auto-update, shipping a fix means asking every user to re-download and
replace it by hand. Auto-update turns that artifact into a product that stays
current: it checks a release channel you control, and replaces itself with a
newer, **cryptographically verified** build.

> Mobile shells (iOS / Android) don't use this — they're WebViews onto a remote
> URL, so their content updates the moment you deploy, and the App Store / Play
> Store update the shell binary. Auto-update targets the one truly frozen thing:
> the **desktop / standalone artifact**.

## The shape of it

```
              build time                          run time
  ┌───────────────────────────┐        ┌──────────────────────────────┐
  soli build --standalone \             ./myapp --check-update
    --update-url https://… \    ──▶     ./myapp --update
    --update-key <pubkey>               Updater.check() / Updater.apply()
  (embeds an update descriptor)         (fetch → verify signature →
                                         download → verify sha256 → swap)
```

1. At **build time** you embed an *update descriptor* — where to look, and the
   public key updates must be signed with.
2. You **publish** a signed `latest.json` manifest and the new artifact.
3. The **running app** checks the channel, verifies the manifest signature
   against the embedded key, downloads the artifact for its own platform,
   verifies its sha256, and atomically replaces itself.

## Building an updatable artifact

Two new flags on `soli build --standalone`:

```bash
soli build ./my_app --standalone \
  --update-url https://updates.example.com/my_app \
  --update-key <ed25519-or-p256-pubkey>
```

| Flag | Meaning |
|------|---------|
| `--update-url <base>` | Base URL you publish updates under. The app fetches `<base>/<channel>/latest.json`. |
| `--update-key <pubkey>` | Base64 P-256 public key the app verifies manifest signatures against. Omit only for local testing. |

The app's own version is read from its `soli.toml` `[package] version`, so bump
that before each release — it's what the updater compares against.

Building with `--update-url` also drops a `<output>.update.json` **stub** next to
the artifact, pre-filled with this build's version, sha256 and size — ready to
merge into your published manifest.

```
  ✓ Standalone executable written to myapp (55.0 MB, 0.4 KB app bundle)
    Platform: linux-amd64 — run it with: ./myapp --port 8080
    Update stub: myapp.update.json (merge into latest.json, then `soli sign-update`)
```

## Signing keys

An auto-updater is a remote-code-execution channel — treat it like one. The
artifact is downloaded from **your** URL, so the sha256 in the manifest is not
enough on its own: whoever can serve a malicious binary can serve a matching
sha256. So the manifest is **signed**, and the app verifies that signature
against the key you embedded at build time.

```bash
# Generate a keypair once. Keep the private key secret; embed the public one.
soli update-keygen
```

```
-----BEGIN PRIVATE KEY-----
…                                 # keep this secret — it signs releases
-----END PRIVATE KEY-----

# Public key — pass to `soli build --update-key`:
BCwQ3U9ezl09YZjeew31+Q2W9NH5mUsGs6QS1yeX/CLaOyrk…
```

Save the private key somewhere safe (a secrets manager, not the repo). Embed the
public key in every build. Rotating the key means every already-shipped artifact
stops trusting new updates — so rotate deliberately.

## Publishing an update

1. **Bump** `soli.toml` `[package] version` and build the new artifact for each
   platform you ship (`--target linux-amd64|linux-arm64|darwin-arm64|…`).
2. **Assemble** `latest.json` from each build's `.update.json` stub — one entry
   per platform, keyed by target:

```json
{
  "version": "1.1.0",
  "notes": "Bug fixes and faster startup.",
  "artifacts": {
    "darwin-arm64": { "url": "https://updates.example.com/my_app/stable/myapp-1.1.0-darwin-arm64", "sha256": "…", "size": 60123904 },
    "linux-amd64":  { "url": "https://updates.example.com/my_app/stable/myapp-1.1.0-linux-amd64",  "sha256": "…", "size": 57656963 }
  }
}
```

3. **Sign** it — this adds the `signature` field:

```bash
soli sign-update latest.json --key private.pem
```

4. **Upload** `latest.json` to `<update_url>/stable/latest.json` and each
   artifact to the URL you listed. HTTPS is required.

The `url` in each stub is a best-effort guess (`<update_url>/stable/<filename>`).
Edit it to wherever you actually host the file.

## Checking and applying from the CLI

Every built artifact understands two flags:

```bash
./myapp --check-update      # fetch + verify manifest, compare versions, report
./myapp --update            # do the above, then download + verify + self-replace
```

```
$ ./myapp --check-update
Update available: 1.0.0 → 1.1.0

Bug fixes and faster startup.

Run with --update to install it.

$ ./myapp --update
updated to v1.1.0
```

A failed or tampered download never touches the installed binary — the new
artifact is staged, verified, and only then atomically renamed into place.
Downgrades are refused.

## Driving updates from Soli — the `Updater` builtin

To offer an in-app "update available — restart to apply" affordance instead of
the terminal, use the `Updater` class from any controller or view:

```soli
def index(req) {
  info = Updater.check()
  # { "configured": true, "available": true,
  #   "current": "1.0.0", "latest": "1.1.0", "notes": "…" }
  return render("home/index", { "update": info })
}

def install_update(req) {
  result = Updater.apply()
  # { "status": "updated", "restart_required": true, "message": "updated to v1.1.0" }
  return { "status": 200, "body": result.to_json() }
}
```

| Method | Returns |
|--------|---------|
| `Updater.version()` | The embedded app version (`"1.0.0"`), or `null` if this build has no update channel. |
| `Updater.check()` | `{ configured, available, current, latest, notes }` — or `{ configured: false, error }` outside an artifact. |
| `Updater.apply()` | `{ status, restart_required, message }` on success, `{ status: "error", error }` on failure. |

Outside a built artifact (e.g. `soli serve` in development) every method degrades
gracefully — `version()` is `null`, and `check()`/`apply()` report
`configured: false` rather than raising — so the same controller code runs in dev
and in the shipped app.

## Security summary

- **Signed manifests.** Updates are verified with P-256 ECDSA against a key
  embedded at build time. An unsigned update is accepted **only** when no key was
  embedded (local testing), and then only with a loud warning.
- **sha256-gated download.** The downloaded artifact must match the sha256 in the
  (already signature-verified) manifest, or it is refused.
- **HTTPS required** for the update URL.
- **Downgrade-safe.** The updater never installs a version older than or equal to
  the running one.
- **Atomic swap.** The new binary is staged and verified before an atomic rename
  replaces the installed one; an interrupted update leaves the original intact.

## Out of scope (v1)

- **Delta updates** — the full artifact is re-downloaded (a desktop artifact is
  ~46–80 MB; acceptable for now).
- **Silent auto-apply** — v1 requires an explicit `--update` or an in-app
  confirmation via `Updater.apply()`. There is no unattended background install.
