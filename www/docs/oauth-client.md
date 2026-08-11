# OAuth Client (Sign in with GitHub / Google)

Scaffold an OAuth **client** so users can sign in with an external provider.
This is the opposite of [`soli generate oidc_provider`](oidc-provider.md) (your
app *is* the IdP).

## Prerequisites

```bash
soli generate auth          # User + sessions (required)
soli generate oauth github  # and/or google
soli db:migrate up
```

## What gets generated

| Path | Role |
|------|------|
| `app/models/oauth_identity.sl` | Links `User` ↔ provider + uid |
| `app/services/oauth_client.sl` | State CSRF, PKCE helpers, find-or-create user |
| `app/services/github_oauth.sl` | GitHub authorize / token / profile |
| `app/services/google_oauth.sl` | Google OIDC code + PKCE |
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

## Login button

Do not re-run the generator to edit views — add a link yourself:

```html
<a href="/auth/github">Sign in with GitHub</a>
<a href="/auth/google">Sign in with Google</a>
```

## Security notes

- Callback verifies `state` against the session (CSRF).
- Google path requires a **verified** email.
- Accounts created via OAuth get a random password and confirmed email.
- Prefer HTTPS redirect URIs in production.

## Ceiling

v1 providers: **GitHub** and **Google** only. Add more by copying a service
file. Full OmniAuth-style catalogs and account-linking UIs are left to the app.

Longer walkthroughs: [GitHub OAuth blog](/docs/blog/github-oauth),
[Google OAuth blog](/docs/blog/google-oauth).
