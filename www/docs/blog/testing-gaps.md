# Closing the Gaps: What Soli Was Missing, and What Changed

Our comparison page has a section called "Honest Gaps": the things a Rails,
Django or Phoenix developer will look for in Soli and not find. It said the test
runner had no filtering, no fail-fast and no mocking; that migrations rolled back
one step at a time; that uploads were buffered in memory with no resumable path;
that a LiveView could not be updated from another process; that the OpenID
Connect provider stopped at code-plus-PKCE; that "sign in with" covered two
providers; and that a component was a template and nothing more.

A gap list is only useful if it gets shorter, so we went through it, one entry at
a time. Before writing anything we checked each against the source, because a
list like that goes stale quietly: OpenTelemetry export and `find_each`, both on
the backlog, were already built, and their task files were the stale part. The
rest were real. This post covers what closed, what we decided not to build, and
what the work turned up along the way.

## Testing

```bash
soli test --filter "creates a post"   # or -n checkout
soli test --fail-fast
soli test --watch --filter Checkout
```

`--filter` matches a test's full description (the `describe` names, then the test
name) case-insensitively. `--fail-fast` stops scheduling tests and files after the
first failure; it is a process-wide flag, so it works across `--jobs` workers.
`--watch` runs the suite in a **fresh child process** each time, because an
interpreter that has already loaded your models carries their state, and a watch
mode that quietly reuses it gives answers a real run would not.

### Mock, stubs and spies

```soli
gateway = new Mock("gateway", {
  "charge": fn(amount) { {"ok": true, "amount": amount} },
  "currency": "EUR"
})
gateway.assert_received("charge", [1200])

# a real class the code under test calls directly
Mock.allow(Gateway).to_receive("charge").and_return({"ok": true})
spy = Mock.allow(Gateway).to_receive("charge").and_call_original
Mock.allow_any_instance(User).to_receive("save").and_return(true)
```

`Mock` is written in Soli and embedded in the binary. It exists only when
`APP_ENV=test`. Stubbing a real class does not patch method tables (Soli keeps
several, per engine): a per-thread table keyed by class and method is consulted
first on member access, at the cost of one relaxed atomic load when nothing is
stubbed, and the runner clears it after every test. A **spy** records the call on
its `Mock` and then runs the real method: the hook resolves the original with
itself switched off, and the wrapper re-enters the interpreter the way
`method_missing` does. A stub matches any arguments; there is no `with(...)`
filter, and a stub reaches the test process, not the server behind a request spec.

Building this exposed a limit: a double works through `method_missing`, which
bound only the parameters you declared. A catch-all declared `(name, a, b)` broke
on any call with fewer arguments. Instance `method_missing` now follows the
class-level form: a last parameter named `args` receives every remaining argument.

## Migrations

```bash
soli db:migrate down --step 3     # or STEP=3
soli db:migrate down --to 20240101000000
soli db:migrate redo
soli db:migrate reset
```

The safety check that refuses to roll back when an applied version has no file on
disk still runs before every step.

## Uploads

Three separate things were on the list.

**LiveView uploads pause and resume.** A `[soli-upload-pause]` button holds the next
chunk. The client remembers each chunked upload in `localStorage`, keyed by field,
name, size and modified time; picking the same file again asks
`GET /live/upload/status`, gets `{"received": [0, 1, 3]}`, and sends only what is
missing. A browser spec proves it: with two of three chunks already on the server,
one request crosses the wire. Partial uploads are now kept ten minutes instead of
two, which is the window resume needs; the per-session and global caps, not the
timeout, are what bound memory. A longer window must not let parked uploads hold
those caps against everyone else, so an upload idle for two minutes gives its room
up to a new one that would otherwise be refused.

**Resumable uploads for attachments (tus 1.0.0).** `resumable_uploads()` in
`config/routes.sl` opens a tus endpoint. Chunks go to disk as they arrive, so
memory is one chunk rather than the file. Raw `PATCH` bytes could not ride the
request model (a UTF-8 `body`), so they travel as one file part, which is how
binary bodies already did. `tus_take(id)` then `attach_<field>` renames the file
into a disk attachment or streams it to S3 a megabyte at a time; only an uploader
that needs the bytes (SoliDB blobs, an image transform) loads it, capped at 64 MiB.

**Direct to S3.** `direct_upload_start` returns a presigned `PUT`;
`direct_upload_finish` asks the bucket what it holds and attaches it. We first
wrote in a comment that the signature pins the size and type. It does not: the
presigner signs the host and the metadata header only, so S3 accepts whatever is
sent to that URL. The control is therefore at finish: the bucket's own `HEAD`
answer is checked against the uploader's limits, and an object that breaks them is
deleted. We caught it because the test asked what `X-Amz-SignedHeaders` said.
Finish also accepts only an id that start issued to the same session for the same
field: otherwise any blob id read off a public URL could be claimed, and deleted
by the size check or the next replace.

## LiveView from anywhere

```soli
live_update("board", { "note": "deploy finished" })
live_update("board", { "score": 9 }, { "room": "lobby" })
live_update("board", { "id": "main", "score": 9 }, { "child": "score" })
```

`send_update` reaches the view whose handler is running. `live_update` reaches any
attached instance from a controller, a job, or another OS process. It delivers to
local instances at once, then publishes to a `_live_bus` collection whose SoliDB
changefeed every process declaring a `router_live` component subscribes to. We
tested it with two `soli serve` processes on one database: a `POST` to one pushed a
patch to a WebSocket held on the other. It is fire and forget, as a Phoenix PubSub
broadcast is; the changefeed can drop an event for a lagging subscriber, so send
state the next update will restate.

Worth knowing: a client can send any event name over its socket, including the
internal one, so worker-side delivery checks a per-process secret token that only
the server holds.

## The OpenID Connect provider

`soli generate oidc_provider` gained:

- **`client_credentials`**, opt-in per confidential client.
- **Token introspection** (RFC 7662): `{"active": false}` and nothing else for a
  dead token, confidential clients only.
- **Dynamic client registration** (RFC 7591), off until you set
  `SOLI_OIDC_REGISTRATION_TOKEN`. That value is the initial access token, and a
  registrant can never request machine grants, `skip_consent`, or scopes outside the
  provider's list.
- **The device authorization grant** (RFC 8628) with an approval page, `slow_down`
  handling and single-use codes.
- **Signed request objects** (JAR, RFC 9101), verified with a per-client key; the
  object's claims replace the query, so nothing outside the signature steers the
  request.

We ran each against a real SoliDB, with a browser session for the approval page.
The first pass of `client_credentials` was wrong: it inherited a check against the
OIDC scope list, so `scope=read` was refused. Only exercising it end to end found
that.

**Implicit and hybrid flows are left out on purpose.** They return tokens in the
redirect URL, where history, logs and referrers keep them, and OAuth 2.1 removes
them.

## Sign in with…

`soli generate oauth` now supports GitLab, Discord, LinkedIn and Microsoft besides
GitHub and Google. The controller finds `<Name>Oauth` by class name, so a new
provider never means editing it. Microsoft requires `MICROSOFT_TENANT`, because it
does not verify the `email` claim and a multi-tenant app can be handed someone
else's address; the service refuses `common`, `organizations` and `consumers`.

Running the flow also found a bug older than any of this: the generated
`/auth/:provider` redirect used `redirect()`, which only accepts local paths, so it
returned 500 for GitHub and Google too.

## Component classes

```soli
# app/components/stats_card_component.sl
class StatsCardComponent
  label: String
  value: Int

  def before_render
    @label = "Untitled" if @label.blank?
  end

  def tone
    @value > 1000 ? "text-green-500" : "text-gray-400"
  end
end
```

`components/stats_card.html.slv` then reads `this.tone()`. The props become the
instance's fields, an optional `before_render` derives values, and a component with
no class renders as before. `soli generate component stats_card --class` writes
both.

## What we did not build

- **`request_uri`** for request objects: fetching a caller-supplied URL is an SSRF
  door.
- **tus `checksum` and `concatenation`**, and S3 multipart for files over 5 GB.
- **In-process TLS.** It is a deliberate trade-off: we front Soli with a reverse
  proxy, and terminating TLS in the worker would change how connections are
  accepted, so it deserves its own design rather than a flag.
- **Verified doubles** that check a stub against the real signature.

## Try it

```bash
soli test --watch --fail-fast --filter "Checkout"
```

References: [Testing](/docs/testing), [Migrations](/docs/database/migrations),
[Models: large uploads](/docs/database/models#large-uploads),
[LiveView](/docs/core-concepts/liveview#live-update),
[OIDC provider](/docs/security/oidc-provider),
[OAuth client](/docs/security/oauth-client) and
[Views: component classes](/docs/core-concepts/views#component-classes).
