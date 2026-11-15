# OAuth Client (Sign in with Apple, GitHub, Google, GitLab, Discord, LinkedIn, Microsoft)

Scaffold an OAuth **client** so users can sign in with an external provider.
This is the opposite of [`soli generate oidc_provider`](oidc-provider.md) (your
app *is* the IdP).

## Prerequisites

```bash
soli generate auth          # User + sessions (required)
soli generate oauth github  # and/or apple, google, gitlab, discord, linkedin, microsoft
soli db:migrate up
```

## What gets generated

| Path | Role |
|------|------|
| `app/models/oauth_identity.sl` | Links `User` ↔ provider + uid |
| `app/services/oauth_client.sl` | State CSRF, PKCE helpers, and which account an identity signs in |
| `app/services/github_oauth.sl` | GitHub authorize / token / profile |
| `app/services/google_oauth.sl` | Google OIDC code + PKCE |
| `app/services/{apple,gitlab,discord,linkedin,microsoft}_oauth.sl` | One per provider you generate |
| `app/controllers/oauth_controller.sl` | `/auth/:provider` + callback, `/oauth/identities` |
| `app/views/oauth/identities.html.slv` | The connected-accounts page |
| `db/migrations/*_create_oauth_identities.sl` | Unique index on `(provider, uid)` |
| `config/routes.sl` | Routes appended (idempotent marker) |

## Environment

**GitHub**

```bash
GITHUB_CLIENT_ID=…
GITHUB_CLIENT_SECRET=…
GITHUB_REDIRECT_URI=http://localhost:3000/auth/github/callback
```

**Google**

```bash
GOOGLE_CLIENT_ID=…
GOOGLE_CLIENT_SECRET=…
GOOGLE_REDIRECT_URI=http://localhost:3000/auth/google/callback
```

**GitLab** (`GITLAB_BASE_URL` for a self-hosted instance, default `https://gitlab.com`)

```bash
GITLAB_CLIENT_ID=…
GITLAB_CLIENT_SECRET=…
GITLAB_REDIRECT_URI=http://localhost:3000/auth/gitlab/callback
```

**Discord** (`DISCORD_*`) and **LinkedIn** (`LINKEDIN_*`, "Sign In with LinkedIn using OpenID Connect") take the same three variables.

**Microsoft** additionally requires your tenant — or the list of tenants:

```bash
MICROSOFT_CLIENT_ID=…
MICROSOFT_CLIENT_SECRET=…
MICROSOFT_REDIRECT_URI=http://localhost:3000/auth/microsoft/callback
MICROSOFT_TENANT=<tenant id or domain>

# several tenants (your customers' directories):
MICROSOFT_TENANT=organizations
MICROSOFT_ALLOWED_TENANTS=<tenant id>,<tenant id>
```

Microsoft does not verify the `email` claim, and a multi-tenant app can be handed
someone else's address by a foreign tenant (the "nOAuth" takeover). So an address
is only ever taken from a tenant you named: your own, or — with
`MICROSOFT_TENANT=organizations` — the ones in `MICROSOFT_ALLOWED_TENANTS`, each
of which you trust as you trust your own. The tenant is read from the id_token
(`tid`), whose issuer must be that tenant's. `common` and `consumers` are
refused, and so is `organizations` without a list.

**Apple** has no client secret to copy: it is a short-lived JWT the service signs
with your key (ES256) for each exchange.

```bash
APPLE_CLIENT_ID=com.example.app.web     # the Services ID
APPLE_TEAM_ID=…
APPLE_KEY_ID=…
APPLE_PRIVATE_KEY_FILE=config/AuthKey.p8   # inside the app; or APPLE_PRIVATE_KEY with its contents
APPLE_REDIRECT_URI=https://your.app/auth/apple/callback
```

Apple posts its answer (`response_mode=form_post`) from its own site, so the
session cookie does not come with it. The callback turns that POST into a GET of
the same URL — which the cookie does come with — and only then checks the state;
`config/routes.sl` exempts `/auth/apple/callback` from the CSRF gate for that
one hop. The name arrives once, with the first sign-in; the email is in the
id_token every time, and may be a private relay address. Apple accepts `https://`
return URLs only, on a domain you verified with it.

## How a provider is found

`/auth/:provider` looks up the `<Name>Oauth` class (`GitlabOauth`, `DiscordOauth`, …)
by name, after checking the segment against `^[a-z][a-z0-9]{1,19}$`, so adding a
provider never means editing the controller. Each service exposes
`authorize_url(state, code_challenge)` and `complete!(code)`. The callback also
insists the provider matches the one this browser session started with. An app
generated before this shipped has a controller with a fixed provider list; the
generator says so, and deleting `oauth_controller.sl` then re-running it swaps in
the new one.

## Which account a sign-in opens

`OauthClient.find_or_create_user`, in this order:

1. the account already linked to this `(provider, uid)`;
2. while a signed-in user is connecting a provider (below), that user —
   refused when the identity belongs to someone else;
3. the account with the provider's verified email, linked from now on;
4. a new one, when sign-up is open: `AUTH_SIGNUP` applies as it does to
   `/signup` (`first_user` by default — the first account becomes the admin,
   then sign-up closes; `open`; `closed`). The account is confirmed and has no
   password; its owner can set one with a reset link.

Rule 3 hands an existing account to whoever proves the address at the provider.
If that account never confirmed its email, someone else may have created it in
wait, with a password they know: its password and sessions are dropped before it
is linked.

## Connected accounts

`/oauth/identities` lists the providers linked to the signed-in account, with a
button to connect each provider the app has a service for and one to disconnect.
Connecting is the same flow as signing in, started while signed in. The last way
into an account cannot be removed: with no password, one provider stays.

## Login button

Do not re-run the generator to edit views — add a link yourself:

```html
<a href="/auth/github">Sign in with GitHub</a>
<a href="/auth/google">Sign in with Google</a>
<a href="/auth/gitlab">Sign in with GitLab</a>
```

## Security notes

- Callback verifies `state` against the session (CSRF).
- **PKCE is `S256`.** `begin_pkce()` stores a random verifier in the session and
  returns `base64url(SHA256(verifier))` unpadded, per RFC 7636 §4.2. Earlier
  cuts sent the raw verifier with `code_challenge_method=plain`, where the
  challenge and the verifier are the same string — PKCE in name only. Never
  ship `plain`.
- The services call `HTTP.request(method, url, headers, body)`. `HTTP.get` /
  `HTTP.post` read their options hash for `timeout` alone and return the body as
  a String, so a `"headers"` key there is silently dropped — which is why the
  generated flow used to 401.
- Provider responses are status-checked, so a 401 reports as a 401 instead of a
  JSON parse error on the provider's error page.
- Google, GitLab, LinkedIn and Apple require the provider to report `email_verified`; Discord requires `verified`; Microsoft is limited to the tenants you name (see above).
- The start redirect uses `redirect_external`: plain `redirect` only accepts local paths, and the first generated controller failed with a 500 for that reason.
- Accounts created via OAuth are confirmed and have no password, and the session
  carries the account's session version, like a password sign-in. Earlier cuts
  stored a random password in a plaintext `password` field, created accounts
  whatever `AUTH_SIGNUP` said, and opened sessions that the first password
  reset on the account then refused.
- Apple's and Microsoft's id_token is read without checking its signature: it
  comes from the provider's token endpoint over TLS, in answer to the server's
  own request (OpenID Connect Core §3.1.3.7). Issuer, audience, expiry — and for
  Microsoft the tenant — are checked.
- With [two-factor](/docs/security/authentication#two-factor) on, a provider
  sign-in is followed by the code challenge.
- Prefer HTTPS redirect URIs in production.

## Ceiling

Seven providers: Apple, GitHub, Google, GitLab, Discord, LinkedIn and Microsoft.
Others: copy a service file, keep the two-method shape above, and the controller finds
it. No OmniAuth-style catalog. Microsoft personal accounts (`consumers`) are not
supported, and neither is an open multi-tenant app (any directory). Apple's
"hide my email" relay address is taken as the account's email as it is. An app
generated before account linking keeps its controller and service: delete
`oauth_controller.sl` and `oauth_client.sl`, then re-run the generator.

Longer walkthroughs: [GitHub OAuth blog](/docs/blog/github-oauth),
[Google OAuth blog](/docs/blog/google-oauth).
