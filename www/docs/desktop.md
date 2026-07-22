# Desktop Applications

`soli desktop build` packages an app as a single executable that carries its own
database. The user double-clicks it; it starts a private database, opens their
browser, and serves the app locally. No installer, no separate database, no
configuration.

```bash
soli desktop build ./myapp --app-id com.example.myapp
```

That produces one file. Run it and the app opens.

## What the artifact contains

```
[ soli runtime ]
[ manifest        — app identity, versions, checksums ]
[ app.sole        — your application, encrypted ]
[ solidb          — the database binary, compressed ]
[ ref_*.ndjson    — read-only reference data (optional) ]
```

Your application source is **always encrypted**. There is no unencrypted desktop
build, because that would ship your source in the clear to every user.

## Requirements

The application must be unlockable at launch, which means a key. Set either
`SOLI_BUNDLE_KEY` (the key itself) or `SOLI_BUNDLE_AUTH_URL` (a key server that
returns it) at both build time and run time — the same resolution chain
`soli build --encrypt` uses. See [Deployment](/docs/development-tools/deploy).

**There is no offline fallback.** The key is never written to disk, so an app
that cannot reach its key server does not start. That is the trade for being
able to revoke an installation.

## Options

| Flag | Meaning |
|---|---|
| `--app-id <id>` | Reverse-DNS identity, e.g. `com.example.myapp`. **Required.** Determines where the user's data lives, so changing it between releases orphans their existing data. |
| `--name <name>` | Display name. Defaults to the source directory's name. |
| `--output <path>` | Artifact path. Defaults to `<app>` or `<app>-<target>`. |
| `--target <t>` | Cross-build: `linux-amd64`, `linux-arm64`, `darwin-amd64`, `darwin-arm64`, `windows-amd64`. |
| `--solidb <path>` | Embed a locally built database binary instead of downloading the published release. |
| `--solidb-version <v>` | Database release to download. Pinned by default. |
| `--seed <dir>` | Directory of `<collection>.ndjson` reference data. |
| `--protect` | Compile the app to a binary AST, stripping source and comments. |

## Shipping reference data

Read-only data your app needs — a country list, a price table — can travel with
the artifact:

```bash
soli desktop build ./myapp --app-id com.example.myapp --seed ./seed
```

Each `seed/<name>.ndjson` is one JSON document per line. It is imported at first
launch, and re-imported only when the content changes.

**Seed collections must be named `ref_*`.** The build fails otherwise, and the
reason matters: importing **replaces a collection wholesale**. Without the
prefix, shipping a `users.ndjson` would silently destroy the user's own `users`
data on their next launch. The prefix keeps the two namespaces disjoint by
construction rather than by review.

```
seed/
  ref_countries.ndjson     ✓
  ref_currencies.ndjson    ✓
  users.ndjson             ✗ build fails — could collide with your model
```

## Where user data lives

| | Data (persists) | Cache (safe to delete) |
|---|---|---|
| Linux | `$XDG_DATA_HOME/<app-id>/db` | `$XDG_CACHE_HOME/<app-id>/bin` |
| macOS | `~/Library/Application Support/<app-id>/db` | `~/Library/Caches/<app-id>/bin` |
| Windows | `%LOCALAPPDATA%\<app-id>\db` | `%LOCALAPPDATA%\<app-id>\cache\bin` |

The database binary is cached and reused across launches, re-verified against
its checksum each time. Deleting the cache costs one extraction, not data.

A second launch of the same app is refused while the first is running — two
servers over one database directory would fail deep inside the storage engine
with an unhelpful error.

## Protecting data at rest

The database files sit in the user's own directory. Mark sensitive fields so
they are encrypted with the key fetched at launch:

```soli
class Customer < Model
  encrypts :tax_id, :bank_account
end
```

Encrypted fields **cannot be queried by value** — each write uses a fresh nonce,
so the ciphertext differs every time. Encrypt what must stay confidential, not
what you filter on.

Fields you do not mark are stored in plaintext and `strings` will find them.

## What this protects against, and what it does not

Worth being precise, because "encrypted desktop app" promises more than any
local software can deliver.

**It does protect against:** a copied artifact being useful to someone without
an authorized key; casual inspection of your source; other local processes
driving the app's API or its database; and it lets you revoke an installation.

**It does not protect against the machine's owner.** The key reaches the process
environment at launch, and on their own machine a user can read it — from
`/proc/<pid>/environ`, from a debugger, from process memory. Assume a motivated
user recovers it. Once they have, it decrypts that data offline, forever.

Treat this as **licensing-grade protection**: it stops copying and enables
revocation. It is not confidentiality against your user. If you hold data whose
disclosure to that user would be a breach, keep it on your server behind an
authenticated API and cache only derived, non-sensitive results locally.

## The local port

The app binds `127.0.0.1` on a port the OS assigns, so it never appears on the
network and never collides with another app.

Loopback keeps the network out but not the machine — every process running as
that user can reach the port. So the browser is launched with a single-use token
that it exchanges once for a session cookie; everything else gets `403`. The
token expires in 60 seconds and a wrong guess burns it.

One limitation to know: cookies are not port-scoped, so another local server in
the same browser profile could read the session cookie. Closing that needs
loopback HTTPS with a per-launch certificate, which means touching the user's
trust store. What the gate does close is a non-browser local process driving
your API.

## Embedding in a native shell

The artifact opens the app in a chrome-less browser window. If you wrap it in a
native shell of your own — a Cocoa/WebView app, an Electron-style container —
you want your window, not that one. Set `SOLI_DESKTOP_NO_WINDOW=1` and the
server opens nothing:

```bash
SOLI_DESKTOP_NO_WINDOW=1 ./myapp
```

The launch URL is still printed, on its own indented line, and your shell needs
it: it carries the single-use token described above, so pointing a web view at
`http://127.0.0.1:<port>/` directly gets a `403`. Read the child's stdout, take
the first `http://127.0.0.1:` line, and load that.

Send `SIGTERM` when your window closes so the database and the decrypted tree
are cleaned up in order — see [Stopping](#stopping).

## Cross-building

`--target` downloads a published runtime and database for that platform and
verifies both against their checksums. It does not compile anything, so you can
build every target from one machine:

```bash
soli desktop build ./myapp --app-id com.example.myapp --target darwin-arm64
soli desktop build ./myapp --app-id com.example.myapp --target windows-amd64
```

Two signing notes, both of which bite at distribution rather than build time:

- **macOS** artifacts built elsewhere have an invalid signature and Apple
  Silicon refuses to run them. Re-sign on a Mac with `codesign --force -s -`, or
  use `rcodesign` from any platform.
- **Windows** artifacts are unsigned, so SmartScreen shows "Windows protected
  your PC" with *Run anyway* hidden behind *More info*. Sign the **finished**
  file — never before packaging, which would invalidate the signature:
  `signtool sign /fd sha256 /tr <timestamp-url> /td sha256 myapp.exe`.

## Stopping

Closing the terminal, or `SIGTERM`, shuts the app down in order: the decrypted
application tree is removed first, then the database is closed cleanly. A clean
close matters — an abrupt kill makes the storage engine replay its write-ahead
log on the next launch, which is slow and looks like corruption.

A hard `kill -9` skips this; the leftover directory is swept at the next launch.

## Size

A typical artifact is 70–80 MB, mostly the database binary (stored compressed,
roughly a third of its size). It contains everything: runtime, application,
database and reference data.
