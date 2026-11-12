# Debugging

Breakpoints, an interactive REPL, a per-request query log and flamegraph, a
request inspector, and a mail inbox — all under `soli serve --dev`, all costing
nothing in production — plus a sampling profiler for scripts and test suites
(`--profile`).

## Errors on the command line

Outside the server, an error prints its usual line and a **code frame** under
it — the file, the source line, a caret, the "did you mean" suggestion and a
stable error code that `soli explain` describes:

```text
Error: Type error: Undefined variable 'nme' (did you mean 'name'?) at 2:7
 --> report.sl:2:7
  |
2 | print(nme)
  |       ^^^
  |
  = help: did you mean 'name'?
  = note: error[E0301] undefined-variable — `soli explain E0301`
```

When the error is about something declared in the same file, the frame shows
that place too, under a `:::` line and a `---` underline — the `def` a call
gives the wrong number of arguments to, the class that has no such member:

```text
Error: Type error: Wrong number of arguments: expected 2, got 3 at 11:1
  --> report.sl:11:1
   |
11 | greet("a", "b", "c")
   | ^^^^^
  ::: report.sl:7:1
   |
 7 | def greet(name, greeting = "Hi")
   | --- declared here
   |
   = note: error[E0403] wrong-arity — `soli explain E0403`
```

See [`error-codes.md`](error-codes.md) for how to read a frame, every code,
and the `--format json` output of `soli check` and `soli lint`.

## Breakpoints

`debug()` stops the request where it stands and answers with the interactive
debug page, holding a snapshot of the locals at that line.

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

`debug()` is a stop, not a pause: the request unwinds there and cannot be
resumed or stepped, and the REPL runs against the snapshot, not the live frame.
Only `debug()` on a line of its own is a breakpoint — `x = debug()` just stores
a marker value.

**Remove it before you deploy.** Outside `--dev`, `debug()` is not ignored: the
request fails and the visitor gets the generic production error page with
status `500` (the breakpoint is kept out of the error log). Under `--dev` the
debug page answers `200`.

The VM does not compile `debug()`; a handler that calls it runs on the
tree-walker instead, with the same result.

> The builtin was once named `break()`. It was renamed because `break` is now a
> loop keyword.

The page gives you a REPL running in the breakpoint's context, a clickable stack
trace, a source viewer with the line highlighted, and a request inspector for
params, query, headers and body. It also opens automatically on an error.

## Step debugger (`soli dap`)

`soli dap` is a [Debug Adapter Protocol](https://microsoft.github.io/debug-adapter-protocol/)
server on stdin/stdout. An editor that speaks DAP launches a script through it
and can then stop, step and look inside it while it runs — a real pause, unlike
the `debug()` page above.

- **Breakpoints** on any line of the script or of a file it imports.
- **Step over, into and out**, continue, and pause a running program.
- **The call stack**, with each frame's line, and for each frame its
  **Locals** and the program's **Globals** (builtins left out). Arrays, hashes
  and objects expand; a name that looks like a secret (`password`, `token`…)
  shows `[REDACTED]`.
- **Evaluate** in the paused frame, from the debug console or on hover. It
  runs real code in that frame's scope, so `total = 0` changes the variable.
- **`debug()` is a breakpoint** while a debugger is attached: the program
  stops there and resumes after it.
- What the program prints arrives in the editor's debug console.

The launch configuration takes `program` (the `.sl` script, required), `cwd`
and `stopOnEntry`.

**Neovim** ([nvim-dap](https://github.com/mfussenegger/nvim-dap)):

```lua
local dap = require("dap")
dap.adapters.soli = { type = "executable", command = "soli", args = { "dap" } }
dap.configurations.soli = {
  { type = "soli", request = "launch", name = "Debug this file",
    program = "${file}", cwd = "${workspaceFolder}" },
}
```

**Helix** (`languages.toml`):

```toml
[language.debugger]
name = "soli"
transport = "stdio"
command = "soli"
args = ["dap"]

[[language.debugger.templates]]
name = "script"
request = "launch"
completion = [{ name = "script", completion = "filename" }]
args = { program = "{0}" }
```

**For now:**

- It runs scripts (`soli script.sl`). Attaching to `soli serve` and debugging
  a spec come next.
- The program runs on the tree-walker, without native kernels, whatever
  engine `soli script.sl` would pick: that is the engine a debugger can stop.
- An outer frame's Locals are as they were at its last statement, the call
  it is waiting on.
- No conditional breakpoints or logpoints yet, and the VS Code extension
  does not declare the debugger yet: use an editor above, or any DAP client.
- On Windows the program's output is not captured, and can disturb the
  protocol.

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
| fn | slate | every Soli function call, on the VM and the tree-walker alike |

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

`fn` spans come from both engines: an action the VM runs shows its own
functions, methods and blocks, as a tree-walked one does. A typed numeric
function compiled to a native kernel opens no frame, so its time stays in its
caller's span.

## Profiling scripts and tests

`--profile` runs a script, `-e` code or a test suite under a sampling profiler
and prints the hottest functions when it ends:

```bash
soli report.sl --profile
soli test --profile
soli test tests/models --profile --profile-out models.json
```

```text
Profile: 1840 ms sampled over 1843 ms wall time, every 1000 µs
   self ms  self%   total ms total%  function
    1610.0  87.5%     1620.0  88.0%  score (report.sl:12)
     150.0   8.2%     1790.0  97.3%  rank (report.sl:30)
      60.0   3.3%     1840.0 100.0%  <main>
Profile written to soli-profile.folded
```

| Flag | Meaning |
|---|---|
| `--profile` (or `--profile=cpu`) | turn the profiler on |
| `--profile-out FILE` | where to write the profile (default `soli-profile.folded`); implies `--profile` |
| `--profile-hz N` | samples per second, 1 to 10 000 (default 1000); implies `--profile` |

The file is **folded stacks** — one `<main>;outer (app.sl:3);inner (app.sl:9) 42`
line per distinct stack — unless its name ends in `.json`, which writes a
**speedscope** profile. Load either into [speedscope.app](https://www.speedscope.app),
or feed the folded file to `flamegraph.pl` or `inferno-flamegraph` for an SVG.
On a script the flags may come before or after its name (`soli app.sl
--profile`), up to the first argument that belongs to the script.

How it works, and what that means for reading it:

- **Wall time, both engines.** A sampler thread ticks `--profile-hz` times a
  second; the engine charges the ticks to the Soli stack it is in at its next
  function entry or exit, tree-walker statement or VM loop check. A script
  waiting on the database or the network shows that wait, charged to the
  function that made the call.
- **Safepoint bias.** Time lands where the engine next checks, not at the
  instant the tick fired, so a very short function can absorb the time of the
  native call just before it returns. Totals are right; single-line precision
  is not the point.
- **What has a frame.** Functions, methods, constructors, lambdas and blocks.
  A typed numeric function compiled to a native kernel runs without a frame and
  is charged to its caller. `<main>` is the top level of the script.
- **`soli test`.** Every spec file's top level is its own `<main> (file)` root,
  and the specs' worker threads are added together, so the total can exceed the
  wall time. Request specs run their controllers in a separate test-server
  process, which is not profiled. `--profile` cannot be combined with `--watch`.

With no `--profile`, the hooks cost one relaxed atomic load per call and per
statement, and `soli serve` never turns them on.

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

## Attaching a coding agent (`soli agent`)

`soli agent` lets a coding agent (Claude Code, Cursor, Codex CLI, any MCP client)
read what the dev bar shows for the running `soli serve --dev`: the recent
requests, each one's queries with binds and durations, the N+1 alert, and the
open error groups. It can also replay a request, run one spec, and use the dev
REPL. The agent edits files itself; `soli agent` writes nothing in the app.

It is an [MCP](https://modelcontextprotocol.io) server on stdin/stdout: the
client starts it, and it opens no port. Declare it in your client from the app
root — for Claude Code, a `.mcp.json` next to `soli.toml`:

```json
{
  "mcpServers": {
    "soli": { "command": "soli", "args": ["agent"] }
  }
}
```

Start `soli serve . --dev` first. When it binds, the dev server writes
`.soli/dev-server.json` (pid, port, app root and the dev REPL token, mode
`0600`) and removes it on shutdown; `soli new` gitignores it. `soli agent` reads
that file from the current directory and exits with `dev server is not running`
when it is missing or its process is gone — it never scans ports. It then calls
the dev server on `127.0.0.1` through the same `/__solidev/*` and `/__dev/repl`
endpoints the dev bar uses, so the same gates apply: loopback peer, local
`Host`, same-origin POST, REPL token.

| Tool | What it returns |
|---|---|
| `requests` | The last 64 requests, newest first: id, method, path, status, `elapsed_us`, route, query count |
| `request` | One request: phase timings, middleware and template timings, every query (statement, binds, duration), outgoing HTTP and KV calls, `n_plus_one` when the dev bar flags one |
| `replay` | Re-runs a request (see **Replay** above) and returns the new request id and status. Anything but `GET`/`HEAD` needs `confirm: true` |
| `errors` | Open [error groups](observability.md#error-tracking-__solierrors) with their latest sample: message, stack, a `curl` line, locals — redacted when stored |
| `graph` | `soli graph query --json` for a question (needs `soli graph build` and SolidB) |
| `test` | `soli test <path>` for one spec file or directory, never the whole suite; failures only, 120 s timeout, 20 failures / 8 KB at most |
| `repl` | Runs Soli code inside the dev process — models, config, database. Pass `repl_session_id` back to keep variables |

The JSON behind `requests`, `request` and `errors` is served at
`GET /__solidev/requests`, `GET /__solidev/requests/:id` and
`GET /__solidev/errors`, dev-only like the rest of `/__solidev/`. It carries no
request headers, cookies or bodies: only what the dev bar and the error page
already show.

A typical loop: `requests` shows `GET /orders` at 400 ms with 53 queries;
`request` shows 50 of them are the same `customers` lookup; the agent adds an
`includes(...)`; `replay` of the same id shows 4 queries; `test` on
`tests/controllers/orders_controller_spec.sl` passes.

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
| Breakpoints | the debug page (`200`) | the request fails (`500`) |
| Debug page | full, interactive | a simple error page |
| Hot reload | enabled | disabled |
| Stack traces | detailed | minimal |
| `dev_queries()` | populated | always `[]` |
| `dev_mode?()` | `true` | `false` |
| Flamegraph, requests panel, inbox | present | not mounted |

`dev_mode?()` is how the app asks which of the two it is — to serve a screen only
while it is being built, for instance. It reads the `--dev` flag, not an
environment variable, so a `.env` copied to production cannot turn it on, and
`--dev` itself is refused when `APP_ENV` names production. `soli test` runs its
server without `--dev`, so it answers `false` in specs; a route gated on it needs
a second, test-only door if its specs are to reach it.

```soli
def before_action(req)
  return {"status": 404, "body": ""} unless dev_mode?()

  req
end
```

The dev bar is injected into `text/html` responses only, never into JSON, and
never in production — where no `X-Soli-Route` header is sent and no script is
added, so there is nothing to pay for.

## See also

- [`linting.md`](linting.md) — `soli lint` and `soli check`
- [`error-codes.md`](error-codes.md) — code frames, error codes, `soli explain`
- [`mailer.md`](mailer.md) — the full inbox tour
- [`observability.md`](observability.md) — production logs, metrics and traces
- Rendered page: `/docs/development-tools/debugging`
