# SEC-066: Tailwind binary downloaded with `curl -sL`, no checksum

- **Severity:** Low
- **Status:** Todo
- **Location:** `src/serve/tailwind.rs:75-91`

**Issue:** `curl -sL` follows redirects to whatever GitHub's CDN returns, writes it to `~/.soli/bin/...`, then `chmod 0755` and exec. No signature, no checksum, no pinned version (uses `releases/latest`). A compromise of the CDN, or DNS/MITM during dev install, becomes direct code execution on the developer machine.

**Fix:** Pin a known version; verify the SHA256 of the downloaded binary against a hash baked into the soli binary.
