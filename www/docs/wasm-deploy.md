# WASM / Edge Deployment

Soli's Rust VM compiles to WebAssembly, enabling your Soli applications to run at the edge on Cloudflare Workers. This gives you the expressiveness of a high-level dynamic language with the performance and global distribution of edge computing.

Rails can't do this. Node can, but with enormous bundle sizes. Soli's tiny WASM footprint makes it ideal for edge deployment.

## Quick Start

```bash
# Build your app as a Cloudflare Worker
soli build my_app --target wasm

# Navigate to the generated project
cd my_app_worker

# Compile to WASM
cargo build --target wasm32-unknown-unknown --release

# Deploy to Cloudflare Workers
npx wrangler deploy
```

## How It Works

Soli is implemented in Rust, which compiles natively to WebAssembly. Running `soli build --target wasm` produces a self-contained Cloudflare Workers project containing:

- A Rust crate with the `worker` framework and the Soli interpreter
- Your application's `.sl` source files embedded at compile time
- A `wrangler.toml` configuration file for Workers deployment

## Generated Project Structure

```
my_app_worker/
├── Cargo.toml              # Worker crate + Soli interpreter dependency
├── wrangler.toml           # Cloudflare Workers configuration
├── public/                 # Static assets directory
├── src/
│   ├── lib.rs              # fetch() request handler
│   └── bundle.rs           # Auto-generated — embeds your .sl files
└── soli_app/               # Copy of your project's source files
    ├── app/
    │   └── controllers/
    └── config/
```

## Prerequisites

- Rust toolchain with `wasm32-unknown-unknown` target installed:
  ```bash
  rustup target add wasm32-unknown-unknown
  ```
- [Node.js](https://nodejs.org) and `wrangler` CLI for Cloudflare deployment:
  ```bash
  npm install -g wrangler
  ```
- A [Cloudflare account](https://dash.cloudflare.com) with Workers enabled

## Deployment Steps

### 1. Build the Worker Project

```bash
soli build my_app --target wasm
```

This creates `my_app_worker/` — a complete, self-contained Rust project ready for WASM compilation.

### 2. Compile to WebAssembly

```bash
cd my_app_worker
cargo build --target wasm32-unknown-unknown --release
```

The first build downloads and compiles all dependencies. Subsequent builds are much faster.

### 3. Deploy to Cloudflare

```bash
npx wrangler deploy
```

Your Soli application is now live at `https://my-app.your-username.workers.dev`.

## Architecture

```
                     ┌──────────────────┐
  HTTP Request ─────►│  Cloudflare Edge  │
                     │  (global network) │
                     └────────┬─────────┘
                              │
                     ┌────────▼─────────┐
                     │  Soli WASM Worker│
                     │                  │
                     │  lib.rs          │
                     │   └─ fetch()     │
                     │       ├─ Parse   │
                     │       │  request │
                     │       ├─ Execute │
                     │       │  Soli    │
                     │       │  app     │
                     │       └─ Return  │
                     │          response│
                     └─────────────────┘
```

## Current Limitations

- **WASM executor is in development** — execution returns `"Execution not supported in WASM mode"`. Full Soli code execution on Workers is coming in a future release.
- **Platform-dependent builtins** (file I/O, HTTP client, model/ORM, database) are not available in WASM mode. Pure builtins (math, strings, crypto, JSON, regex, datetime, validation, i18n) work.
- **Cloudflare Workers APIs** (KV, R2, D1, Queues) are not yet integrated as Soli builtins.

## Comparison

| Feature | Soli (Edge) | Rails | Node (Edge) |
|---------|------------|-------|-------------|
| Edge deployment | ✅ Native WASM | ❌ No | ✅ V8 isolates |
| Bundle size | ~2-5 MB | N/A | ~20-100 MB |
| Language expressiveness | High (Ruby-like) | High | Medium |
| Startup time | <10ms | N/A | ~50-200ms |
| File I/O | ❌ (not in WASM) | ✅ | ✅ |
| Database | ❌ (not yet) | ✅ | ✅ |
| Static typing | Optional | ❌ | ❌ (TS adds) |

## Future Roadmap

- [ ] Full WASM tree-walking interpreter with pure builtins
- [ ] Cloudflare Workers API bindings (KV, R2, D1, Queues)
- [ ] HTTP client builtin using Workers `fetch` API
- [ ] Model/ORM with D1 database adapter
- [ ] Session storage via Workers KV
- [ ] `soli deploy --target cloudflare` for one-command deployment
