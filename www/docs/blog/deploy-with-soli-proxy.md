# Deploying a Soli App Behind soli-proxy with GitHub Actions

This site is a Soli app, and every push to `main` that touches it ends the same way:
GitHub Actions boots it, copies it to a server, runs its migrations, and asks
soli-proxy to switch traffic to the new version. If the new version fails to come
up, the old one keeps serving. This post sets up the same pipeline for your app,
from a bare Linux server to a green check mark.

You end up with:

- **soli-proxy** in front of everything: HTTPS with Let's Encrypt certificates it
  requests by itself, and blue/green deploys with a health gate.
- **Your app** in a folder named after its domain, started and restarted by the proxy.
- **A workflow** that refuses to deploy an app that does not boot, and checks the
  result through the proxy, with the real hostname, after the switch.

<figure style="margin:1.5rem auto;max-width:1024px;">
  <img src="/images/blog/deploy-with-soli-proxy.svg" width="1024" height="576" alt="A push to main runs two GitHub Actions jobs. The verify job installs a pinned soli, boots the app and asks for /up, then runs lint and the specs. The deploy job rsyncs the code to /srv/sites/myapp.example.com over SSH, runs soli db:migrate up, then runs soli-proxy deploy. On the server, soli-proxy holds two slots for the app: the blue slot keeps serving while the green slot starts and passes its /up health check, then traffic moves to green and blue is stopped. The proxy terminates HTTPS with a Let's Encrypt certificate for myapp.example.com." style="display:block;width:100%;height:auto;border-radius:12px;border:1px solid #30363d;background:#0b0d0f;">
  <figcaption style="text-align:center;color:#8b949e;font-size:0.875rem;margin-top:0.5rem;">Verify on the runner, copy and migrate on the server, then let the proxy move traffic only once the new slot answers.</figcaption>
</figure>

What you need: a Linux server you can `sudo` on, with ports 80 and 443 reachable
from the internet, and a DNS `A` record for your domain pointing at it. The
examples use `myapp.example.com`; replace it everywhere.

## The server, once

### Install the two binaries

Both install scripts take `--system` to install into `/usr/local/bin`, which is on
every user's `PATH`, including the proxy's and your deploy user's over SSH:

```bash
curl -sSL https://raw.githubusercontent.com/solisoft/soli_lang/main/install.sh | sh -s -- --system
curl -sSL https://raw.githubusercontent.com/solisoft/soli-proxy/main/install.sh | sh -s -- --system
soli --version
soli-proxy --version
```

The proxy's install script does not verify a checksum. Later upgrades through
`sudo soli-proxy update` do: it refuses a release without its `.sha256` file.

### A user for the apps

The proxy runs as root, because it binds ports 80 and 443, and starts each app as
an ordinary user. The same user is the one GitHub Actions logs in as:

```bash
sudo useradd --create-home --shell /bin/bash deploy
sudo mkdir -p /etc/soli-proxy /var/lib/soli-proxy /srv/sites
sudo chown deploy:deploy /srv/sites
```

### Configure the proxy

soli-proxy reads two files from one folder: `proxy.conf`, for hand-written routing
rules, and `config.toml`, for everything else. Apps found in the sites folder are
routed without rules, so `proxy.conf` can stay empty:

```bash
sudo touch /etc/soli-proxy/proxy.conf
```

```toml
# /etc/soli-proxy/config.toml
[server]
# Plain HTTP: redirects to HTTPS, and answers Let's Encrypt's challenge.
bind = "0.0.0.0:80"
https_port = 443

[tls]
mode = "letsencrypt"
cache_dir = "/var/lib/soli-proxy/certs"

[letsencrypt]
email = "you@example.com"
staging = false
terms_agreed = true

[admin]
bind = "127.0.0.1:9090"
api_key = "paste the output of: openssl rand -hex 32"

[apps]
default_user = "deploy"
default_group = "deploy"
```

A few things in this file are less optional than they look:

- **`cache_dir` and `terms_agreed` are required.** Leave either out and the proxy
  refuses to start.
- **Certificates are requested over HTTP-01**, so port 80 must be reachable. You do
  not list domains anywhere: the proxy asks for a certificate for each app it finds,
  and renews them when they have less than 30 days left.
- **To try things first, set `staging = true`.** Staging certificates are not trusted
  by browsers, but the rate limits are generous. When you switch to `false`, delete
  `account_credentials.json` from `cache_dir`, or the proxy keeps using the staging
  account.
- **The admin API stays on loopback.** `soli-proxy deploy`, which the workflow runs
  over SSH, talks to the running proxy through it and sends `api_key`. Without a key
  the API is open to every local user, and any of them could deploy or stop your apps.
- **Without `default_user`, apps refuse to start.** A proxy running as root does not
  start an app as root.

The deploy user must be able to read the key, and nobody else:

```bash
sudo chown root:deploy /etc/soli-proxy/config.toml
sudo chmod 640 /etc/soli-proxy/config.toml
```

### Run it under systemd

```ini
# /etc/systemd/system/soli-proxy.service
[Unit]
Description=soli-proxy
After=network.target

[Service]
Type=simple
User=root
WorkingDirectory=/var/lib/soli-proxy
ExecStart=/usr/local/bin/soli-proxy --conf /etc/soli-proxy/proxy.conf --sites-dir /srv/sites
Restart=always
RestartSec=5
Environment="RUST_LOG=info"

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now soli-proxy
journalctl -u soli-proxy -f
```

- **`--conf` takes the path to `proxy.conf`.** `config.toml` is read from the same
  folder. There is no `--config` flag.
- **`WorkingDirectory` matters.** The proxy keeps its state in `run/` relative to it:
  which slot serves each app, the slot ports, and each slot's log in
  `run/logs/<app>/blue.log` and `green.log`.
- **Don't add `-d`.** systemd is the supervisor; a daemonised process would look to
  it like a process that exited.

Open 80 and 443 in your firewall. The admin port, 9090, and the ports the slots
listen on, 20000 to 30000 by default, are only used from the server itself.

## The app's folder

Create the folder the code will land in, named after the domain:

```bash
sudo -u deploy mkdir /srv/sites/myapp.example.com
```

The name is what makes the rest automatic. When the proxy finds a folder with an
`app/` and an `app/models/` inside, it treats it as a Soli app:

- it serves the folder's name as the domain, and requests its certificate;
- it starts `soli serve . --port $PORT --workers $WORKERS` as `deploy`;
- it health-checks `/up`. Every Soli app answers `/up` without a route, and it
  answers 503 until the app's session store is connected, so traffic does not switch
  to a slot that has only half started.

Two files in that folder belong to the server, not to the repository. The workflow
never overwrites either.

**`app.infos`** holds the settings the proxy runs your app with. It can be one line:

```toml
# /srv/sites/myapp.example.com/app.infos
workers = 2
```

Keep `start_script` out of it unless you need a custom command. If you set one,
also set `domain = "myapp.example.com"`: the folder name is only used as the domain
when the proxy detects the app itself. An app with a `start_script` and no
`domain` starts, runs, and receives no traffic.

**`.env`** holds the app's secrets and its database connection: `SOLIDB_HOST`,
`SOLIDB_PASSWORD`, `SOLI_SESSION_SECRET` and the rest. Create it with mode 600,
owned by `deploy`. The database it points at must already be reachable from the
server. Setting one up is a post of its own; see [the database docs](/docs/database/configuration).

You don't need to restart anything when the code first arrives. The proxy watches
`/srv/sites`, notices the new app, starts it, and requests its certificate.

## SSH access for GitHub

Make a key pair that exists only for this. Its private half goes to GitHub, and its
public half goes to the deploy user. On your machine:

```bash
ssh-keygen -t ed25519 -f deploy_key -N "" -C "github-actions@myapp"
scp deploy_key.pub you@myapp.example.com:
ssh-keyscan -t ed25519 myapp.example.com     # the line for SSH_KNOWN_HOSTS
```

On the server:

```bash
sudo install -d -m 700 -o deploy -g deploy /home/deploy/.ssh
sudo tee -a /home/deploy/.ssh/authorized_keys < deploy_key.pub > /dev/null
sudo chown deploy:deploy /home/deploy/.ssh/authorized_keys
sudo chmod 600 /home/deploy/.ssh/authorized_keys
rm deploy_key.pub
```

In the repository's settings, under **Secrets and variables → Actions**, add three
secrets and one variable:

| Name | Kind | Value |
|---|---|---|
| `DEPLOY_HOST` | secret | the server's hostname or IP |
| `SSH_DEPLOY_KEY` | secret | the contents of `deploy_key` (the private half) |
| `SSH_KNOWN_HOSTS` | secret | the `ssh-keyscan` line |
| `DEPLOY_DOMAIN` | variable | `myapp.example.com` |

`SSH_KNOWN_HOSTS` is what lets the workflow check it is talking to your server. The
alternative, `StrictHostKeyChecking=no`, would push production code to whatever
answers on that address.

## The workflow

```yaml
# .github/workflows/deploy.yml
name: deploy

on:
  push:
    branches: [main]
  workflow_dispatch:

# Two pushes close together must not interleave halfway through an rsync, and a
# deploy that has started is never cancelled: that would leave half a tree.
concurrency:
  group: deploy
  cancel-in-progress: false

env:
  SOLI_VERSION: "2.9.1"   # the version you run on the server

jobs:
  verify:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5

      # The published install.sh only knows `latest`; pin the version you run.
      - name: Install soli ${{ env.SOLI_VERSION }}
        run: |
          set -euo pipefail
          tmp="$(mktemp -d)"
          curl -fsSL "https://github.com/solisoft/soli_lang/releases/download/v$SOLI_VERSION/soli-linux-amd64.tar.gz" \
            -o "$tmp/soli.tar.gz"
          tar -xzf "$tmp/soli.tar.gz" -C "$tmp"
          sudo install -m 0755 "$(find "$tmp" -type f -name soli -perm -u+x | head -1)" /usr/local/bin/soli
          soli --version

      # Boot the app the way the server will, and ask for /up. A syntax error, a
      # missing view or a broken route fails here instead of in production.
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

      - name: Lint
        run: soli lint

      # Add `soli test` here once your specs can reach a database on the runner
      # (they read .env.test).

  deploy:
    needs: verify
    runs-on: ubuntu-latest
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

      - name: The site folder is ready
        run: ssh "deploy@$HOST" "test -f '$SITE/.env' && test -f '$SITE/app.infos'"

      # What leaves the repository leaves the server (--delete-delay), except
      # what belongs to the server: its secrets and the proxy's own files.
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

      # Starts the idle slot, waits for its /up, moves traffic, stops the old
      # slot. A slot that never answers is stopped and the old one keeps serving;
      # the command then exits non-zero and this job goes red.
      - name: Switch traffic
        run: ssh "deploy@$HOST" "soli-proxy deploy -c /etc/soli-proxy/proxy.conf '$DOMAIN'"

      # End to end: through the proxy, with the real hostname and certificate.
      - name: The site answers through the proxy
        run: |
          set -euo pipefail
          for i in $(seq 1 10); do
            code="$(curl -s -o /dev/null -w '%{http_code}' -m 8 "https://$DOMAIN/up")"
            echo "attempt $i: $code"
            [ "$code" = "200" ] && exit 0
            sleep 3
          done
          exit 1
```

Most of it explains itself. Four choices in it are worth knowing why:

**The `verify` job boots the app and asks for `/up`.** Booting compiles every
handler and view. That is the cheapest way to catch the errors that would otherwise
only show up when the new slot fails its health check on the server. Add
`soli test` next to `soli lint` as soon as your specs can run on the runner. They
read `.env.test`, so the database it names has to be running there.

**`soli-proxy deploy` rather than `touch restart.txt`.** Touching `restart.txt` in
the site folder also starts a blue/green deploy, but nothing reports back to you: if
the new slot cannot start, the old one keeps serving and the workflow stays green.
`soli-proxy deploy` waits for the outcome and exits non-zero with the reason. The
`-c` flag points at `proxy.conf`; the command reads the admin address and
`api_key` from the `config.toml` beside it.

**The last check goes through the proxy.** Probing the app's own port would skip
the parts that can still be wrong after a successful switch: the routing, the
certificate, the redirect to HTTPS.

**`app.infos` and `.env` are excluded from rsync**, even if a copy is in the
repository. They describe how the server runs your app and what it connects to.
Change them on the server, then run `soli-proxy deploy` yourself.

## When something goes wrong

| Symptom | Where to look |
|---|---|
| The switch step fails | The new slot's output is in `/var/lib/soli-proxy/run/logs/myapp.example.com/` (`blue.log`, `green.log`). The previous version is still serving. |
| Every request gets 421 | No app is routed for that host: check that `domain` is set if `app.infos` has a `start_script`, and that a `default ->` rule in `proxy.conf` is not catching the request first. |
| No certificate | Port 80 has to reach the proxy for HTTP-01, and the DNS record has to point at this server. `journalctl -u soli-proxy` has the ACME errors. |
| The app does not start, and the log says it refuses to run as root | `[apps] default_user` is missing from `config.toml`. |
| `soli-proxy deploy` says 401 | `api_key` in `config.toml` does not match, or the deploy user cannot read the file. |

A failed start puts the app in quarantine: the proxy stops retrying it until the
next explicit deploy, so a crash loop does not take the old slot down with it.

## Rolling back

The simplest rollback is the one the workflow already gives you: revert the commit
and push. The previous code goes through the same checks and the same switch.

If you need to go back faster, without a build, `soli cloud deploy` keeps each
release in its own folder on the server and rolls back by moving a symlink, with
the same proxy and the same health gate underneath. The
[deployment docs](/docs/development-tools/deploy) cover it, and `soli deploy`, which pulls a git
checkout on the server instead of receiving files from CI.
