# Debugging

Breakpoints, an interactive REPL, a per-request query log and flamegraph, a
request inspector, and a mail inbox — all under `soli serve --dev`, all costing
nothing in production.

## Breakpoints

`debug()` pauses execution and opens the interactive debug page.

```soli
def process_user(user_id: Int) -> Hash
  user = database.find(user_id)
  debug()                       # inspect `user` here

  if user.nil?
    return {"error": "User not found"}
  end

  profile = enrich_profile(user)
  debug()                       # and the enriched data here
  profile
end
```

Breakpoints work in development only. In production `debug()` is ignored.

> The builtin was once named `break()`. It was renamed because `break` is now a
> loop keyword.

The page gives you a REPL running in the breakpoint's context, a clickable stack
trace, a source viewer with the line highlighted, and a request inspector for
params, query, headers and body. It also opens automatically on an error.

## The REPL

The debug page's REPL executes any Soli code in the breakpoint's scope.

```soli
req               # the full request
req["params"]     # route parameters
req["query"]      # query string
req["body"]       # POST / PUT body
req["headers"]    # HTTP headers
session           # session data
breakpoint_env    # locals at the breakpoint
```

`@` references the last result, and the arrow keys browse history:

```soli
req["params"]["id"]   # "123"
@ + 100               # 223
```

**Access.** The REPL endpoint is token-protected and accepts loopback clients by
default. If your `*.test` domains point at a trusted local server on another
machine, start dev mode with `SOLI_DEV_REPL_ALLOW_REMOTE=1` and pin the token
yourself with `SOLI_DEV_REPL_SECRET=<long-random-string>`. The server refuses to
start in remote-allowed mode without the secret, so the credential is never
embedded in an HTML error page.

**Local `Host` only.** The REPL token, the dev-bar diagnostics, `/__dev/*`, the
inbox and request replay answer only when the request's `Host` is local —
`localhost`, `*.localhost`, an IP literal, or a host listed in `SOLI_APP_HOSTS`.
This stops a DNS-rebinding page from driving them through your browser. A name
such as `myapp.test` or `mymac.local` must be added to `SOLI_APP_HOSTS`; until it
is, those endpoints 404 and error pages carry no REPL token. The inbox *clear*
and *replay* POSTs also require a same-origin `Origin`/`Referer`, and the jobs
dashboard is credential-free only from a loopback peer with a local `Host`.

## The query log

Under `--dev`, every query a request runs through the Model layer is captured
into a per-request stack. `dev_queries()` returns it.

The query, HTTP and KV logs (also kept in production when `SOLI_LOG` asks for
them) hold at most **10 000 entries** per request, WebSocket/LiveView event or
background job — a loop issuing a million queries no longer grows the log
without bound — and are reset at the start of each WebSocket event and each job,
so one does not inherit the previous one's entries.

Every backend is covered. On SoliDB the entries are AQL; on the SQL adapters
they are the SQL actually sent, with binds numbered the way `$1` / `?` appear in
the statement — so the dev bar's DB panel, the N+1 badge,
`assert_no_n_plus_one` and `soli test --fail-on-n1` behave identically on all of
them. Bind values over 200 characters are truncated, with their real length
noted, so a large document cannot flood the panel.

| Key | Type | Meaning |
|---|---|---|
| `query` | `String` | the statement sent |
| `bind_vars` | `Hash \| null` | bind variables, or null |
| `duration_ms` | `Float` | wall-clock milliseconds |

```soli
def index
  users = User.where("doc.active == true").all
  posts = Post.includes("author").all

  # [] in production, populated under --dev
  render("users/index", {
    "users":   users,
    "posts":   posts,
    "queries": dev_queries()
  })
end
```

```erb
<% if queries.length > 0 %>
  <div class="dev-bar">
    <h3><%= queries.length %> queries</h3>
    <ol>
      <% for q in queries %>
        <li>
          <code><%= q["query"] %></code>
          <% if q["bind_vars"] != null %>
            <small>binds: <%= json_stringify(q["bind_vars"]) %></small>
          <% end %>
          <span><%= q["duration_ms"] %> ms</span>
        </li>
      <% end %>
    </ol>
  </div>
<% end %>
```

Covered: every Model operation, eager-loaded `includes`, soft-delete scopes,
uniqueness validation lookups, HABTM join-table operations, direct
`Solidb(host, db).query(...)` calls, mocked queries from `register_query_mock`,
and internal session-storage queries.

**Zero production cost.** The executor never calls the logger there — the gate
is a single relaxed atomic load — and `dev_queries()` always returns `[]`, so
the debug-bar partial is safe to leave in your layout unconditionally.

## The flamegraph

Click **flame** in the dev bar for a hierarchical view of every span in the
request: middleware, before/after actions, controller dispatch, view, partials,
every Soli function call, plus DB and HTTP. Hover a rectangle for its duration,
click to zoom, double-click to reset.

| Kind | Colour | Source |
|---|---|---|
| middleware | amber | each scoped or global middleware invocation |
| before/after_action | dim amber | each controller hook fire |
| action | cyan | top-level handler dispatch (`posts#show`) |
| view / partial | green | each `render(...)` / `render_partial(...)` |
| db | purple | each query (name = first 80 chars) |
| http | pink | each outbound `HTTP.*` call |
| fn | slate | every Soli function call |

X is request time in microseconds from request start; Y is stack depth, parents
above children. Reading it:

- a **view** rectangle spanning most of the request → render dominates; check
  the partial breakdown in the render panel;
- **many narrow purple rectangles at one depth** → likely an N+1, and the query
  panel will flag it;
- a **wide cyan action with thin children** → the time is in your own code
  outside the framework hooks; zoom into the `fn` spans.

The panel has a `⬇ trace.json` link that downloads the same data as Chrome Trace
Event Format — drop it into [ui.perfetto.dev](https://ui.perfetto.dev) or
`chrome://tracing` for timeline navigation and aggregation.

**Large requests.** A request that walks a few hundred records produces
thousands of spans, and drawing every one made the panel itself the slowest
thing on the page (3.4 MB of markup on a real dashboard). The chart therefore
draws the **300 heaviest spans** — chosen by duration, not by order, so the
expensive ones are never the ones cut — and the header says so
(`showing 300 heaviest`). The span count in the header stays exact. Set
`SOLI_DEV_FLAME_MAX` to change the bound, or `SOLI_DEV_FLAME_MAX=0` to draw
everything.

`trace.json` is never truncated: it is the artefact you load into a profiler.
Up to 64 KB it is inlined in the link; above that the link points at
`/__solidev/trace/<request-id>`, a dev-only endpoint that serves the complete
trace from the same ring buffer the requests panel reads (404 once the request
has aged out).

## The requests panel

Every dev response carries `X-Soli-Route` naming the route that handled it. The
dev bar shows the current page's route beside the URL, and clicking the URL
expands a panel listing every route the page touched — the page itself plus each
XHR, `fetch` or HTMx call it fired afterwards.

The bar patches `fetch` and `XMLHttpRequest` once and reads the header off each
response. **Each row's duration is server-side render time** (from
`X-Soli-Render-Us`) — time actually spent in your app, not the client round-trip
with its queue, network and transfer; hover it to see the round-trip too.

Click a row to retarget the db, http, kv and flame panels to *that* request, so
you can open the flamegraph for one XHR rather than only for the page. Row 0 is
always the page.

**Replay.** Each row has a `↻` button that re-dispatches the captured request —
same method, path, query, headers and body — through the real worker path, so
the handler runs again without re-driving the UI. The panels then retarget to
the replay's fresh request id. Replayed responses are tagged `X-Soli-Replay: 1`.

> **Replaying re-runs side effects.** A replayed `POST` / `PATCH` / `DELETE`
> re-executes its mutation — a second insert, a second charge — exactly as if
> the client had resubmitted. The per-form CSRF check is skipped (the session
> token may have rotated since capture), and multipart uploads are not
> re-parsed: the raw body is re-sent, but file fields arrive empty.

**Scope.** Only same-origin requests that hit a Soli route become rows; static
assets, cross-origin fetches and 404s are ignored. The list resets on a full
page load. Only the most recent requests are kept, so a long-idle row may report
that it has aged out. WebSocket, SSE and `sendBeacon` traffic is not captured,
and a request fired before the bar's script runs may be missed.

**HTMx.** The bar is not re-injected on a swap — that would stack a second bar —
so the header follows HTMx navigations instead: when HTMx pushes the URL
(`hx-push-url` / `hx-boost`) the header's route and render time update. Widget
requests that do not change the URL are added to the panel but leave the header
alone.

## The mail inbox

Under `--dev`, `/__soli/inbox` — or the **tools** button — reads every email the
app has sent since the server started. A built-in MailCatcher, with no second
process.

Each message opens with its headers, its attachments, and tabs for the HTML body
(in a sandboxed iframe), the text part, and the raw RFC 5322 source, downloadable
as `.eml`. The listing is searchable (`?q=` matches subjects, addresses, bodies
and attachment names) and paginated (`?per=` / `?page=`).

**No local SMTP server required**: a mailer with no configured host captures into
the inbox instead of failing, so a signup flow works on a laptop with no mail
infrastructure. Messages are tagged `sent`, `captured` or `failed` — failures are
captured too, with the error.

The inbox is in-memory (the last 100 messages, cleared on restart) and its routes
exist only under `--dev`. `/__soli/mailers` previews templates with fake data
instead, and `/__soli/components` is the component catalog.

## Development versus production

```bash
soli serve . --dev

# Allow the debug REPL from another trusted local machine.
# The secret is required — startup refuses without it.
SOLI_DEV_REPL_ALLOW_REMOTE=1 SOLI_DEV_REPL_SECRET=<long-random-string> \
  soli serve . --dev
```

| | Development | Production |
|---|---|---|
| Breakpoints | enabled | ignored |
| Debug page | full, interactive | a simple error page |
| Hot reload | enabled | disabled |
| Stack traces | detailed | minimal |
| `dev_queries()` | populated | always `[]` |
| Flamegraph, requests panel, inbox | present | not mounted |

The dev bar is injected into `text/html` responses only, never into JSON, and
never in production — where no `X-Soli-Route` header is sent and no script is
added, so there is nothing to pay for.

## See also

- [`linting.md`](linting.md) — `soli lint` and `soli check`
- [`mailer.md`](mailer.md) — the full inbox tour
- [`observability.md`](observability.md) — production logs, metrics and traces
- Rendered page: `/docs/development-tools/debugging`
