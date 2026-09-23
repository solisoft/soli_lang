# Live Reload

Soli MVC includes a live reload feature that automatically refreshes your browser when files change during development. This speeds up your development workflow by eliminating the need to manually refresh the page.

## How It Works

When you run your application with the `--dev` flag, live reload is enabled. The server establishes a WebSocket connection with your browser that listens for file change events.

### Connection Methods

The live reload client uses two connection methods:

1. **WebSocket (Primary)**: Establishes a persistent WebSocket connection at `/__livereload_ws` for real-time reload signals
2. **Server-Sent Events (Fallback)**: Uses SSE at `/__livereload` if WebSocket connections are unavailable

The client automatically detects which method works best for your browser and server configuration.

## Usage

Run your application with the `--dev` flag to enable live reload:

```bash
soli serve . --dev
soli serve ./myapp --dev --port 8080
```

When the server starts, you'll see a message indicating live reload is enabled:

```
Live reload enabled. Open http://localhost:5011 in your browser.
```

As you edit and save files, the browser will automatically reload to reflect your changes.

## Configuration

Live reload follows the `--dev` flag, and nothing else: `soli serve <folder> --dev`
enables it, `soli serve <folder>` does not. There is no environment variable that
turns it on — a production server never serves the client script or the reload
endpoints, so the cost in production is zero rather than merely disabled.

## Events

The live reload system watches these directories:

| Watched | Reloaded |
|---|---|
| `app/views/` (`.slv`, `.erb`, `.md`) | the template cache |
| `app/controllers/` (`*_controller.sl`) | controllers, and the routes derived from them |
| `app/models/`, `app/services/`, `app/policies/`, `app/mailers/` | all four, together |
| `app/middleware/` | middleware |
| `app/helpers/` | view helpers |
| `app/jobs/` (`*_job.sl`) | job classes |
| `config/routes.sl` | the route table and the `<name>_path` helpers |
| `public/`, `app/assets/css/` | static assets; Tailwind recompiles |

The four model-ish directories are **one signal**: they load in a fixed order
into the same environment (a policy refers to a model, a mailer to both), so a
change in any of them reloads all four. Editing a policy or a mailer on its own
used not to reload anything at all.

When any watched file changes, the workers reload what changed and a reload
signal is sent to connected browsers.

## Troubleshooting

### Live Reload Not Working

1. **Check browser console**: Look for `[livereload]` messages indicating connection status
2. **Verify port availability**: Ensure port 5011 is not in use by another process
3. **Disable browser extensions**: Some extensions may interfere with WebSocket connections
4. **Check file permissions**: Ensure the server has read access to your application files

### WebSocket Connection Failed

If you see WebSocket errors in the console, the client will automatically fall back to SSE. If both fail:

```bash
# Restart the development server
pkill -f "soli serve"
soli serve . --dev
```

### Multiple Browser Tabs

Live reload works across multiple browser tabs. When a file changes, all connected tabs will reload.

## Production Mode

When you start the server **without** `--dev`, behaviour flips for static assets:

- **Live reload is disabled.** No WebSocket, no file watcher, no auto-refresh.
- **CSS and JS files are snapshotted into memory at startup.** The server walks `public/` once and serves the bytes it loaded, with content-hash `ETag` and `Cache-Control: public, max-age=31536000, immutable`.

The in-memory snapshot exists to prevent a deploy-time race: if you overwrite `public/css/app.css` on disk before restarting the binary, the running process keeps serving the **old** bytes. Browsers that already loaded HTML referencing a specific asset version don't suddenly fetch mismatched new bytes against the cached page. The next binary restart reloads from disk.

```
Cached 12 CSS/JS assets (438213 bytes) for prod-mode serving
```

You'll see a line like the above on prod startup confirming the snapshot. Files larger than 10 MB are skipped (and read from disk on demand). Other extensions (images, fonts) continue to be read fresh from disk per request — only `.css` and `.js` are cached. A file served from disk that is larger than 1 MiB is streamed rather than read into memory whole, so a large download does not hold its full size in a worker.
