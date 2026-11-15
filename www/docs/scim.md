# SCIM Provisioning

Let an organisation's directory — Okta, Microsoft Entra ID, OneLogin — create,
update and deactivate your app's users. SCIM 2.0 is the protocol directories
speak for that; it completes [SAML](/docs/security/saml) or
[OAuth](/docs/security/oauth-client) sign-in, which only cover signing in.

```bash
soli generate auth    # SCIM provisions the auth scaffold's User
soli generate scim
soli db:migrate up
```

## What it generates

| File | |
|---|---|
| `app/controllers/scim_controller.sl` | The SCIM server, under `/scim/v2` |
| `app/middleware/scim_active.sl` | Keeps deactivated accounts out (`# order: 8`) |
| `db/migrations/…_add_scim_user_indexes.sl` | Index on the users' `scim_external_id` |
| `config/routes.sl` | The routes, and `skip_csrf("/scim/*")` |

No file of the auth scaffold is edited.

## Setup

1. Give the directory a bearer token of its own, 32 characters or more:

   ```bash
   SCIM_TOKEN=<token>                       # one directory
   SCIM_TOKENS=okta=<token>,entra=<token>   # several, each named
   ```

   A shorter token is ignored, so a placeholder left in `.env` opens nothing.
2. In the directory's provisioning settings: base URL `APP_BASE_URL/scim/v2`,
   authentication by bearer token ("HTTP Header" in Okta, "Secret Token" in
   Entra), and `userName` mapped to the user's email.

## Endpoints

| Route | |
|---|---|
| `GET /scim/v2/ServiceProviderConfig`, `/ResourceTypes`, `/Schemas` | What the server supports |
| `GET /scim/v2/Users` | List (`startIndex`, `count`, up to 100 a page), or find with `filter=userName eq "…"` / `externalId eq "…"` |
| `POST /scim/v2/Users` | Create — `409` (`uniqueness`) when the `userName` exists |
| `GET /scim/v2/Users/:id` | Read |
| `PUT /scim/v2/Users/:id` | Replace |
| `PATCH /scim/v2/Users/:id` | Update — what directories use to deactivate |
| `DELETE /scim/v2/Users/:id` | Remove the account |

Responses are `application/scim+json`; errors use SCIM's error schema.

## How a directory's user maps to an account

| SCIM | Account |
|---|---|
| `userName` (else the first of `emails`) | `email`, lowercased |
| `active` | `deactivated_at` — `nil` while active |
| `externalId` | `scim_external_id` |
| `displayName`, `name.formatted` | `name` |
| `name.givenName`, `name.familyName` | `scim_given_name`, `scim_family_name` |

An account a directory creates is a `member`, confirmed, with no password: it
signs in through SAML, OAuth or a magic link, or sets a password with a reset
link.

Both shapes of `PATCH` are understood: Okta's, with no path and the attributes
in the value (`{"op": "replace", "value": {"active": false}}`), and Entra's,
with a path and — for `active` — the string `"False"`
(`{"op": "Replace", "path": "active", "value": "False"}`), including
`emails[type eq "work"].value`.

## Deactivation

`active: false` closes the user's sessions (their session version is bumped,
their remember-me cookie forgotten), and from then on
`app/middleware/scim_active.sl` treats any request of theirs as a guest's —
whatever signed them in: a password, a provider, a link, an API token. Checking
there rather than in each sign-in action covers them all. `active: true`
restores the account; the sessions closed meanwhile stay closed.

`DELETE` removes the account. To keep what it owns, change `destroy` in the
controller to deactivate instead — the comment there says how.

## Limits

- **Users only.** No `Groups`: an account's role is not provisioned. Map a
  SCIM attribute to it in `_assign` if your directory sends one.
- **No bulk, sorting, ETags or password change**, and the filter is `userName
  eq` / `externalId eq` — what directories send, not the whole filter grammar.
- **Tokens are static**, read from the environment; rotate by listing the new
  one beside the old in `SCIM_TOKENS`, then removing the old.
- A directory's token only opens `/scim/v2`; it is not a user's
  [API token](/docs/security/authentication#api-tokens), and neither is it.

## See also

- [SAML Single Sign-On](/docs/security/saml)
- [Authentication](/docs/security/authentication) — sessions, two-factor, API tokens
