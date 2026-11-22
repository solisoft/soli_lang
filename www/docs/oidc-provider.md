# OpenID Connect Provider

Soli can *be* an identity provider, not just talk to one. `soli generate
oidc_provider` scaffolds a working OpenID Connect provider implementing the
**Authorization Code flow with PKCE** — the only flow OAuth 2.1 still
recommends — so other applications can "Sign in with <your app>".

```bash
soli generate auth            # the provider signs tokens about a signed-in user
soli generate oidc_provider
```

The second command refuses to run without the first: the provider needs a
`User` model and the `current_user` middleware to know who is being
authenticated.

## What it generates

| Path | Purpose |
|------|---------|
| `app/services/oidc_config.sl` | Issuer, keys, TTLs, scopes, claim mapping — every knob lives here |
| `app/services/oidc_helper.sl` | Response builders, JWK export, token minting |
| `app/models/oauth_client.sl` | Registered relying parties |
| `app/models/oauth_authorization_code.sl` | Single-use codes + the atomic burn |
| `app/models/oauth_refresh_token.sl` | Rotation + reuse detection |
| `app/models/oauth_consent.sl` | Remembered user→client grants |
| `app/models/oauth_revocation.sl` | Access-token (`jti`) denylist |
| `app/controllers/oidc_discovery_controller.sl` | Discovery + JWKS |
| `app/controllers/oauth_authorizations_controller.sl` | Authorize + consent |
| `app/controllers/oauth_tokens_controller.sl` | Token + revoke |
| `app/controllers/oauth_userinfo_controller.sl` | UserInfo |
| `app/controllers/oauth_sessions_controller.sl` | RP-initiated logout |
| `app/views/oauth_authorizations/{new,error}.html.slv` | Consent screen, non-redirectable errors |
| `db/migrations/*_add_oauth_indexes.sl` | Collections + the unique indexes |

Both service files sit in `app/services/` rather than `app/helpers/`, because
`app/helpers/` is only visible to view templates while `app/services/` is
loaded into the global scope alongside models — which is where controllers can
reach it.

## Endpoints

| Route | Purpose |
|-------|---------|
| `GET /.well-known/openid-configuration` | Discovery document |
| `GET /.well-known/jwks.json` | Public signing keys |
| `GET /oauth/authorize` | Authorization + consent screen |
| `POST /oauth/authorize` | Consent decision |
| `POST /oauth/token` | Code, refresh and client-credentials grants |
| `GET\|POST /oauth/userinfo` | Claims for a bearer token |
| `POST /oauth/revoke` | RFC 7009 revocation |
| `POST /oauth/introspect` | RFC 7662 token introspection |
| `POST /oauth/register` | RFC 7591 dynamic client registration (off unless a token is set) |
| `POST /oauth/device_authorization` | RFC 8628 device grant, step 1 |
| `GET\|POST /oauth/device` | RFC 8628 device grant, the person's approval page |
| `GET /oauth/logout` | RP-initiated logout |

> The generated routes call `skip_csrf` on `/oauth/token`, `/oauth/revoke`, `/oauth/introspect`, `/oauth/register` and `/oauth/device_authorization`.
> Those are server-to-server calls with no browser `Origin`, so the same-origin
> gate would reject every legitimate exchange; the *client* is authenticated
> instead. The consent `POST` is a real browser form and keeps CSRF protection.

## Setup

### 1. Signing keys

The generator does **not** create keys — a private key written into `config/`
ends up committed. Generate them yourself:

```bash
openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out oidc.pem
openssl rsa -in oidc.pem -pubout -out oidc.pub.pem
```

```bash
SOLI_OIDC_ISSUER=https://id.example.com
SOLI_OIDC_PRIVATE_KEY="$(cat oidc.pem)"
SOLI_OIDC_PUBLIC_KEY="$(cat oidc.pub.pem)"
```

Both halves are configured because signing needs the private PEM and JWKS
publication needs the public one. `SOLI_OIDC_ISSUER` **must** match your public
origin exactly, with no trailing slash — relying parties compare the `iss`
claim byte for byte. Unset, it is `http://localhost:5011` in development, and
with `APP_ENV=production` every request that needs it fails rather than
publish that issuer. It is never read from the `Host` header.

### 2. Migrations

```bash
soli db:migrate up
```

SoliDB creates a collection on first model access, so the migration exists for
the **indexes**. The unique ones are not an optimisation: `code_digest` unique
is the database-level backstop that makes an authorization code single-use even
if the application-level burn ever raced.

### 3. Register a relying party

```soli
result = OauthClient.register("My App", ["https://app.example/callback"], {})
print(result["client_id"])
print(result["client_secret"])   # shown once — only its Argon2 digest is stored
```

Options: `client_type` (`"confidential"` | `"public"`), `scopes`,
`grant_types`, `require_pkce`, `skip_consent`, `request_object_key`,
`introspect_any` (a resource server allowed to introspect tokens issued to other
clients — see below), `post_logout_redirect_uris` (where `/oauth/logout` may
send the browser back to; left out, the client's `redirect_uris` serve, as they
did before the option existed — `[]` means nowhere). Every URI must be `https`,
or plain `http` to this machine only (`localhost`, `127.0.0.1`, `[::1]`) — the
rule dynamic registration applies.

**Machine-to-machine access.** Add `"client_credentials"` to a confidential client's
`grant_types` (it is off by default) and it can `POST /oauth/token` with
`grant_type=client_credentials` and an optional `scope`, authenticating with
Basic or form credentials. The access token's `sub` is the client itself and it
carries `gty: "client-credentials"`. There is no user, so no id token, no refresh
token, and `openid` / `offline_access` are refused; a scope the client does not
hold is `invalid_scope`, a public client is `unauthorized_client`, and the
userinfo endpoint answers 401 for such a token.

A **public** client (SPA, mobile) has no secret, so PKCE is forced on and
cannot be switched off — it is the only thing binding the code to the
requester.

### 4. Record when the user signed in, and come back mid-flow

**Every path that signs a user in** records the moment, next to its
`session_set("user_id", …)`. Apps generated after 2.39.3 already do it in each
scaffold below (and impersonation already deletes it); in an app generated
earlier, the generator does not modify files it did not create, so make these
edits yourself:

```soli
session_set("auth_time", DateTime.utc.to_unix)
```

| Sign-in path | Where |
|---|---|
| Password | `SessionsController#create` (`app/controllers/sessions_controller.sl`), next to `session_regenerate()` |
| Two-factor | `TwoFactorController#verify` (`--two-factor`): the code check that finishes the sign-in |
| Sign-up | `RegistrationsController#create`, which signs the new account in |
| Password reset | `PasswordsController#update`, which signs the user in with the new password |
| Passkey | `PasskeySessionsController#create` (`--passkeys`) |
| Magic link | `MagicLinksController#redeem` (`--magic-link`) |
| SAML | `SamlController#acs` (`soli generate saml`) |
| OAuth provider | already done: `OauthSso.sign_in` ([OAuth Client](oauth-client.md)) |

A remember-me cookie that restores a session (`load_current_user`) is not a
sign-in: leave `auth_time` unset there, so `max_age` asks for a real one.

**Impersonation** (`--impersonation`) must `session_delete("auth_time")` when an
admin starts and when they stop impersonating: the session then belongs to
someone who did not sign in at that time, and a relying party asking `max_age`
must hear "unknown", not "fresh".

Then, in `SessionsController#create`, replace the final `return redirect("/")`
with:

```soli
destination = session_get("oidc_return_to") ?? "/"
session_delete("oidc_return_to")
return redirect(destination)
```

Without `auth_time` the provider does not know when the user signed in: the
id_token carries no `auth_time` (it used to claim the authorization instant,
which told a client that a day-old session was fresh), and a request with
`max_age` is answered `login_required` — a missing value counts as unknown,
never as now. A path that forgets it therefore fails closed for `max_age`
clients, while one that sets it and is not a real sign-in (impersonation) would
fail open: that is why impersonation deletes it. Without the redirect edit, a
user who signs in mid-flow lands on the home page instead of completing the
authorization.

**`prompt` and `max_age`.** `prompt=none` answers `login_required` when no one
is signed in and `consent_required` when consent is missing, without showing a
page. `prompt=consent` shows the consent screen even when consent was given
before or the client has `skip_consent`. `max_age=N` is honoured from the
recorded `auth_time`: an older (or unrecorded) sign-in is answered
`login_required`, on the consent form's POST as on the GET, so a consent page
left open issues no code once the sign-in is past `max_age`. `prompt=login` is always answered `login_required`: the
provider cannot make the person sign in again on its own, and OIDC Core
§3.1.2.1 prefers that error to silently ignoring the request.

## More grants and endpoints

### Token introspection (RFC 7662)

`POST /oauth/introspect` with `token=…` (and optionally `token_type_hint`) answers
`{"active": true, "scope", "client_id", "sub", "exp", …}` for a live access or
refresh token, and `{"active": false}` — nothing else — for anything that is not.
The caller must be an authenticated **confidential** client (Basic or form
credentials); an anonymous introspection endpoint is a token-validity oracle.
A client introspects **only the tokens issued to it**: another client's token
answers `{"active": false}`, or any confidential client could read whose token
it is and what it grants. A resource server that checks tokens issued to other
clients is registered by an admin with `"introspect_any": true` (dynamic
registration cannot set it). A refresh token whose user's session version moved
since it was issued (below) answers `{"active": false}`, as the token endpoint
would refuse it. Discovery advertises `introspection_endpoint`.

### Dynamic client registration (RFC 7591)

Off by default: `POST /oauth/register` answers 404 until you set
`SOLI_OIDC_REGISTRATION_TOKEN`. That value is the *initial access token* — send it
as `Authorization: Bearer …` — so only callers you gave it to can register
clients. Body, as JSON:

```json
{"client_name": "My RP", "redirect_uris": ["https://rp.example/cb"],
 "scope": "openid email", "token_endpoint_auth_method": "client_secret_basic"}
```

The `201` reply carries the `client_id` and, unless `token_endpoint_auth_method`
is `none` (a public client, PKCE forced on), a `client_secret`. Whatever the
metadata says, a registering caller cannot request `client_credentials` or the
device grant, cannot set `skip_consent` or `introspect_any`, and cannot ask for
scopes outside `OIDC_SUPPORTED_SCOPES`. Redirect URIs must be `https`, or `http`
to this machine only — `127.0.0.1`, `[::1]` or `localhost` (RFC 8252 §7.3, for
native apps and development) — without a fragment: a plain-http redirect to a
remote host sends the authorization code across the network in the clear. The
loopback host must be the whole host: after it only an optional `:port` and a
path or query may follow, and a backslash is refused anywhere — so
`http://localhost@evil.example/`, `http://localhost.evil.example/`,
`http://[::1]evil.example/` and `http://localhost:80\evil.example` (a browser
reads the backslash as a slash) are all refused. `post_logout_redirect_uris`,
optional, is held to the same rule; a client registered this way is sent back
after logout only to those, never to its `redirect_uris`. Discovery advertises
`registration_endpoint` only when enabled.

### Device authorization grant (RFC 8628)

For a TV or a CLI. Opt a client in with
`"grant_types": ["urn:ietf:params:oauth:grant-type:device_code", "refresh_token"]`
(public clients are fine). The device:

1. `POST /oauth/device_authorization` with `client_id` and `scope` gets
   `device_code`, `user_code` (`WDJB-MJHT`), `verification_uri`,
   `verification_uri_complete`, `expires_in` and `interval`.
2. Shows the user code. The person opens `/oauth/device`, signs in, types the code,
   and approves or denies.
3. Meanwhile the device polls `POST /oauth/token` with
   `grant_type=urn:ietf:params:oauth:grant-type:device_code` and its `device_code`,
   and receives `authorization_pending`, `slow_down` (poll faster than `interval`
   and the interval grows by 5 s), `access_denied`, `expired_token` or the tokens.

The device code is stored as a digest and consumed atomically, so a code yields
tokens exactly once. User codes use an alphabet without vowels or look-alikes;
`OIDC_DEVICE_CODE_TTL` and `OIDC_DEVICE_INTERVAL` live in `oidc_config.sl`.

### Signed request objects (JAR, RFC 9101)

Register the client's public PEM as `request_object_key`
(`OauthClient.register(name, uris, {"request_object_key": pem})`) and it can send
`GET /oauth/authorize?client_id=…&request=<jwt>`. The JWT is RS256, `iss` the
client id, `aud` your issuer, and its claims (`redirect_uri`, `scope`, `state`,
PKCE…) **replace** the query parameters, so nothing outside the signature can steer
the request. Any failure renders an error page rather than redirecting.
`request_uri` is not supported: fetching a caller-supplied URL is an SSRF door.

## The flow

```
GET /oauth/authorize?response_type=code&client_id=…&redirect_uri=…
    &scope=openid+email&state=…&nonce=…
    &code_challenge=…&code_challenge_method=S256
  → not signed in?  redirect to /login, come back after
  → consent already given (or skip_consent)?  redirect with ?code=…&state=…
  → otherwise render the consent screen

POST /oauth/token
  grant_type=authorization_code&code=…&redirect_uri=…&code_verifier=…
  Authorization: Basic base64(client_id:client_secret)
  → { access_token, token_type, expires_in, scope, id_token, refresh_token }
```

The PKCE challenge is the unpadded base64url of the SHA-256 of the verifier:

```soli
challenge = Base64.urlsafe_encode(Hex.decode(Crypto.sha256(code_verifier)))
```

`Crypto.sha256` returns hex, so it must be decoded to raw bytes first —
encoding the hex *text* produces a different value of twice the length that no
relying party will accept.

## Tokens

**Access tokens are signed JWTs** (`typ: "at+jwt"`, RFC 9068), so a resource
server verifies them offline against the JWKS with no call back to the
provider. The cost is that revoking a grant kills the *refresh* token
immediately while an already-issued access token lives out its TTL. That TTL is
deliberately short (10 minutes) because it, not the denylist, is what bounds
the exposure. The `oauth_revocations` table closes the window for the
provider's own endpoints.

**Refresh tokens are opaque and rotate.** Every use issues a new one and
retires the old. Presenting a retired token means it leaked, so the entire
family is revoked — the legitimate holder loses access too, which is correct,
because the provider cannot tell the two apart.

A refresh token is only issued when the client asked for `offline_access`.

**A password reset ends the grants.** An authorization code, a device
approval and a refresh token each record the user's `session_version` when they
are made; once that has moved — a password reset, an account taken over from an
unconfirmed squatter, anything that signs every session out — the code exchange
or device poll is refused `invalid_grant`, and so is the next refresh (its whole
family revoked); introspection reports such a refresh token inactive. A refresh
token minted from a grant carries the grant's version, never the user's current
one, so a code issued before a reset cannot launder itself into a fresh-looking
token. Rows written before the field existed count as version 0. Without this, a
squatter's code, issued minutes before the address's owner took the account
over, still turned into tokens for it.

## id_token claims

`iss`, `sub`, `aud`, `exp`, `iat`, plus `nonce` (echoed from the authorization
request), `auth_time`, and `at_hash` — the left-most 128 bits of the SHA-256 of
the access token, which lets a client detect an access token swapped in from a
different response. Scope-gated claims come from the
`oidc_user_claims` hook in `app/services/oidc_config.sl`; that is the one place
that needs to know what a `User` looks like. `profile` releases `name` (and
`preferred_username` from a `username` field, if the user has one); the address
is released only under `email` (`email`, `email_verified`) — the consent screen
names them separately.

Tokens carry a `kid` header matching the JWKS entry, so relying parties select
the right key.

## Key rotation

```bash
# 1. keep the outgoing key published
SOLI_OIDC_PREVIOUS_PUBLIC_KEY="$(cat old.pub.pem)"
# 2. promote the new pair, deploy
SOLI_OIDC_PRIVATE_KEY="$(cat new.pem)"
SOLI_OIDC_PUBLIC_KEY="$(cat new.pub.pem)"
# 3. once OIDC_ID_TOKEN_TTL has elapsed, unset the previous key
```

The JWKS lists both keys during the overlap; signing always uses the active one
alone. This only works because tokens carry `kid`.

## Security notes

These are the decisions worth knowing about, because several of them are places
where a looser implementation would still appear to work.

**An access token dies with what it was granted under.** It carries the
session version of its grant (`sv`) and its refresh-token family (`cid`), and
`/oauth/userinfo` and `/oauth/introspect` refuse it once the user's sessions
were ended (a password reset, a takeover, a SCIM deactivation), the account is
gone or deactivated, or the family was revoked — a replayed code, a reused
refresh token, `/oauth/revoke`. That reaches tokens issued without a refresh
token too. A resource server verifying offline still sees it valid until `exp`
(10 minutes): use introspection where immediate revocation matters.

**A revoked family stays revoked.** `revoke_chain` records the family before it
revokes its rows, and a refresh token minted while that happens is born
revoked — a rotation in flight does not outlive the revocation.

**A logout ends the remembered session too.** `/oauth/logout` forgets the
remember-me token and expires its cookie, as `DELETE /logout` does; otherwise
the next request restored the session and the relying party's "sign in" found
the user still there.

**An impersonating admin grants nothing.** `/oauth/authorize` and
`/oauth/device` refuse while the session is an impersonation
(`--impersonation`): the consent and the refresh token would outlive it, at a
third party, in the user's name.

**Device codes.** A user code names one pending request (a unique index; a
taken code is drawn again), the approval updates that request by its key, and
a poll writes only its own two fields — it cannot put a decided request back
to pending or un-consume one.

**A refresh-token family has a ceiling.** Rotation used to buy 30 more days each time, for ever. A family now ends `OIDC_REFRESH_CHAIN_TTL` (90 days) after its first token, or when the sign-in it came from does (a SAML session's expiry) if that is sooner: a token never outlives the ceiling, and past it the client sends the user through `/oauth/authorize` again.

**A code and a refresh token belong to one client.** The token endpoint looks at whose code or refresh token it was handed before it does anything with it. Another client presenting one gets `invalid_grant` and changes nothing — it cannot burn a code it intercepted, nor, by replaying a used code or a rotated token, revoke the tokens of the client it belongs to. A client whose `grant_types` lost `refresh_token` can no longer refresh.

**PKCE for every public client.** A public client has no secret, so PKCE is what binds a code to it. It is required whatever the stored `require_pkce` flag says; the flag only matters for confidential clients.

**No credential in the URL.** `/oauth/token`, `/oauth/revoke`, `/oauth/introspect` and `/oauth/device_authorization` answer `invalid_request` to a `client_secret`, `code`, `code_verifier`, `refresh_token`, `device_code` or `token` sent in the query string: it would work, and stay in access logs, proxies and histories.

**`/oauth/revoke` revokes access tokens too.** Handed one of its own access tokens, a client gets it denylisted by `jti` — userinfo and introspection refuse it from then on. It used to answer `200` and leave the token valid.

**`iss` in the authorization response.** Every redirect back to a client — a code or an error — carries `iss`, and discovery says so (`authorization_response_iss_parameter_supported`, RFC 9207): a client talking to several providers can tell which one answered.

**A client with a request-object key signs every request.** Registering `request_object_key` used to make signed requests possible, not required: plain parameters still worked, so the signature protected nothing. Such a client now gets an error page without `request=`, and a request object is refused when it expires more than ten minutes ahead. The consent form of a signed request carries only a reference: what the signature covered is kept in the session for ten minutes and read back from there, so the form cannot change it and a short-lived object does not expire under the person reading the page.

**Smaller things.** `SOLI_OIDC_REGISTRATION_TOKEN` under 32 characters is ignored (registration answers `404`). An access token signed with the previous key verifies until it expires, as an id_token hint already did. The logout confirmation names the application asking and where the browser goes next. Outside production, an unset `SOLI_OIDC_ISSUER` falls back to `APP_BASE_URL` before `localhost`. Device requests an hour past their expiry are swept as new ones start. A code whose sign-in has ended by the time it is exchanged (a SAML session's expiry) issues nothing.

**Throttled.** `/oauth/token`, `/oauth/introspect`, `/oauth/revoke` and
registration share `OIDC_REQUESTS_PER_IP` (300 a minute) per address — each
verifies a client secret, an Argon2 hash, for anyone naming a client id;
starting a device flow has `OIDC_DEVICE_STARTS_PER_IP` (30), and typing user
codes `OIDC_USER_CODE_ATTEMPTS_PER_IP` (20). The constants are in
`app/services/oidc_config.sl`.

**Exact `redirect_uri` matching.** Byte for byte against a registered URI — no
prefix matching, no wildcards, no normalization. Anything looser lets an
attacker who can register a lookalike path harvest authorization codes.

**Two errors never redirect.** An unknown `client_id` or an unregistered
`redirect_uri` renders a 400 page instead of bouncing the browser. Redirecting
on an unvalidated `redirect_uri` *is* the open redirect that RFC 6749 §4.1.2.1
carves these two cases out to prevent. Every other authorization error does
redirect, with `error`, `error_description` and the echoed `state`.

**PKCE `S256` only.** `plain` is rejected for every client type. It offers no
protection against an attacker who intercepted the authorization request, and
accepting it only adds a downgrade path.

**Codes are single-use and bound.** 60-second TTL, and tied to `client_id`,
`redirect_uri` and `code_challenge` — all re-checked at exchange. The burn is a
single atomic statement, so two simultaneous exchanges cannot both win. A
replayed code revokes every token the first exchange produced.

**Client authentication.** `client_secret_basic` or `client_secret_post`, never
both (RFC 6749 §2.3). Secrets are Argon2 digests. A failed *Basic*
authentication returns 401 with `WWW-Authenticate`; the same failure via form
parameters returns 400 — a distinction from RFC 6749 §5.2 that is routinely
gotten wrong.

**`state` is opaque.** Echoed verbatim on every redirect back, success or
error. It is the client's CSRF defense and rewriting it in any way would break
that.

**Token responses are `Cache-Control: no-store`.** They carry bearer
credentials and must not sit in a proxy cache. This is why the generated code
returns a raw response hash rather than `render_json`, whose fast-path headers
cannot be extended.

**Logout asks unless the request proves itself.** A bare `GET /oauth/logout`
shows a confirmation page that posts back (CSRF-checked), so an
`<img src="/oauth/logout">` on another site logs no one out. The page is skipped,
and `post_logout_redirect_uri` honoured, only for an `id_token_hint` this
provider signed: verified with `jwt_verify` (RS256, the active or previous key,
your issuer), with a year of leeway on the expiry because RP-Initiated Logout
lets a relying party send an id_token that has since expired. The hint must name
the user signed in here — another user's real id_token for the same client
still gets the confirmation page — and an access token is not accepted as a
hint. The hint used to be read with `jwt_decode_unsafe`, so anyone could forge
one naming any client and skip the confirmation.

With **no one signed in** there is nothing to end, and nothing ties the request
to this browser: the visitor gets a "You are signed out" page with a link back
to the relying party instead of an immediate redirect, which would let anyone
holding an id_token (their own) bounce visitors through this endpoint.

The return URI must be one of the client's `post_logout_redirect_uris` — or,
for a client registered without that field, one of its `redirect_uris`.

### Caveats

- Registered `redirect_uris` must be absolute `http`/`https` with no fragment.
  Custom-scheme native redirects (`com.example.app:/cb`) are rejected at
  registration, because the redirect helper only accepts http(s); supporting
  them would mean bypassing the guard that closes the open redirect.
- An app serving files from `public/.well-known/` (ACME HTTP-01, for example)
  can shadow the discovery document, since static files take precedence for
  `GET`.
- Not implemented: front/back-channel logout, DPoP or mTLS sender constraining,
  and `request_uri`.
- **Implicit and hybrid flows are left out on purpose.** They return tokens in the
  redirect URL, where history, logs and referrers keep them; OAuth 2.1 removes
  them and the authorization-code flow with PKCE covers every case they served.

## Connecting a Soli app as a client

For the common providers (GitHub, Google, GitLab, Microsoft…), `soli generate
oauth` writes the consumer side — see [OAuth Client](oauth-client.md); its shared service is
`OauthSso`, so it no longer clashes with this scaffold's `OauthClient` model and
one app can be both. To hand-roll it, see the [GitHub](/docs/blog/github-oauth)
and [Google](/docs/blog/google-oauth) guides. Generate `state` and the PKCE
verifier with `Crypto.random_token()`, and verify the `id_token` against the
provider's JWKS with `jwt_verify(token, "", {"algorithm": "RS256", "key": pem,
"issuer": …, "audience": …})`.
