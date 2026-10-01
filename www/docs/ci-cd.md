# CI/CD with GitHub Actions

One workflow, `ci.yml`, covers a Soli app on GitHub, in two jobs:

- **`verify`** runs on every push and pull request: formatting, lint, the app boots and answers
  `/up`, and the specs pass against a real SoliDB.
- **`deploy`** runs on `main` once `verify` is green: it copies the app to the server, runs the
  migrations and lets soli-proxy switch traffic only when the new version answers.

A second, optional workflow, `release.yml`, publishes **executables** on a tag: the app built with
`soli build --standalone`, or a script built with `soli build tool.sl`, for each platform.

Everything below runs on GitHub's `ubuntu-latest` runners and needs no third-party action
besides `actions/checkout` and `actions/upload-artifact`.

## Pin the versions

`install.sh` only installs the latest release, which changes under you. Download the release
you run in production instead, and check it:

```yaml
env:
  SOLI_VERSION: "2.10.3"     # the soli your server runs
  SOLIDB_VERSION: "2.1.0"    # the SoliDB your specs run against
```

```yaml
- name: Install soli and SoliDB
  run: |
    set -euo pipefail
    fetch() {
      local name="$1" repo="$2" want="$3" tmp got
      tmp="$(mktemp -d)"
      curl -fsSL "https://github.com/$repo/releases/download/v$want/$name-linux-amd64.tar.gz" \
        -o "$tmp/pkg.tar.gz"
      tar -xzf "$tmp/pkg.tar.gz" -C "$tmp"
      sudo install -m 0755 "$(find "$tmp" -type f -name "$name" -perm -u+x | head -1)" \
        "/usr/local/bin/$name"
      got="$("$name" --version | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1)"
      [ "$got" = "$want" ] || { echo "$name: $got installed, $want expected" >&2; exit 1; }
    }
    fetch soli   solisoft/soli_lang "$SOLI_VERSION"
    fetch solidb solisoft/solidb    "$SOLIDB_VERSION"
```

Bump `SOLI_VERSION` in the same commit that upgrades the server, so CI always tests the soli
that will serve the code.

## Continuous integration

```yaml
# .github/workflows/ci.yml
name: ci

on:
  push:
  pull_request:

# A newer push to a branch makes its running check pointless, but on main the
# run may be deploying: never cancel that halfway through an rsync.
concurrency:
  group: ci-${{ github.ref }}
  cancel-in-progress: ${{ github.ref != 'refs/heads/main' }}

env:
  SOLI_VERSION: "2.10.3"
  SOLIDB_VERSION: "2.1.0"

jobs:
  verify:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5

      - name: Install soli and SoliDB
        run: |
          # … the `fetch` step from "Pin the versions" above …

      - name: Formatting
        run: soli fmt --check

      - name: Lint
        run: soli lint

      # Booting compiles every handler and view: a syntax error, a missing view or
      # a broken route fails here, not in production.
      - name: The app boots and answers /up
        run: |
          set -euo pipefail
          soli serve . --port 19555 --workers 1 > /tmp/boot.log 2>&1 &
          pid=$!
          code=000
          for i in $(seq 1 45); do
            code="$(curl -s -o /dev/null -w '%{http_code}' -m 2 http://127.0.0.1:19555/up || echo 000)"
            [ "$code" = "200" ] && break
            kill -0 $pid 2>/dev/null || break
            sleep 1
          done
          kill $pid 2>/dev/null || true
          if [ "$code" != "200" ]; then
            echo "/up answered $code" >&2
            tail -40 /tmp/boot.log >&2
            exit 1
          fi

      # SoliDB creates its admin account on first start from
      # SOLIDB_ADMIN_PASSWORD. Use the credentials .env.test names, or every
      # spec fails with a 401.
      - name: Start SoliDB
        run: |
          set -euo pipefail
          val() { grep -E "^$1=" .env.test | head -1 | cut -d= -f2- | tr -d '"'; }
          export SOLIDB_ADMIN_PASSWORD="$(val SOLIDB_PASSWORD)"
          mkdir -p "$RUNNER_TEMP/solidb-data"
          solidb --host 127.0.0.1 --port 6745 --data-dir "$RUNNER_TEMP/solidb-data" \
            > "$RUNNER_TEMP/solidb.log" 2>&1 &
          for i in $(seq 1 40); do
            code="$(curl -s -o /dev/null -w '%{http_code}' -m 2 http://127.0.0.1:6745/_api/databases || true)"
            [ "${code:-000}" != "000" ] && exit 0
            sleep 0.5
          done
          tail -40 "$RUNNER_TEMP/solidb.log" >&2
          exit 1

      - name: Specs
        run: soli test --coverage=xml --coverage-min=80

      - name: Coverage report
        if: always()
        uses: actions/upload-artifact@v4
        with:
          name: coverage
          path: coverage/
          if-no-files-found: ignore
```

What each step relies on:

- **`soli fmt --check`** rewrites nothing; it prints `would reformat: <path>` for each file that
  would change and exits non-zero. A new app from `soli new` passes it from its first commit.
- **`soli lint`** exits non-zero on any finding. See [Linting](/docs/development-tools/linting)
  for the rules.
- **`/up` is built in.** Every app answers it, with 200 once its session store is ready. It is
  also what soli-proxy probes before switching traffic, so this step checks the same thing the
  deploy will.
- **`soli test` reads `.env.test`.** Commit it with credentials for the runner's SoliDB only.
  The runner creates the test database, runs `db/migrations` against it, gives each worker a
  copy of the schema, and drops them all at the end; nothing has to be prepared. Leave
  `SOLI_TEST_SOLIDB_HOST` unset in CI.
- **`--coverage=xml`** writes Cobertura to `coverage/cobertura.xml`, which most coverage
  services read; `--coverage=html` writes `coverage/index.html`. `--coverage-min=80` fails the
  job below 80%. See [Testing](/docs/testing).

An app whose specs need no database can drop the SoliDB lines from the install step and the
*Start SoliDB* step.

## Continuous deployment

The deploy runs as a second job of the same workflow, so it starts only when `verify` is green,
and only on `main`. Add it to `ci.yml`:

```yaml
  deploy:
    needs: verify
    if: github.ref == 'refs/heads/main' && github.event_name == 'push'
    runs-on: ubuntu-latest
    # Two merges close together must not interleave in the middle of an rsync, and
    # a deploy that has started is never cancelled: that would leave half a tree.
    concurrency:
      group: deploy
      cancel-in-progress: false
    environment:
      name: production
      url: https://${{ vars.DEPLOY_DOMAIN }}
    env:
      HOST: ${{ secrets.DEPLOY_HOST }}
      DOMAIN: ${{ vars.DEPLOY_DOMAIN }}
      SITE: /srv/sites/${{ vars.DEPLOY_DOMAIN }}
    steps:
      - uses: actions/checkout@v5

      - name: Prepare SSH
        run: |
          install -d -m 700 ~/.ssh
          printf '%s\n' "${{ secrets.SSH_DEPLOY_KEY }}" > ~/.ssh/id_ed25519
          printf '%s\n' "${{ secrets.SSH_KNOWN_HOSTS }}" > ~/.ssh/known_hosts
          chmod 600 ~/.ssh/id_ed25519 ~/.ssh/known_hosts

      # The server's own files stay the server's: its secrets and the proxy's files.
      - name: Copy the code
        run: |
          rsync -rlz --delete-delay --delay-updates \
            --exclude='.git/' --exclude='.github/' \
            --exclude='.env' --exclude='.env.*' \
            --exclude='app.infos' --exclude='restart.txt' \
            --exclude='tmp/' --exclude='coverage/' --exclude='node_modules/' \
            ./ "deploy@$HOST:$SITE/"

      - name: Migrations
        run: ssh "deploy@$HOST" "cd '$SITE' && soli db:migrate up"

      # Starts the idle slot, waits for its /up, moves traffic, stops the old slot.
      # A slot that never answers is stopped, the old one keeps serving, and this
      # step fails with the reason.
      - name: Switch traffic
        run: ssh "deploy@$HOST" "soli-proxy deploy -c /etc/soli-proxy/proxy.conf '$DOMAIN'"

      # End to end: through the proxy, with the real hostname and certificate.
      - name: The site answers
        run: |
          for i in $(seq 1 10); do
            code="$(curl -s -o /dev/null -w '%{http_code}' -m 8 "https://$DOMAIN/up")"
            [ "$code" = "200" ] && exit 0
            sleep 3
          done
          exit 1
```

This assumes a server already running soli-proxy, with the app's folder under
`/srv/sites/<domain>` holding its `.env` and `app.infos`, and a `deploy` user that can run
`soli-proxy deploy`. Setting that up once — the proxy, its systemd unit, the folder, the SSH
key — is walked through in
[Deploying a Soli App Behind soli-proxy with GitHub Actions](/docs/blog/deploy-with-soli-proxy).

**Why `soli-proxy deploy` and not `touch restart.txt`.** Touching `restart.txt` also starts a
blue/green switch, but nothing reports back: if the new slot cannot start, the old one keeps
serving and the workflow stays green. `soli-proxy deploy` waits for the outcome and exits
non-zero.

**Rolling back** is a revert of the commit on `main`: the previous code goes through the same
checks and the same switch.

### Secrets and variables

Set them under *Settings → Secrets and variables → Actions* (secrets) and the `production`
environment (variables):

| Name | Kind | What it holds |
|------|------|---------------|
| `DEPLOY_HOST` | secret | the server's address |
| `SSH_DEPLOY_KEY` | secret | the private key of the `deploy` user, without a passphrase |
| `SSH_KNOWN_HOSTS` | secret | the output of `ssh-keyscan <host>`, so the first connection is verified |
| `DEPLOY_DOMAIN` | variable | the domain the app answers on, also its folder name |

The app's `.env` never goes through GitHub: it lives on the server, and rsync leaves it alone.

### With `soli deploy` instead

If your servers pull a git checkout rather than receive files, the deploy job can run
[`soli deploy`](/docs/development-tools/deploy) from the runner: it reads `deploy.toml`, pulls
on every server in parallel, migrates on the first, and triggers the same soli-proxy switch. It
needs the SSH key loaded into an agent:

```yaml
      - name: Deploy
        run: |
          eval "$(ssh-agent -s)"
          ssh-add - <<< "${{ secrets.SSH_DEPLOY_KEY }}"
          soli deploy
```

## Publishing executables on a tag

`soli build --standalone` turns the app into one executable with the runtime inside, and
`soli build tool.sl` does the same for a script (see [Deployment](/docs/development-tools/deploy)).
`--target` builds for another platform from the same Linux runner: it downloads the soli
runtime of that platform, at the same version, from the soli release.

```yaml
# .github/workflows/release.yml
name: release

on:
  push:
    tags: ["v*"]

permissions:
  contents: write   # to create the release and upload to it

env:
  SOLI_VERSION: "2.10.3"

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5

      - name: Install soli
        run: |
          # … the `fetch` step from "Pin the versions" above, soli only …

      - name: Build every platform
        run: |
          set -euo pipefail
          mkdir -p dist
          for target in linux-amd64 linux-arm64 darwin-amd64 darwin-arm64 windows-amd64; do
            soli build . --standalone --target "$target" -o "dist/myapp-$target"
            # A script instead of an app:
            # soli build tool.sl --target "$target" -o "dist/tool-$target"
          done
          ls -la dist

      - name: Publish the release
        env:
          GH_TOKEN: ${{ github.token }}
        run: gh release create "$GITHUB_REF_NAME" dist/* --generate-notes
```

A Windows target gets `.exe` appended. A darwin executable is re-signed ad hoc when it is built
on a Mac; built on Linux, sign it on a Mac before distributing it, or Apple Silicon refuses to
start it. Each executable carries the whole runtime, so expect 40 to 80 MB per file.

## See also

- [Deployment](/docs/development-tools/deploy) — `soli deploy`, bundles, standalone and script
  executables.
- [Testing](/docs/testing) — `.env.test`, parallel workers, coverage.
- [Deploying a Soli App Behind soli-proxy with GitHub Actions](/docs/blog/deploy-with-soli-proxy) —
  the server side, from a bare machine.
