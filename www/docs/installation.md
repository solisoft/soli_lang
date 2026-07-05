# Installation

## Prerequisites

- Node.js (v16 or higher)
- npm or yarn
- OpenSSL — required to build from source via Cargo
  - Debian/Ubuntu: `sudo apt install libssl-dev`
  - Fedora/RHEL: `sudo dnf install openssl-devel`
  - macOS (Homebrew): `brew install openssl`
  - Verify: `openssl version`

## Install SoliLang

### Quick Install (Recommended)

```bash
curl -sSL https://raw.githubusercontent.com/solisoft/soli_lang/main/install.sh | sh
```

This detects your OS and architecture, downloads the latest release binary, and installs it to `~/.local/bin`.

When run as **root** (e.g. through `sudo`, or inside a Docker image build), the installer
automatically targets `/usr/local/bin` so every user on the machine can run `soli` — no
`--system` flag needed:

```bash
curl -sSL https://raw.githubusercontent.com/solisoft/soli_lang/main/install.sh | sudo sh
```

A global install also removes any stale per-user copy (e.g. `/root/.local/bin/soli`) left by
older installs, so PATH can't shadow the new binary with an outdated one.

For system-wide installation as a non-root user (the script will use `sudo` for the copy step):

```bash
curl -sSL https://raw.githubusercontent.com/solisoft/soli_lang/main/install.sh | sh -s -- --system
```

To force a per-user install even when running as root, pass `--user`:

```bash
curl -sSL https://raw.githubusercontent.com/solisoft/soli_lang/main/install.sh | sudo sh -s -- --user
```

### Updating

`soli update` replaces the binary in place, wherever it was installed. If Soli lives in a
root-owned directory (such as `/usr/local/bin`), run the update with `sudo`:

```bash
sudo soli update
```

Running `soli update` without the needed permissions prints a hint telling you to re-run with
`sudo`.

### Via Cargo

```bash
cargo install solilang
```

### From Source

```bash
# Clone the repository
git clone https://github.com/solisoft/soli_lang.git
cd soli_lang

# Build the project
cargo build --release

# Install globally
cargo install --path .
```

## Docker

Soli ships an official container image on the GitHub Container Registry, rebuilt
and published for every release.

```bash
# Pull the latest release (or pin a version, e.g. :v1.13.5)
docker pull ghcr.io/solisoft/soli_lang:latest
```

The image's entrypoint **is** the `soli` binary, so any `soli` subcommand works
as the container command:

```bash
docker run --rm ghcr.io/solisoft/soli_lang:latest --version
```

### Run a Soli app in a container

Mount your project into the container and publish the server port. The server
binds `0.0.0.0` by default, so the published port is reachable from the host:

```bash
docker run --rm -p 5011:5011 \
  -v "$(pwd):/app" -w /app \
  ghcr.io/solisoft/soli_lang:latest serve . --port 5011
```

Your app is now available at `http://localhost:5011`.

### Build the image yourself

The repository ships a multi-stage `Dockerfile` that compiles a release binary
and copies it into a slim Debian runtime:

```bash
git clone https://github.com/solisoft/soli_lang.git
cd soli_lang
docker build -t soli .
docker run --rm soli --version
```

## Create a New MVC Project

```bash
# Clone this example or template
git clone https://github.com/solilang/solilang.git
cd solilang/examples/mvc_app

# Install frontend dependencies
npm install

# Build CSS
npm run build:css

# Start development server
npm run dev
```

## Project Setup

### 1. Configure Routes

Edit `config/routes.sl`:

```soli
get("/", "home#index");
get("/about", "home#about");
post("/contact", "home#contact");
```

### 2. Create Controllers

Create controllers in `app/controllers/`:

```soli
def index
  return render("home/index", {
    "title": "Welcome"
  })
end
```

### 3. Add Views

Create templates in `app/views/home/`:

```erb
<h1><%= title %></h1>
<p>Welcome to my app!</p>
```

## Running in Development

```bash
soli serve . --dev
```

In `--dev` mode the server compiles your Tailwind CSS for you: it scans
`app/assets/css/*.css`, detects whether the project is **Tailwind v3 or v4**
(from your CSS directives and `package.json`), and writes the result to
`public/css/`. It recompiles on startup and whenever views, asset CSS,
controllers, or helpers change, so new utility classes show up on the next
reload.

Which Tailwind binary it uses:

- a local `node_modules/.bin/tailwindcss` if present (whatever version you
  installed — preferred), otherwise
- a SHA-256-pinned standalone CLI downloaded to `~/.soli/bin/` (v4.3.1 for
  v4 projects, v3.4.17 for legacy v3 projects).

Because the dev server handles this, a separate Tailwind watcher is optional.
If you prefer to run your own (e.g. for the official `--watch` incremental
mode), the template still ships the npm scripts:

```bash
# Start both the Tailwind watcher and the Soli server together
npm run dev
```

## Building for Production

```bash
# Build CSS
npm run build:css

# Build Soli application
cargo build --release
```

## Verifying Installation

Create a test file:

```soli
# test.sl
println("Hello, SoliLang!");
```

Run it:

```bash
soli test.sl
```

You should see: `Hello, SoliLang!`
