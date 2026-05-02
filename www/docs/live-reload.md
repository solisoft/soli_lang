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
soli run --dev
```

Or use the dev script:

When the server starts, you'll see a message indicating live reload is enabled:

```
Live reload enabled. Open http://localhost:5011 in your browser.
```

As you edit and save files, the browser will automatically reload to reflect your changes.

## Configuration

Live reload is automatically enabled when running in development mode. It can be controlled via environment variables:

| Variable | Description |
|----------|-------------|
| `SOLI_ENV=development` | Enables live reload (default) |
| `SOLI_ENV=production` | Disables live reload |

## Events

The live reload system watches for changes in these directories:

- `app/views/` - View templates
- `app/controllers/` - Controller files
- `app/models/` - Model files
- `config/` - Configuration files
- `public/` - Static assets

When any file in these directories changes, a reload signal is sent to connected browsers.

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
pkill -f "soli run"
./dev.sh
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

You'll see a line like the above on prod startup confirming the snapshot. Files larger than 10 MB are skipped (and read from disk on demand). Other extensions (images, fonts) continue to be read fresh from disk per request — only `.css` and `.js` are cached.
