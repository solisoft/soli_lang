# OAuth Client (Sign in with GitHub, Google, GitLab, Discord, LinkedIn, Microsoft)

Scaffold an OAuth **client** so users can sign in with an external provider.
This is the opposite of [`soli generate oidc_provider`](oidc-provider.md) (your
app *is* the IdP).

## Prerequisites

```bash
soli generate auth          # User + sessions (required)
soli generate oauth github  # and/or google, gitlab, discord, linkedin, microsoft
soli db:migrate up
```

## What gets generated

| Path | Role |
|------|------|
| `app/models/oauth_identity.sl` | Links `User` ↔ provider + uid |
| `app/services/oauth_client.sl` | State CSRF, PKCE helpers, find-or-create user |
| `app/services/github_oauth.sl` | GitHub authorize / token / profile |
| `app/services/google_oauth.sl` | Google OIDC code + PKCE |
| `app/services/{gitlab,discord,linkedin,microsoft}_oauth.sl` | One per provider you generate |
| `app/controllers/oauth_controller.sl` | `/auth/:provider` + callback |
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

**Microsoft** additionally requires your tenant:

```bash
MICROSOFT_CLIENT_ID=…
MICROSOFT_CLIENT_SECRET=…
MICROSOFT_REDIRECT_URI=http://localhost:3000/auth/microsoft/callback
MICROSOFT_TENANT=<tenant id or domain>
```

Microsoft does not verify the `email` claim, and a multi-tenant app can be handed
someone else's address by a foreign tenant. So the generated service refuses
`common`, `organizations` and `consumers`: sign-in is limited to the tenant whose
directory you control.

## How a provider is found

`/auth/:provider` looks up the `<Name>Oauth` class (`GitlabOauth`, `DiscordOauth`, …)
by name, after checking the segment against `^[a-z][a-z0-9]{1,19}$`, so adding a
provider never means editing the controller. Each service exposes
`authorize_url(state, code_challenge)` and `complete!(code)`. The callback also
insists the provider matches the one this browser session started with. An app
generated before this shipped has a controller with a fixed provider list; the
generator says so, and deleting `oauth_controller.sl` then re-running it swaps in
the new one.

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
- Google, GitLab and LinkedIn require the provider to report `email_verified`; Discord requires `verified`; Microsoft is limited to your tenant (see above).
- The start redirect uses `redirect_external`: plain `redirect` only accepts local paths, and the first generated controller failed with a 500 for that reason.
- Accounts created via OAuth get a random password and confirmed email.
- Prefer HTTPS redirect URIs in production.

## Ceiling

Six providers: GitHub, Google, GitLab, Discord, LinkedIn and Microsoft (one tenant).
Others: copy a service file, keep the two-method shape above, and the controller finds
it. No Apple sign-in (its client secret is a signed JWT), no multi-tenant Microsoft,
and no OmniAuth-style catalog or account-linking UI.

Longer walkthroughs: [GitHub OAuth blog](/docs/blog/github-oauth),
[Google OAuth blog](/docs/blog/google-oauth).
