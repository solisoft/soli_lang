# SEC-067: `asset_cache::walk` follows symlinks

- **Severity:** Low
- **Status:** Todo
- **Location:** `src/serve/asset_cache.rs:57-114`

**Issue:** `std::fs::read_dir` follows directory symlinks. A symlink in `public/` pointing at `/etc` would have any matching `.css`/`.js` files cached and served. Mostly mitigated by the extension filter, but `/etc/foo.js` (or any file the attacker plants with that extension) would be exposed.

**Fix:** Use `WalkDir::follow_links(false)` or skip symlinks during the walk.
