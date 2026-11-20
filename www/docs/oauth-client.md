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
| `app/services/oauth_sso.sl` | `OauthSso`: state CSRF, PKCE helpers, re-authentication, and which account an identity signs in |
| `app/services/github_oauth.sl` | GitHub authorize / token / profile + verified email, PKCE |
| `app/services/google_oauth.sl` | Google OIDC code + PKCE |
| `app/services/{apple,gitlab,discord,linkedin,microsoft}_oauth.sl` | One per provider you generate |
| `app/controllers/oauth_controller.sl` | `/auth/:provider` + callback, `POST /auth/:provider/link`, `/oauth/identities`, `/oauth/reauth` |
| `app/views/oauth/identities.html.slv` | The connected-accounts page |
| `app/views/oauth/reauth.html.slv` | "Confirm it's you" before a provider is linked |
| `app/mailers/oauth_mailer.sl` + `app/views/oauth_mailer/provider_linked.html.slv` | Tells the owner a provider was linked to their account |
| `db/migrations/*_create_oauth_identities.sl` | Unique index on `(provider, uid)` |
| `config/routes.sl` | Routes appended (idempotent marker) |

The shared service is `OauthSso` in `oauth_sso.sl`. It used to be `class
OauthClient` in `oauth_client.sl`, which clashed with the `OauthClient` model
that [`soli generate oidc_provider`](oidc-provider.md) writes — an app could not
be both a client and a provider. The generator notes an old `oauth_client.sl`
when it finds one: delete it, the `*_oauth.sl` services and
`oauth_controller.sl`, then re-run it for each provider.

## Environment

**GitHub** (`GITHUB_BASE_URL` / `GITHUB_API_URL` for GitHub Enterprise Server,
default `https://github.com` / `https://api.github.com`)

```bash
GITHUB_CLIENT_ID=…
GITHUB_CLIENT_SECRET=…
GITHUB_REDIRECT_URI=http://localhost:3000/auth/github/callback
```

The address is the one `/user/emails` marks **primary and verified** — never the
profile's public `email`, which is whatever the user typed and GitHub never
checked. The flow uses PKCE (S256), which GitHub supports.

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

The address is taken when the instance says `email_verified`. With
`GITLAB_BASE_URL` pointing at an instance someone else runs, that is *their*
word: its administrators can create a user with any address and mark it
verified, and rule 3 below then hands them the account with that address here.
Point it only at gitlab.com or an instance you trust as you trust your own user
directory — the same reasoning as Microsoft's named tenants.

**Discord** (`DISCORD_*`) and **LinkedIn** (`LINKEDIN_*`, "Sign In with LinkedIn
using OpenID Connect") take the same three variables. Neither uses PKCE: Discord
does not document it, and LinkedIn offers it only to native apps it approves.

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

Even inside a named tenant an administrator can give a user any `email`, so the
address is used only when Microsoft says it is verified: `email` with the
`xms_edov` claim `true` (the domain's owner is verified), or
`verified_primary_email`. Both are **optional claims you must add**: Entra admin
center → App registrations → your app → Token configuration → Add optional claim
→ token type **ID** → `email`, `xms_edov`, `verified_primary_email`. Without them
no address is trusted: a Microsoft identity then signs in only once a signed-in
user has linked it at `/oauth/identities`. `preferred_username` is never taken as
an address.

The identity's uid is `<tid>:<sub>`: `sub` is unique only within one tenant.
Apps generated before that stored the bare `sub`. **Upgrading:** delete
`app/services/microsoft_oauth.sl` and run `soli generate oauth microsoft` (the
generator notes an old file). The new service rewrites an old row to
`<tid>:<sub>` the first time its owner signs in — when the `tid` recorded with
the row (its `raw` claims) is the token's, or, for a row that recorded none, only
in a single-tenant app (`MICROSOFT_TENANT` a tenant id or domain): in a
multi-tenant app the same `sub` may belong to a user of another directory. A row
it does not adopt stays as it is, and its owner links Microsoft again from
`/oauth/identities`.

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

`/auth/:provider` (and `/auth/:provider/link`) looks up the `<Name>Oauth` class (`GitlabOauth`, `DiscordOauth`, …)
by name, after checking the segment against `^[a-z][a-z0-9]{1,19}$`, so adding a
provider never means editing the controller. Each service exposes
`authorize_url(state, code_challenge)` and `complete!(code)`. The callback also
insists the provider matches the one this browser session started with. An app
generated before this shipped has a controller with a fixed provider list; the
generator says so, and deleting `oauth_controller.sl` then re-running it swaps in
the new one.

## Which account a sign-in opens

`OauthSso.find_or_create_user`, in this order:

1. the account already linked to this `(provider, uid)`;
2. while a signed-in user is connecting a provider (below), that user —
   refused when the identity belongs to someone else, when the account's email
   is not confirmed, without a recent sign-in, or when the account signs in
   through single sign-on only;
3. the account with the address the provider **verified**, linked from now on —
   but never one that signs in through single sign-on only;
4. a new one, when sign-up is open: `AUTH_SIGNUP` applies as it does to
   `/signup` (`first_user` by default — the first account becomes the admin,
   then sign-up closes; `open`; `closed`). The account is confirmed and has no
   password; its owner can set one with a reset link.

A provider that reports no verified address (an unverified Google, GitLab,
LinkedIn, Apple or Discord address, a GitHub account without a verified primary
one, a Microsoft token without `xms_edov` / `verified_primary_email`) still
signs in through rule 1 — once it is linked — but never finds or creates an
account by address.

Rule 3 hands an existing account to whoever proves the address at the provider.
If that account never confirmed its email, someone else may have created it in
wait, with a password they know — and, before, providers they linked to it. So
`OauthSso.take_over_unconfirmed` calls `User#revoke_credentials` (written by
`soli generate auth`) before it is linked: the password and remember-me token
are cleared, the session version bumped (every session ends), and the user's
linked identities, passkeys, API tokens, two-factor setup and magic links are
deleted. The address's owner gets the account alone. An app whose `user.sl`
predates `revoke_credentials` gets a note from the generator.

**Single sign-on only.** An account created by [`soli generate saml`](saml.md)
is marked `sso_only` (`User#sso_only?`): it signs in through its identity
provider and nothing else, so an administrator who removes someone there removes
every way in. A provider vouching for the same address would be a way around
that, so `OauthSso` refuses such an account in every rule — by address (rule 3),
through an identity linked before it became SSO-only (rule 1), and when linking
(rule 2). The check goes through `respond_to?("sso_only?")`, so an app whose
`User` has no such method is unaffected.

Whenever a provider is linked to an account that already existed — by its owner
at `/oauth/identities` or by rule 3 — `OauthMailer.provider_linked` emails the
account's address, so an owner hears about a way in they did not add.

## Connected accounts

`/oauth/identities` lists the providers linked to the signed-in account, with a
button to connect each provider the app has a service for and one to disconnect.
The last way into an account cannot be removed: with no password, one provider
stays.

**Connecting starts from a POST.** Each Connect button is a `button_to` form
posting to `POST /auth/:provider/link`, CSRF-checked like any other form. The
plain `GET /auth/:provider` is for signing a *guest* in; a signed-in user who
reaches it is shown `/oauth/identities` and nothing starts. It used to link: any
site could send a recently signed-in visitor to `/auth/github` (a link, a
top-level redirect), and the attacker's own GitHub account, authorized on the
way, ended up attached to the victim's account. Starting a flow also drops any
link left over from one abandoned half-way, so a later sign-in in the same
browser is never taken for a link.

Connecting needs:

- **A confirmed email.** An unconfirmed account (anyone can create one with any
  address) cannot link a provider: the POST answers 403.
- **A recent sign-in** — within ten minutes (`OauthSso.reauth_seconds`). A
  provider sign-in counts; a password sign-in through `/login` is not timed by
  the auth scaffold, so the POST sends the user to `/oauth/reauth`, which asks
  for the password (failures count toward the account lockout) and then goes
  straight on to the provider. An account with no password signs out and back
  in with a provider it has, then connects within ten minutes. Without this, a
  session borrowed for a minute — a shared computer, a stolen cookie — could
  attach a provider that keeps signing in after a password reset.
- **An account that is not SSO-only** (above): 403.

An app generated before linking moved to the POST keeps its controller; the
generator notes it. Delete `app/controllers/oauth_controller.sl`,
`app/services/oauth_sso.sl` and `app/views/oauth/identities.html.slv`, then
re-run it: the route `post("/auth/:provider/link", "oauth#link_provider")` is
appended under its own marker.

## Sign-in time (`auth_time`)

A provider sign-in records `session_set("auth_time", DateTime.utc.to_unix)`
(`OauthSso.sign_in`; linking does not, the session was already signed in). An
app that is also an [OpenID Connect provider](oidc-provider.md) puts it in its
id_tokens and checks `max_age` against it.

## Login button

Do not re-run the generator to edit views — add a link yourself (a plain link is
right here: it only ever signs a guest in):

```html
<a href="/auth/github">Sign in with GitHub</a>
<a href="/auth/google">Sign in with Google</a>
<a href="/auth/gitlab">Sign in with GitLab</a>
```

## Security notes

- Callback verifies `state` against the session (CSRF).
- **PKCE is `S256`** (every provider but Discord and LinkedIn). `begin_pkce()`
  stores a random verifier in the session and returns
  `base64url(SHA256(verifier))` unpadded, per RFC 7636 §4.2. Earlier cuts sent
  the raw verifier with `code_challenge_method=plain`, where the challenge and
  the verifier are the same string — PKCE in name only. Never ship `plain`.
- Every value of a token request body is `url_encode`d. Google's used to be
  concatenated raw, so a secret holding `&`, `+` or `=` broke the exchange.
- **Errors are logged, not shown.** A refusal written for the user (`OauthSso.refuse`,
  e.g. "this Apple account is linked to another user") reaches the page; what a
  provider sent — `error_description`, an error body, an address, a tenant id —
  goes to the log (`Logger.warn`) and the page says only that sign-in failed.
- The services call `HTTP.request(method, url, headers, body)`. `HTTP.get` /
  `HTTP.post` read their options hash for `timeout` alone and return the body as
  a String, so a `"headers"` key there is silently dropped — which is why the
  generated flow used to 401.
- Provider responses are status-checked, so a 401 reports as a 401 instead of a
  JSON parse error on the provider's error page.
- An address counts only when the provider verified it: Google, GitLab, LinkedIn
  and Apple report `email_verified`; Discord `verified`; GitHub marks the
  address primary and verified in `/user/emails`; Microsoft needs `xms_edov` or
  `verified_primary_email`, from a tenant you name (see above).
- The start redirect uses `redirect_external`: plain `redirect` only accepts local paths, and the first generated controller failed with a 500 for that reason.
- Accounts created via OAuth are confirmed and have no password, and the session
  carries the account's session version, like a password sign-in. Earlier cuts
  stored a random password in a plaintext `password` field, created accounts
  whatever `AUTH_SIGNUP` said, and opened sessions that the first password
  reset on the account then refused.
- Apple's and Microsoft's id_token is read without checking its signature: it
  comes from the provider's token endpoint over TLS, in answer to the server's
  own request (OpenID Connect Core §3.1.3.7). Issuer, audience, expiry — and for
  Microsoft the tenant — are checked. That trust needs the answer to really be
  the provider's, so the token exchange follows no redirect, and
  `APPLE_BASE_URL` / `MICROSOFT_BASE_URL` may name only this machine (a test
  stand-in: `http://127.0.0.1:<port>`, `localhost`, `[::1]`); any other value
  is refused rather than trusted to hand back a token.
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
