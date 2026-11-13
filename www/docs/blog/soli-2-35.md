# Soli 2.35: the bugs an agent tripped on

For the video on the [home page](/#agents), Claude Code built a yoga studio
booking site from an empty `soli new`. It took four prompts and 28 minutes, and
the result has 151 passing specs, Stripe Checkout and live seat counts. Before
that take there were two dry runs, and each time the agent tripped on something.
It never wrote anything absurd. It wrote what the docs said, or what a Ruby
developer would write, and Soli did something else.

Most of 2.35 is that list, fixed. The rest is `soli agent`, which gives a coding
agent what the dev bar shows.

<figure style="margin:1.5rem auto;max-width:1024px;">
  <img src="/images/blog/soli-2-35.svg" width="1024" height="576" alt="Four prompts of a Claude Code session on the left; on the right, the eight places where it tripped, each fixed in 2.35: class constants by name, try as a value, nested interpolation, bare save, a fresh app's .env.test, scaffold specs that pass, mailer views that don't double-escape, and validations replaced on a dev reload." style="display:block;width:100%;height:auto;border-radius:12px;border:1px solid #30363d;background:#0b0d0f;">
  <figcaption style="text-align:center;color:#8b949e;font-size:0.875rem;margin-top:0.5rem;">Where the agent tripped over two dry runs and one take, and the release that fixed each.</figcaption>
</figure>

## Class constants, read by name

The agent put the studio's rules on the model, which is where they belong:

```soli
class Studio
  const MAX_SEATS = 12

  def seats_left(booked)
    MAX_SEATS - booked
  end
end

studio = Studio.new()
print(studio.seats_left(4))     # 8
print(Studio.MAX_SEATS)         # 12
print(Studio::MAX_SEATS)        # 12
```

Before 2.35, a `const` in a class body was only an instance field. `@MAX_SEATS`
read it, but a bare `MAX_SEATS` in a method was an undefined variable, and
`Studio.MAX_SEATS` raised. Now the class's methods (instance and static), its
constructor and the blocks inside them read the constant by name, nested classes
included. Outside the class it reads as `Studio.MAX_SEATS` or
`Studio::MAX_SEATS`. A parameter or a local with the same name still wins. This
works on both engines.

A related problem: on the second dry run, `soli fmt` turned those constants into
`static const` and rewrote every reference as `Studio.MAX_SEATS`. It now leaves
them alone.

## `try` is a value

```soli
label = try
  JSON.parse(raw)
catch error
  "unparseable"
end
```

That did not parse before. It does now, and a `try` that ends a method returns
the value of whichever branch ran. Until now it returned `nil` on both engines,
so a method like this one silently answered `nil`:

```soli
def port_from(raw)
  try
    raw.to_i
  catch error
    8080
  end
end
```

`finally` still runs, and contributes no value.

**Behaviour change:** if you have a method that ends in a `try` and you counted
on it returning `nil`, it now returns the branch's value.

## Nested interpolation, and a bare `save`

```soli
print("Booked by #{"#{first} #{last}".upcase}")    # Booked by GRACE HOPPER
```

A string inside `#{…}` that interpolates in its turn used to stop the lexer with
"Unterminated string". It now lexes, and `soli fmt` keeps it intact.

Inside a model method, a bare `save` now calls `Model#save`. Before, it raised
"Undefined variable 'save'", even though a bare call to your own `_helper`
worked. And `soli fmt` no longer rewrites `"#{@name}"` into
`"#{this.name}"`.

## A fresh app tests out of the box, at 100%

The first thing an agent does after `soli new` is run the tests, and in 2.34
`soli test` refused to start because there was no `.env.test`. `soli new` now
writes one, pointing at an `<app>_test` database that the runner creates,
truncates and drops:

```
$ soli new studio && cd studio && soli test
  3 files passed, 0 failed (3 total)
  10 tests
Coverage: 100.0% (59/59, threshold: 80%) ✓
```

It also writes specs for everything it generates: the home controller, the
application helper, the sample `authenticate` middleware and, with `--eui`, the
counter component. A new app reports 100% instead of 63% (or 8.9% with
`--eui`).

The scaffold is held to the same bar. The controller spec that
`soli generate scaffold` writes used to assume a record with id `1`, posted a
`name` field the resource didn't have, and checked `@posts` on the show page.
It now creates the records it reads, from a sample value per field, and passes on
a fresh app:

```
$ soli generate scaffold lesson title:string seats:integer && soli test
  4 files passed, 0 failed (4 total)
  22 tests
Coverage: 100.0% (94/94, threshold: 80%) ✓
```

A file can opt out of the report with `# coverage: off` in its leading comments,
and specs can now call a controller's top-level `def`s the way they call a
helper's.

## Generators that write code that runs

Three generated files were wrong in ways an agent copies faithfully:

- The scaffolded model called `before_save("normalize_fields")` on a method that
  didn't exist. The line is gone.
- The mailer view, the OIDC views and the generated agent guides wrote
  `<%= h(…) %>`, which escapes twice. `<%=` already escapes.
- The scaffold summary listed `post_controller.sl` for a file it had written as
  `posts_controller.sl`.

The Stripe, SendGrid and background-jobs posts on this blog had the same
problem, worse. They used `raise` (Soli has `throw`), read `HTTP.post` as a
status hash (it returns the body), sent buyers to Stripe with `redirect` instead
of `redirect_external`, and called a dozen names that don't exist. The agent
read the Stripe post first and inherited all of it. They have been rewritten,
and they run.

## An edited validation replaces the old one on reload

The last bug turned up while recording the video itself. The agent had written
a "class is full" rule, then narrowed it to new bookings with `on: "create"`.
The dev server reloaded the model and added the new rule beside the old one. So
the old rule, which also ran on updates, kept refusing the webhook's update
that marks a booking paid, until the server was restarted.

The repro is a validation that fails on any update:

```soli
class Thing < Model
  validates("name", {"custom": "only_on_create"})
  # ...
end
```

```
GET /probe
{"saved": false, "errors": [{"field": "name", "message": "ran on update"}]}

# add "on": "create" to the validates call, save the file, no restart

GET /probe
{"saved": true, "errors": []}
```

In 2.34 the second request still failed. A model's validations are now built
along with its callbacks and installed whole on every reload.

## `soli agent`: what the dev bar shows, for the agent

The dev bar already had what an agent needs to debug a slow or failing page:
each request's queries with binds and durations, the N+1 alert, and the open
error groups. But it showed them in a browser, where the agent wasn't.
`soli agent` is an MCP server on stdin/stdout that Claude Code, Cursor or any
MCP client starts from the app root:

```json
{
  "mcpServers": {
    "soli": { "command": "soli", "args": ["agent"] }
  }
}
```

It opens no port. When `soli serve --dev` binds, it now writes
`.soli/dev-server.json` (pid, port, app root and dev REPL token, mode `0600`,
gitignored by `soli new`) and removes it on shutdown. `soli agent` reads that
file and calls the server on `127.0.0.1` behind the same gates as the dev bar.
Without a running server it says so and exits; it never scans ports:

```
dev server is not running (no live `soli serve --dev` found via …/.soli/dev-server.json; start it from the app root)
```

It has seven tools:

- `requests` and `request` give the dev bar's snapshot as JSON.
- `errors` gives the open error groups, with their redacted latest sample.
- `replay` re-runs a request; anything but `GET` needs `confirm: true`.
- `graph` runs `soli graph query`.
- `test` runs one spec and returns only its failures.
- `repl` runs code inside the dev process.

A typical loop:

1. `requests` shows `GET /orders` at 400 ms with 53 queries.
2. `request` shows that 50 of them are the same lookup.
3. The agent adds an `includes(...)`.
4. `replay` of the same request shows 4 queries.
5. `test` passes.

`soli agent` writes nothing in the app. The agent still edits the files itself.
The full reference is in
[Attaching a coding agent](/docs/development-tools/debugging#soli-agent).

## Also in 2.35

- **Security:** the bundled SQLite is updated. `rusqlite` 0.41 brings
  `libsqlite3-sys` 0.39.1 (RUSTSEC-2026-0459, a CVE in C SQLite), and `rustix`
  goes to 1.1.6 (RUSTSEC-2026-0486).
- **`soli fmt`:**
  - a comment after the last line of a block stays inside the block (it used to
    move below the `end`, where the next pass attached it to the code that
    followed);
  - `soli fmt --help` prints its usage.
- **Scaffold:** the index page is titled after the resource (`"Posts"`, not
  `"PostsController"`).
- **`soli new`:** the "Get started" box lines up.
- **[Benchmarks](/docs/getting-started/benchmarks):** re-measured with Soli
  2.34.0 on SoliDB 2.9.3, with Rails on Shuttlebay added as a reference row.

The complete list is in the
[changelog](/docs/getting-started/changelog#v2-35-0).

## Why it went this way

A framework that a person learns gets a little slack: you read the error, you
learn the quirk, and you route around it next time. An agent doesn't learn the
quirk. It reads the docs and the generated code, takes both at their word, and
writes the same workaround in every app. So every place where Soli and its own
docs disagreed came out of these runs as a bug report.

Every snippet and output above was run against the 2.35.0 binary.
