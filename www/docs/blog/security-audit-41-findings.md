# An Audit's 41 Findings

Before v2.5.0 we did a static audit of Soli's server, builtins, ORM and interpreter/VM. We read the code and asked two questions of every part of it: what can someone on the internet make it do? And what does it hold on to in a worker that runs for weeks? The audit came back with **41 findings**. The first round of fixes closed 36 and got partway on the other 5. A follow-up closed four of those five and fixed one more gap it found along the way. One item is still open.

Most of the findings follow the same pattern: **the check was there, but it wasn't where the request actually went.** A CSRF exemption assumed application code never sees framework paths. An expiry sweep only ran from a function the server never calls. A trusted-proxy list was consulted on one thread and ignored on another. Each was an assumption that had stopped being true without anyone noticing.

This post covers what was wrong, why, what changed, and what you have to do after upgrading.

<figure style="margin:1.5rem auto;max-width:1024px;">
  <img src="/images/blog/security-audit-41-findings.svg" width="1024" height="576" alt="41 audit findings: 36 closed and 5 partly closed in the first round; a follow-up closes four of the five, leaving the locale lookup order. Three groups: security gates, leaks in long-lived workers, hot-path costs. A side card shows the cargo-audit waiver file going from eleven entries to ten as quick-xml moves from 0.31 to 0.41, clearing RUSTSEC-2026-0194 and -0195." style="display:block;width:100%;height:auto;border-radius:12px;border:1px solid #30363d;background:#0b0d0f;">
  <figcaption style="text-align:center;color:#8b949e;font-size:0.875rem;margin-top:0.5rem;">41 findings, two commits, one left open. Most were checks that existed but never saw the request.</figcaption>
</figure>

## Paths the framework owns never reach your routes

CSRF protection in Soli has two layers: an Origin/Referer gate and per-form token verification. Framework endpoints under the reserved `/__…` namespace skip both. That is reasonable as long as the framework is the only thing that ever answers those paths.

In production that assumption failed. The dev endpoints don't exist there, so a request for them fell through to application routing. `POST /__soli/account/delete` matched an app route declared as `post("/:locale/account/delete")`, with `:locale` bound to `__soli` and **no CSRF check at all**, because the path was still exempt.

Now, a request under `/__soli/`, `/__solidev/`, `/__dev/`, `/__coverage__` or `/__livereload` (the exact path, `/__livereload/…` or `/__livereload_ws`) that no framework handler claims gets a `404`. The exemption is only sound if the app never sees these paths, so the server now enforces that. `/__livereload` is matched exactly rather than as a prefix, so an app can still route `/__livereloadanything` if it wants to.

`/_health`, `/_ready` and `/_metrics` came off the exemption list too. They answer GET and HEAD only, which CSRF doesn't check, so the exemption only ever covered a *POST*, and a POST there can only reach an application route.

## `--dev` endpoints and DNS rebinding

The dev server exposes powerful endpoints: the dev bar's diagnostics, `/__dev/*`, the mail inbox, request replay, and a REPL whose token is embedded in dev error pages. The diagnostics were already restricted to trusted peers such as loopback, but a peer check can't see DNS rebinding. A page on `attacker.example` re-points its own name at `127.0.0.1`, and your browser, which *is* a loopback peer, then reads those endpoints as same-origin.

The attacker can't change the `Host` header your browser sends. So the dev endpoints now answer only for a local host: `localhost`, `*.localhost`, any IP literal, or a host listed in `SOLI_APP_HOSTS`. An IP has no DNS to rebind, which is why opening the LAN address on your phone still works. A request without a `Host` is refused. The REPL token is no longer embedded in error pages served to any other host.

Two related gates ship with this. The dev jobs dashboard is credential-free only from a loopback peer with a local `Host`. The inbox *clear* and *replay* POSTs must now carry a same-origin `Origin` or `Referer`.

**What you do:** if you develop against a LAN name like `mymac.local` or `myapp.test`, add it to `SOLI_APP_HOSTS`, or the dev bar goes quiet.

## The upload budget, charged as bytes arrive

`SOLI_MAX_BODY_SIZE` (8 MiB by default) limits one request. `SOLI_MAX_INFLIGHT_BODY_BYTES` limits the sum of all bodies being buffered, and defaults to 16 times the per-request cap, so 128 MiB. The problem was *when* each request was charged. Every body reserved the full per-request cap as soon as it arrived. So sixteen idle chunked POSTs, each one sending nothing, used up the whole budget, and every other upload on the server got a `503`.

Bodies are now charged as they arrive. A request starts with a 64 KiB reservation, which doubles as needed up to `SOLI_MAX_BODY_SIZE`. A body that stalls between two frames for `SOLI_BODY_IDLE_TIMEOUT_SECS` (default `10`) gets a `408`. A transport error mid-body is now a `400`. It used to be a `413`, which was misleading.

The aggregate budget was still first come, first served, so one client opening a few dozen slow uploads could hold all of it. The follow-up fixed that too. The new `SOLI_BODY_BUDGET_PER_IP_BYTES` limits one client's share. The default is a quarter of the global budget, never less than one maximum-size body and never more than the global budget. Over the limit, the client gets the same `503 Server busy: too many uploads in flight` with `Retry-After: 1`. IPv6 clients are counted per `/64`.

"One client" means the TCP peer, and that has a consequence you need to act on. **Behind a reverse proxy without trust proxy enabled, every client is the proxy.** All of them share one quarter of your upload capacity. Turn trust proxy on, raise the variable, or set it to `0`.

## Behind a proxy, "local" means nothing

Two findings came from the same gap.

`/_metrics` without `SOLI_METRICS_TOKEN` was allowed for loopback and private peers. Behind a reverse proxy, every peer looks local. Now, without a token, `/_metrics` returns `404` for any request carrying `X-Forwarded-For`, `X-Real-IP` or `Forwarded`, and for any request while trust proxy is on. If you scrape metrics through a proxy, set `SOLI_METRICS_TOKEN`.

The second one was worse, and the follow-up found it. `SOLI_TRUSTED_PROXIES` names the hops whose `X-Forwarded-*` headers are believed. The async-side origin checks didn't consult that list: the CSRF Origin gate, the WebSocket upgrade, live reload, and the `--dev` same-origin check. They run on a tokio thread where no peer address is recorded. That read as "not a request", so they believed `X-Forwarded-Host` from **any** peer whenever trust proxy was on. They now evaluate the list against the real TCP peer, like the rest of the request path.

**What you do:** `SOLI_TRUST_PROXY=1` is only safe behind a proxy that strips inbound `X-Forwarded-*` before setting its own. Pair it with `SOLI_TRUSTED_PROXIES` (for example `10.0.0.0/8,127.0.0.1,::1`), so that a client reaching the app directly isn't trusted.

## Smaller gates

- **HTTP/2 (h2c)** connections send keep-alive pings every `SOLI_H2_KEEPALIVE_SECS` (default `30`) and close after `SOLI_CONN_IDLE_TIMEOUT_SECS` (default `60`) with no activity. Static files over 1 MiB are streamed from disk instead of read into memory.
- **5xx pages.** A custom `errors/5xx` template gets a generic `message` such as `"Internal Server Error"` in production, never the internal error text. The real error is still logged.
- **Cookie sessions have an absolute lifetime.** `SOLI_SESSION_MAX_LIFETIME` defaults to 30 days (`0` disables it) and counts from issue, so a stolen cookie can't be kept alive just by using it. `session_regenerate()` restarts the clock.
- **SSRF.** PDF remote images and the PAdES `sign.tsa` URL now go through the same guarded client as `HTTP.*`, which re-validates every redirect hop and caps the body. The blocklist gained `0.0.0.0/8`, `192.0.0.0/24`, `198.18.0.0/15`, `240.0.0.0/4`, and IPv6 forms that embed a blocked IPv4 address (NAT64, 6to4, Teredo, IPv4-compatible).
- **Document keys.** Empty, `.` and `..` keys and collection names are refused. Field names over 128 characters are refused in `order`, `find_by` and hash `where`.
- **SQL TLS.** Outside `--dev`, a Postgres or MySQL connection to a non-local host that doesn't verify the server logs a one-time warning recommending `verify-full`. The default is unchanged.
- **`Crypto.modexp`** limits the modulus and exponent to 8192 bits and the base to 16384. The cost grows with `bits(exp) · bits(modulus)²`, so a request-supplied operand could pin a worker for minutes.
- **`soli.lock` integrity.** `soli install` records `#@integrity <name>|<resolved sha>|sha256-<hex>` for each git or registry dependency and refuses a mismatch. The hash covers the *extracted tree*, not the archive, because GitHub and GitLab tarballs aren't byte-stable: the host can recompress them at any time. Path dependencies aren't checked.

## Leaks in long-lived workers

A Soli worker handles many requests, WebSocket events and jobs over its lifetime. Anything that grows per request and never shrinks eventually becomes a problem in production.

**In-memory sessions were never freed.** The store had an expiry sweep, triggered every 1000th new session or every 30 seconds. But it was armed only from `get_or_create`, which the serve path never calls, so on a server expired sessions stayed forever. The store is now split into 16 locked shards, the sweep is armed from session creation and lookup, and the store is capped by `SOLI_SESSION_MAX_IN_MEMORY` (default `100000`, `0` = unlimited). Past the cap, the least recently used sessions are evicted, which logs those users out.

**Per-request logs that no request emptied.** The query, HTTP, KV and span logs are thread-local vectors, and the only place they were cleared was the start of an HTTP request. WebSocket events and background jobs never go through there, so on a realtime worker those logs only grew. In `--dev`, a view redrawing ten times a second grew the process by 0.85 MB/s on one EUI app; after the fix it went from 66 to 72 MB over a minute and then stayed flat. One function, `forget_request_logs`, now runs on both paths, and each log is capped at 10 000 entries.

**SSE subscribers on quiet topics.** A disconnected subscriber was dropped only when something was broadcast to its topic, so a topic that never broadcasts kept every sender that had ever subscribed. Registering a subscriber now prunes its topic, and periodic sweeps remove empty ones.

**A throwaway interpreter that kept its builtins.** Soli builds a fresh `Interpreter` for every named-scope call, every user method on a primitive, and every validator. `Retry` was evaluated into the environment it was registered into, so its methods closed over that environment while the environment owned `Retry`. Neither was ever freed, and each throwaway interpreter kept a builtins registry of about 350 KB alive. `Retry` is now evaluated once per thread, in an environment of its own.

**Closure cycles, which you can write yourself.** A closure created in a method captures the method's environment. If you store that closure on `this`, the instance and the closure keep each other alive. A new lint rule, `smell/closure-cycle`, catches it:

```soli
class PriceFormatter
  currency: String

  new(currency: String) {
    this.currency = currency
    this.format = fn(amount) { "#{amount} #{this.currency}" }
  }
end
```

```
price_formatter.sl:6:5 - [smell/closure-cycle] a closure stored on `this` captures the
method's environment, which holds `this`: the instance and the closure keep each other
alive and are never freed; store a method name or pass the closure per call
```

The runtime handles the case the lint can't see. When the only references keeping a call's environment alive come from functions defined inside that call, and none of those functions escaped (returned, or stored in a global, collection or field), the environment is released when the call ends. The follow-up also moved query bind-variable names out of the process-wide symbol table. Before that, a client-chosen `where` key was interned and kept for the life of the worker.

## Hot-path costs

The audit also flagged work that was correct but repeated on every request. Zero-argument actions like the scaffold's `def index` always went to the tree-walker and now run on the VM. The SoliDB JWT took a global write lock per query. SQL adapters ran `CREATE TABLE IF NOT EXISTS` before every write. A request queued behind busy workers re-checked every millisecond instead of being woken when a slot freed.

Two of these fixes change behavior you can observe. ETag values change once after upgrading, so clients revalidate once. And seven variables, including `SOLI_APP_HOSTS`, `SOLI_DISABLE_CSRF` and `SOLI_CSRF_TOKENS`, are now read once per process, so changing one needs a restart.

## `cargo audit`: the spreadsheet waivers are gone

`.cargo/audit.toml` lists the advisories we've knowingly accepted, each with why it's there and what would let us remove it. Going into this cycle it had eleven entries. One of them covered two real vulnerabilities, RUSTSEC-2026-0194 and RUSTSEC-2026-0195 in quick-xml below 0.41: a quadratic duplicate-attribute scan and an unbounded namespace-declaration allocation. Both are denial of service, reachable only through `Spreadsheet.excel*` on an untrusted `.xlsx` or `.ods` file.

We waived them because we couldn't fix them. Every release of calamine and umya-spreadsheet resolved quick-xml below 0.41, and forcing the version with `[patch.crates.io]` didn't compile. calamine 0.36.1 and umya-spreadsheet 3.1.0 now require quick-xml ^0.41, so the manifest moved to them. calamine needed no source change across ten major versions. umya renamed `get_sheet_mut` to `sheet_mut`, which now returns a `Result`, and that result is reported rather than unwrapped because this code runs on a request.

The commit's summary was "`cargo audit --deny warnings` passes with no vulnerability waived". Reading the file again for this post, that is a little generous. Of the ten entries left, most are unmaintained-crate notices, but one is **RUSTSEC-2026-0258**, an HTTP/2 denial of service in `h2` 0.3.27. That copy is pinned by `hyper` 0.14 under `rusoto_core`, and the 0.3 line has no patched release. Our own HTTP/2 stack is on h2 0.4.18 and not affected. The waived copy is client-only, used solely by the S3 builtins, so an attacker would have to *be* the S3 endpoint, and it goes away with the migration to aws-sdk-s3. It's a narrow exposure, but it is still an accepted vulnerability.

## Sharp edges: documented, not changed

Some findings were APIs behaving exactly as designed, where the design has an edge you can cut yourself on. We documented these instead of changing them.

**`Crypto.totp_verify` has no replay protection.** It's stateless and accepts ±1 step, so the same code verifies repeatedly for up to about 90 seconds. Your app has to remember the last accepted step for each user, and rate-limit attempts, because six digits are only 10⁶ guesses:

```soli
# Which 30-second step did this code come from? nil if none of the three.
def accepted_totp_step(secret, code, now)
  current_step = now / 30   # Int division
  [current_step - 1, current_step, current_step + 1].find(fn(step) Crypto.totp_generate(secret, step * 30, 30) == code)
end

def verify_second_factor(user, code, now)
  step = accepted_totp_step(user["totp_secret"], code, now)
  return false if step.nil?
  return false if user["totp_last_step"].present? && step <= user["totp_last_step"]
  user["totp_last_step"] = step   # persist this with the user record
  true
end
```

With a fixed `now`, `Crypto.totp_verify` returns `true` for the same code at `now` and at `now + 20`. `verify_second_factor` returns `true` the first time and `false` the second.

**`jwt_verify` checks `aud` only when you pass `audience`.** A token minted for one service verifies at another unless you ask:

```soli
signing_secret = "0123456789abcdef0123456789abcdef"
token_for_billing = jwt_sign({"sub": "42", "aud": "billing"}, signing_secret, {"expires_in": 3600})

jwt_verify(token_for_billing, signing_secret)["sub"]                   # "42"
jwt_verify(token_for_billing, signing_secret, {"audience": "reporting"})
# {"error": true, "message": "InvalidAudience"}
```

Two more got the same treatment. `Crypto.pkcs1_unpad` isn't constant-time, so never expose its errors from a decryption endpoint. And `strip_html` is a naive tag stripper, not a sanitizer, so use `sanitize_html` for untrusted HTML.

## After upgrading

| If you… | Do this |
|---|---|
| develop against `mymac.local`, `myapp.test` or similar | add it to `SOLI_APP_HOSTS` |
| run behind a reverse proxy | `SOLI_TRUST_PROXY=1` plus `SOLI_TRUSTED_PROXIES`, or every client shares the proxy's quarter of the upload budget |
| scrape `/_metrics` through a proxy | set `SOLI_METRICS_TOKEN` |
| keep users signed in beyond 30 days on the cookie driver | set `SOLI_SESSION_MAX_LIFETIME` (seconds), knowing what it costs |
| run many sessions on the in-memory driver | expect LRU eviction past 100 000, or move to a persistent driver |
| use TOTP | store the last accepted step per user, and rate-limit |
| accept JWTs from more than one issuer or client | pass `audience` |

## What's left

The first commit listed five partial findings: interned bind-variable keys, no runtime cycle collection, no lockfile content hash, the enqueue poll, and the locale lookup order. The follow-up closed the first four. The locale lookup order is still open, and `instance.traverse(name)` with a request-supplied edge name still interns that name. Both are written down, so the next audit starts there instead of rediscovering them.

Full reference: [Production security defaults](/docs/security/defaults) and [Server Hardening](/docs/builtins/hardening).
