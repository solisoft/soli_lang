# Deployment

Soli ships four ways to put an app on a server, in increasing order of independence from the
machine you deploy to:

| Command | What lands on the host | Rollback |
|---------|------------------------|----------|
| `soli deploy` | a git working tree, updated in place | redeploy the previous commit |
| `soli cloud` | an immutable release directory, behind a symlink | move the symlink |
| `soli build` | a single `.soli` bundle + the `soli` binary | replace the file |
| `soli build --standalone` | one native executable, runtime included | replace the file |

`soli env` is the fifth command in the family, but it deploys *branches* rather than releases —
see [Preview environments](#preview-environments).

## `soli deploy` — sync a working tree

Create a `deploy.toml` in your project root:

```toml
git_url    = "https://github.com/your-org/your-project.git"
git_branch = "main"
git_folder = "www/"

[[servers]]
name      = "prod-1"
username  = "deploy"
ip        = "192.168.1.100"
folder    = "/var/www/myapp"
api_key   = "your-api-key"
proxy_url = "https://proxy.example.com"

[[servers]]
name      = "prod-2"
username  = "deploy"
ip        = "192.168.1.101"
folder    = "/var/www/myapp"
api_key   = "your-api-key"
proxy_url = "https://proxy.example.com"
```

Then:

```bash
soli deploy                          # from the current directory
soli deploy --folder /path/to/app    # or -f
```

### Configuration

Global keys:

| Key | Meaning |
|-----|---------|
| `git_url` | repository URL (required) |
| `git_branch` | branch to deploy (default `main`) |
| `git_folder` | subfolder to deploy, e.g. `www/` (default `/`) |

Per-server keys, under `[[servers]]`:

| Key | Meaning |
|-----|---------|
| `name` | identifier used in the log output |
| `username` | SSH username |
| `ip` | server address |
| `folder` | deployment path on the server |
| `api_key` | soli-proxy API key |
| `proxy_url` | soli-proxy base URL |

### The flow

1. **Sync code — all servers in parallel.** SSH to `username@ip`, then `git clone` on the first
   deploy or `git pull` afterwards.
2. **Migrations — first server only.** `soli db:migrate up`, on one host, so two servers cannot
   race the same migration.
3. **Deploy — all servers in parallel.** `POST` to the soli-proxy deploy API, which health-checks
   the new slot and switches traffic.

### Requirements

- SSH key-based authentication, with the key loaded into `ssh-agent`.
- The `soli` binary on the target's `PATH`.
- soli-proxy running on each server, with matching API keys.

`soli deploy` is Unix-only — it is built on ssh2. Reading `deploy.toml` is not, so `soli cloud`
and `soli env` work everywhere.

### Assets during a deploy

In production the server snapshots every `.css` and `.js` under `public/` into memory at boot.
New asset bytes landing on disk before the binary restarts do not affect the running process — it
keeps serving its snapshot, so in-flight HTML never references an asset version the process has
already replaced. The next start reloads from disk. See [Production mode](live-reload.md).

## Immutable releases — `soli cloud`

`soli deploy` updates a working tree in place. `soli cloud` does the opposite: it builds an
artifact, lands it in a directory that is never modified again, and moves a symlink. Rolling back
is repointing that symlink — no rebuild, and the bytes it returns to are provably the bytes that
were serving before.

```bash
soli cloud deploy --domain crm.example.com   # build, ship, health-gate, alias
soli cloud releases                          # what is on the host; * marks live
soli cloud rollback                          # back one release
soli cloud rollback --to 20260801T200000Z-a3f21c9
soli cloud deploy --dry-run                  # print the plan, change nothing
```

Servers come from the same `deploy.toml` the other commands read — a second file describing the
same hosts is a second thing to keep in sync. The proxy admin key comes from
`SOLI_PROXY_API_KEY`.

### The layout

```
releases/<app>/20260801T200000Z-a3f21c9/   a build, never modified after it lands
releases/<app>/20260801T211909Z-b7e0d31/
sites/<app>  ->  releases/<app>/20260801T211909Z-b7e0d31
```

The release id is a UTC timestamp then a short commit, in that order, so lexical sort *is*
chronological sort. That is what answers "the previous release", and it has to stay right on the
day two deploys land in the same minute — a timestamp alone collides, a SHA alone does not sort
and repeats when the same commit is redeployed.

Releases live *beside* `sites/`, never inside it: inside, the proxy would discover every past
release as an app and try to run all of them.

### The order is the product

```
mkdir     releases/<app>/<id>
upload    .soli -> releases/<app>/<id>        nothing points at it yet
repoint   sites/<app> -> releases/<app>/<id>  ln -sfn, atomic
deploy    <app> (blue/green, health-gated)
health    https://<domain>/up within 90s      old slot still serving
alias     <domain> -> <app>                   traffic moves here
prune     oldest releases, never the live one
```

- **Upload before repoint** — a transfer that dies half way leaves an unused directory, not a live
  symlink pointing at half an app.
- **Repoint before deploy** — the proxy reads the app from `sites/<app>`. Deploying first would
  start the release that is already live, and report success.
- **Health before alias** — blue/green keeps the old slot serving until the new one answers.
  Moving the alias first sends real traffic at a release that may still be starting.
- **Prune last, never the live one** — a deploy that pruned first would have thrown away what it
  needs to roll back to. Pruning also skips the live release explicitly, because after a few
  rollbacks it can fall outside the newest five.

### A failed deploy is not rolled back for you

Up to and including the upload, a failure is invisible — retry it. From the repoint onward the
deployment is live, and the error names the release that is currently serving plus the exact
command to go back.

It stops there on purpose. An automatic rollback in the middle of a half-applied change is a
second uncontrolled change on top of the first, at the moment when least is known about what is
wrong.

`--dry-run` prints the plan and changes nothing — and it is the *same* plan a real deploy
executes, not a second description of it. It still reads the host, because a plan computed from an
invented view is a guess rather than a dry run; if it cannot reach the host it says so before
printing.

## Preview environments

`soli env` gives every branch its own running environment: a git worktree, its own subdomain, and
its own SoliDB database created from your migrations and seeds. Soli Proxy supplies the runtime —
port allocation, blue/green slots, health gating on `/up`, TLS — so an environment is just a site
directory it discovers.

```bash
soli env up --branch feat/cart      # create it
soli env list                       # what is running
soli env url feat/cart              # print the URL
soli env down feat/cart             # stop, unlink, remove worktree, drop the database

soli env up --branch feat/cart --server prod-1   # same, on a remote proxy
```

### Configuration

Add a `[preview]` section to `deploy.toml`. Every key is optional.

```toml
[preview]
domain_base       = "dev.example.com"    # for --server environments
local_domain_base = "dev.example.test"   # for local ones
sites_dir         = "~/workspace/proxy/sites"
worktrees_dir     = "~/.soli/previews"
env_template      = ".env.preview.example"
build_command     = ""                   # optional; Soli needs no npm step
seed              = true
```

`build_command` is for projects that keep their own asset toolchain. A
`soli new` app has no `package.json` and compiles its Tailwind with the
binary Soli ships, so leave it empty unless you added a build step yourself
— `npm ci` on a project without a lockfile fails the preview build.

### The env template must not be your production `.env`

`env_template` is copied into each worktree and then overlaid with the generated `APP_ENV`,
`SOLIDB_DATABASE`, `SOLI_SESSION_DRIVER` and URL values, so a `SOLIDB_DATABASE` in the template can
never survive into a preview. A template carrying production database credentials would let a
preview migrate and seed straight into production — the one mistake here you cannot undo. Commit a
credential-free template. A missing template is a hard error naming the file; it never falls back
to the app's own `.env`.

The `.example` suffix is load-bearing too. The generated `.env` sets `APP_ENV=preview`, and Soli
layers `.env.preview` *over* `.env` with override; a template named `.env.preview` would be checked
out by git into every worktree and silently win. `soli env up` refuses to start when it finds one,
and explains why.

Preview sessions are pointed at the SoliDB driver so they land in the branch's own database.
SoliKV has no namespaces and its session keys carry a fixed global prefix, so previews sharing one
SoliKV would share sessions. Cache keys need no change — they are already scoped by
`SOLIDB_DATABASE`.

### Domains are flat

A preview is reachable at `<branch>--<app>.<base>` — for example
`feat-cart--demo.dev.example.com`. The double dash is deliberate: DNS wildcards and the proxy's SNI
resolver both match exactly one label deep, so a flat name means a single `*.dev.example.com`
record and a single wildcard certificate cover every app and every branch. A nested
`<branch>.<app>.<base>` scheme would need a record and a certificate per app.

Branch names are sanitised into a DNS label: lowercased, illegal characters replaced, runs
collapsed. Anything over 30 characters keeps a 24-character prefix plus 6 hex of the full name's
SHA-256, so two long `task/…` branches sharing a prefix cannot collapse onto one domain. If
`<slug>--<app>` would exceed the 63-character DNS label limit the slug is shortened, never the app
name, so the domain still says what it belongs to.

### Teardown

`soli env down` reverses all four steps, and the order matters: the proxy is asked to stop the app
*before* the symlink is removed, because it only drops vanished apps from its map — unlinking first
leaves an orphan process holding its allocated ports. The database name and host are read back from
the worktree's generated `.env` rather than re-derived, so a teardown long after creation still
targets the right database. Failures are collected and all reported, since a partial teardown is
exactly when you need to know what survived.

### Running under the proxy

Pass `--strict-port` in the app's `start_script`. By default `soli serve` scans upward for a free
port when the one it was given is taken, which is helpful interactively and wrong under a
supervisor: the proxy health-checks the port it assigned, so an app that quietly moved to
`port + 1` reads as unhealthy and is quarantined after three such failures — a port race presenting
as a broken deployment. `--strict-port` exits instead.

## Bundle deployment

Bundle the application into a single `.soli` file. No source files on the server — only the `soli`
binary and the bundle.

```bash
soli build my_app
scp my_app.soli deploy@server:/opt/my_app/
ssh deploy@server "soli serve /opt/my_app/my_app.soli"
```

`soli deploy` can do the copying, with `mode = "bundle"`:

```toml
mode          = "bundle"
bundle_source = "./my_app.soli"

[[servers]]
name      = "prod-1"
username  = "deploy"
ip        = "192.168.1.100"
folder    = "/opt/myapp"
proxy_url = "https://proxy.example.com"
```

`soli build` collects every `.sl`, `.slv`, `.yml`, `.css` and `.js` into the bundle. `soli serve
app.soli` extracts it to `/tmp/soli_PID` and boots normally. On the proxy, set
`start_script = "soli serve /opt/myapp/my_app.soli --port $PORT --workers $WORKERS"` in the app's
`app.infos`. Run migrations before bundling — there is no auto-migration in bundle mode.

**Secrets stay out of the bundle.** Dotfiles are never bundled, so ship `.env` separately and drop
it *next to* the `.soli` file — `soli serve app.soli` loads `.env` (and `.env.{APP_ENV}`) from the
bundle's directory before boot. Variables already set in the process environment win. Binary assets
(images, fonts) under `public/` are not bundled either: deploy those alongside, or serve them from
a CDN.

## Encrypted and protected bundles

When you deploy to servers you do not fully control, encrypt the bundle so the source cannot be
copied off disk, and fetch the decryption key from a key server you own — revoking it there is a
remote kill-switch.

| Flag | What it does |
|------|--------------|
| `--encrypt` | Wraps the whole bundle in AES-256-GCM. The `.soli` on disk is ciphertext; the key is needed to boot. |
| `--protect` | Implies `--encrypt`, and replaces every `.sl` source with its compiled binary AST — after decryption there is still no readable source (comments and formatting are gone; identifiers and string literals remain, as in any bytecode). |

```bash
# The key is read from the environment — never passed as an argument.
export SOLI_BUNDLE_KEY="a-long-random-secret"

soli build my_app --protect
#   ✓ Bundle written to my_app.soli (1.9 KB) (protected: binary AST, encrypted)

soli serve my_app.soli --port 8080
```

`--encrypt` and `--protect` are on/off flags that take **no value**, and `--protect` already
implies `--encrypt` — you never pass both. The folder and the flags may appear in any order. There
is **no `--key` argument**: a key on the command line would leak into shell history and the process
list, so it always comes from the environment.

### Where the key comes from

Resolved in this order, at both build and serve time:

1. `SOLI_BUNDLE_KEY` — the key material itself (handy for local testing).
2. `SOLI_BUNDLE_AUTH_URL` — a URL you expose. Soli issues a `GET`, sending `SOLI_BUNDLE_API_KEY`
   (if set) as an `x-api-key` header; the response body is the key material, in any encoding — it
   is hashed to a 256-bit key. This is the revocable path.

Both can live in the `.env` beside the `.soli` file, loaded before decryption, so the deployed
host only ever carries its own API key:

```bash
SOLI_BUNDLE_AUTH_URL=https://keys.example.com/my_app
SOLI_BUNDLE_API_KEY=srv-7f3c...      # this host's identity; revoke it to lock the app out
```

Delete or disable that entry on your key server and the next boot fails with a clear error — a
decommissioned or stolen host stops working. A wrong or rotated key fails the same way.

### What this protects, and what it does not

Encryption protects your source against *casual copying*: a hosting provider's backup, a leaked
`.soli`, or a co-tenant browsing the disk sees only ciphertext, and decrypted files live in a
private RAM-backed directory (`/dev/shm`, mode `0700`) removed on shutdown — never on persistent
disk. `--protect` raises the cost of reconstructing the source, like shipping `.pyc` instead of
`.py`. Because the key is fetched at boot, revoking it is a kill-switch.

It does **not** protect against an attacker with root on the running server — they can read the key
from the process environment or the decrypted files from RAM. If your threat model includes a
hostile host, do not deploy the source there at all.

Operational notes:

- Decrypted bundles extract to `/dev/shm`. On a system without it (macOS) the boot is **refused**
  rather than silently writing plaintext to disk; `SOLI_BUNDLE_ALLOW_DISK=1` overrides it (temp
  dir, still `0700`).
- A `--protect` bundle is locked to the exact Soli version that built it — the binary AST has no
  cross-version format guarantee. A different `soli` fails with a "rebuild the bundle" error.
- `--protect` does not yet support apps with an `engines/` directory.

## Standalone executables

`--standalone` goes one step further than a `.soli` bundle: it embeds the entire Soli runtime,
producing a single native executable that boots your app directly. The target machine needs no
`soli` install at all. It composes with `--encrypt` and `--protect`, and key resolution works
exactly as above.

```bash
soli build my_app --standalone --protect
#   ✓ Standalone executable written to my_app (41.2 MB, 1.9 KB app bundle, protected: binary AST, encrypted)

scp my_app deploy@server:/opt/my_app/
ssh deploy@server "/opt/my_app/my_app --port 8080 --workers 4"
```

### Cross-platform builds

`--target` selects which platform's runtime to embed — build on your workstation, deploy anywhere.
The matching official release runtime (same version as your `soli`) is downloaded,
**sha256-verified** against the published checksum, cached under `~/.cache/soli/runtimes/`, and
embedded.

```bash
soli build my_app --standalone --protect --target linux-arm64
#   ✓ Standalone executable written to my_app-linux-arm64 ...
```

Supported targets, matching the published release artifacts exactly:

```
linux-amd64   linux-arm64   darwin-amd64   darwin-arm64   windows-amd64
```

Air-gapped or mirrored environments can point `SOLI_RELEASE_BASE_URL` at their own artifact server
(same layout: `{base}/v{version}/soli-{target}.tar.gz` plus `.sha256`). Without `--target` the
running `soli` binary itself is embedded, and no network is needed.

### Running a standalone app

The executable accepts app-oriented flags — `--port`, `--host`, `--workers`, `--dev`, `--version`,
`--help` — and reads its `.env` from the **directory containing the executable**, the same
convention as a `.soli` bundle. For encrypted apps, put `SOLI_BUNDLE_KEY` or
`SOLI_BUNDLE_AUTH_URL` there.

Operational notes:

- The runtime adds a ~40 MB baseline to the artifact regardless of app size.
- The protected-bundle version lock is satisfied by construction — runtime and bundle ship as a
  matched pair.
- Encrypted standalones inherit the RAM-only extraction contract: `/dev/shm`, or
  `SOLI_BUNDLE_ALLOW_DISK=1`.
- **Do not post-process the artifact.** `strip`, `objcopy`, `upx` or re-signing tools that rewrite
  the file destroy the embedded bundle trailer.
- **macOS:** building *on* a Mac ad-hoc re-signs the artifact automatically. A `darwin-arm64`
  artifact cross-built on Linux must be re-signed before Apple Silicon will run it:
  `codesign --force -s - my_app-darwin-arm64`, on a Mac. Use `codesign` specifically — the payload
  rides inside `__LINKEDIT`, and a signer that regenerates that segment (such as `rcodesign`)
  discards it.
- `soli update` does not apply to standalone apps — a runtime fix means rebuilding and redeploying
  the artifact.

## See also

- [Auto-update](auto-update.md) — signed over-the-air updates for shipped binaries.
- [Static & Markdown server](static-server.md) — `soli serve` on a folder that is not an app.
- [Configuration](configuration.md) — the environment variables a deployed app reads.
- [Migrations](migrations.md) — what `soli db:migrate up` runs during a deploy.
