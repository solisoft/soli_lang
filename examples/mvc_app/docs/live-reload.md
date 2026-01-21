# Live Reload

Soli MVC includes a live reload feature that automatically refreshes your browser when files change during development. This speeds up your development workflow by eliminating the need to manually refresh the page.

## How It Works

When you run your application with the dev server, live reload is automatically enabled. The server establishes a WebSocket connection with your browser that listens for file change events.

### Connection Methods

The live reload client uses two connection methods:

1. **WebSocket (Primary)**: Establishes a persistent WebSocket connection at `/__livereload_ws` for real-time reload signals
2. **Server-Sent Events (Fallback)**: Uses SSE at `/__livereload` if WebSocket connections are unavailable

The client automatically detects which method works best for your browser and server configuration.

## Usage

Simply run your application in development mode:

```bash
cd /home/olivier.bonnaure@delupay.com/workspace/solilang/examples/mvc_app
./dev.sh
```

Or use the Soli CLI with the `--dev` flag:

```bash
soli run --dev
```

When the server starts, you'll see a message indicating live reload is enabled:

```
Live reload enabled. Open http://localhost:3000 in your browser.
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
2. **Verify port availability**: Ensure port 3000 is not in use by another process
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
