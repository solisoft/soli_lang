# Deployment

Soli ships three ways to put an app on a server, in increasing order of independence from the
machine you deploy to:

| Command | What lands on the host | Rollback |
|---------|------------------------|----------|
| `soli deploy` | a git working tree, updated in place | redeploy the previous commit |
| `soli build` | a single `.soli` bundle + the `soli` binary | replace the file |
| `soli build --standalone` | one native executable, runtime included | replace the file |

`soli build tool.sl` is a different use of the same command: it turns a script, not an app, into
one executable — see [Script executables](#script-executables).

`soli env` is the fourth command in the family, but it deploys *branches* rather than releases —
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

`soli deploy` is Unix-only — it is built on ssh2. Reading `deploy.toml` is not, so `soli env`
works everywhere.

### Assets during a deploy

In production the server snapshots every `.css` and `.js` under `public/` into memory at boot.
New asset bytes landing on disk before the binary restarts do not affect the running process — it
keeps serving its snapshot, so in-flight HTML never references an asset version the process has
already replaced. The next start reloads from disk. See [Production mode](live-reload.md).

Other files under `public/` are read from disk on request. In production a path that was not a file
a moment ago — every dynamic route is checked this way first — is taken on trust for one second,
so a file written under `public/` while the server runs (an upload saved there, say) is served at
most a second after a request for that same path found nothing.

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

### Pinned versions in production

A project that pins its interpreter (`soli_version = "=2.0.3"`, see
[Modules & Packages](/docs/language/modules)) switches to that version wherever it runs, including
under soli-proxy: the proxy starts an app with the app directory as the working directory, so the
pin resolves the same way it does on your machine.

Two things to get right on a server:

- **Provision the toolchain ahead of time, not at start-up.** The proxy gives a new instance 30
  seconds to pass its health check. A first start after changing a pin would spend part of that
  window downloading, and a slow link can push it over — the deploy then fails and succeeds on the
  retry, once the cache is warm. Fetch the version during the deploy step instead, or pre-populate
  the cache directory.
- **The cache must be readable by the user the app runs as.** It lives under
  `$XDG_CACHE_HOME/soli/runtimes` or `~/.cache/soli/runtimes`, so `HOME` in the app's environment
  has to belong to that user. A shared directory pointed at by `XDG_CACHE_HOME` works well when
  several apps run as different users.

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

## Script executables

`soli build` given a `.sl` file, rather than an app folder, builds one executable that **runs that
script** — a command-line tool, not a server. The target machine needs no `soli` install.

```bash
soli build tool.sl
#   Building tool from tool.sl...
#     Built tool (79.0 MB, linux-x86_64, runs on the VM, or the tree-walker if it needs it)

./tool 32
```

```soli
# tool.sl
import "./lib/math.sl"   # export def fib(n: Int) -> Int …

args = System.argv
if args.length == 0
  print("usage: tool N")
else
  n = args[0].to_i()
  print("fib(#{n}) = #{fib(n)}")
end
```

Every argument after the program name reaches the script as [`System.argv`](builtins.md#systemargv),
an Array of Strings. Run through the CLI instead, the script gets what follows `--`:
`soli tool.sl -- 32` (also `soli -e "…" -- a b`).

| Option | Effect |
|--------|--------|
| `-o`, `--output FILE` | Output path. Default: the script's name without `.sl`, plus `-<target>` for a cross build and `.exe` for Windows. |
| `--target T` | Embed another platform's runtime, as for [`--standalone`](#cross-platform-builds): `linux-amd64`, `linux-arm64`, `darwin-amd64`, `darwin-arm64`, `windows-amd64`. |
| `--vm` | Always run the script on the bytecode VM, with no fallback to the tree-walker. |
| `--tree` | Always run the script on the tree-walking interpreter. |
| `--no-type-check` | Skip the type check at build time. |

What the executable contains, and what it does not:

- **The resolved program, not the sources.** Imports are resolved and the program is type-checked
  when you build, so an error `soli tool.sl` would report stops the build. What ships is the
  program as a serialized AST. Paths in it are relative to the script, so the build machine's
  directory layout is not inside the file.
- **The whole Soli runtime**, so the file is about the size of the `soli` binary (~80 MB) however
  small the script.
- **Native kernels are compiled when the executable starts**, on the machine it runs on, like
  `soli tool.sl` does (see [Native Kernels](native-kernels.md)) — load-time compilation, not an
  ahead-of-time object file.
- **The engine is chosen as for `soli tool.sl`**: the bytecode VM, unless the script needs the
  tree-walker (see [Configuration → Script Engine](configuration.md#script-engine)). The choice is
  made when the executable starts, from the program it carries; `--vm` or `--tree` at build time
  fixes it instead, and `SOLI_ENGINE` does not change it. `SOLI_ENGINE_LOG=1` prints it.
- **It starts as fast as `soli` does**: one that prints a line runs in about 3 ms on Linux.
- It exits `0`, or `70` after printing `Error: …` to stderr, like `soli tool.sl`.

`--encrypt`, `--protect` and `--update-url` / `--update-key` apply to app bundles and are refused
for a script. The macOS re-signing note and the "do not post-process the artifact" rule above apply
here too.

## See also

- [Auto-update](auto-update.md) — signed over-the-air updates for shipped binaries.
- [Static & Markdown server](static-server.md) — `soli serve` on a folder that is not an app.
- [Configuration](configuration.md) — the environment variables a deployed app reads.
- [Migrations](migrations.md) — what `soli db:migrate up` runs during a deploy.
