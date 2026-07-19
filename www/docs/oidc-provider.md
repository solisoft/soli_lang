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
| `POST /oauth/token` | Code and refresh grants |
| `GET\|POST /oauth/userinfo` | Claims for a bearer token |
| `POST /oauth/revoke` | RFC 7009 revocation |
| `GET /oauth/logout` | RP-initiated logout |

> The generated routes call `skip_csrf` on `/oauth/token` and `/oauth/revoke`.
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
claim byte for byte.

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
`grant_types`, `require_pkce`, `skip_consent`.

A **public** client (SPA, mobile) has no secret, so PKCE is forced on and
cannot be switched off — it is the only thing binding the code to the
requester.

### 4. Two edits to `sessions_controller.sl`

The generator does not modify files it did not create, so make these yourself:

```soli
# in `create`, next to session_regenerate():
session_set("auth_time", DateTime.utc().to_unix())

# and replace the final `return redirect("/")` with:
destination = session_get("oidc_return_to") ?? "/"
session_delete("oidc_return_to")
return redirect(destination)
```

Without the first, `auth_time` falls back to the authorization instant. Without
the second, a user who signs in mid-flow lands on the home page instead of
completing the authorization.

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

## id_token claims

`iss`, `sub`, `aud`, `exp`, `iat`, plus `nonce` (echoed from the authorization
request), `auth_time`, and `at_hash` — the left-most 128 bits of the SHA-256 of
the access token, which lets a client detect an access token swapped in from a
different response. Scope-gated claims (`email`, `name`, …) come from the
`oidc_user_claims` hook in `app/services/oidc_config.sl`; that is the one place
that needs to know what a `User` looks like.

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

### Caveats

- Registered `redirect_uris` must be absolute `http`/`https` with no fragment.
  Custom-scheme native redirects (`com.example.app:/cb`) are rejected at
  registration, because the redirect helper only accepts http(s); supporting
  them would mean bypassing the guard that closes the open redirect.
- An app serving files from `public/.well-known/` (ACME HTTP-01, for example)
  can shadow the discovery document, since static files take precedence for
  `GET`.
- Not implemented: dynamic client registration (RFC 7591), request objects
  (JAR), implicit and hybrid flows, client-credentials and device-code grants,
  front/back-channel logout, DPoP or mTLS sender constraining, and token
  introspection (RFC 7662).

## Connecting a Soli app as a client

The consumer side is hand-rolled — see the [GitHub](/docs/blog/github-oauth)
and [Google](/docs/blog/google-oauth) guides. Generate `state` and the PKCE
verifier with `Crypto.random_token()`, and verify the `id_token` against the
provider's JWKS with `jwt_verify(token, "", {"algorithm": "RS256", "key": pem,
"issuer": …, "audience": …})`.
