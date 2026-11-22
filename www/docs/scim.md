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
| `app/controllers/scim_controller.sl` | The SCIM server, under `/scim/v2`: users |
| `app/controllers/scim_groups_controller.sl` | Groups and their members |
| `app/models/scim_group.sl` | `ScimGroup`, `ScimMembership`, and what a group gives (`SCIM_GROUP_ROLES`) |
| `app/middleware/scim_active.sl` | Keeps deactivated accounts out (`# order: 8`) |
| `db/migrations/…_add_scim_user_indexes.sl` | Index on the users' `scim_external_id`; the `scim_groups` and `scim_memberships` collections |
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
| `GET /scim/v2/Groups` | List, or find with `filter=displayName eq "…"` |
| `POST /scim/v2/Groups`, `GET` / `PUT` / `PATCH` / `DELETE /scim/v2/Groups/:id` | Create, read, replace, change members or name, delete |

Responses are `application/scim+json`; errors use SCIM's error schema.

## How a directory's user maps to an account

| SCIM | Account |
|---|---|
| `userName` (else the first of `emails`) | `email`, lowercased |
| `active` | `deactivated_at` — `nil` while active |
| `externalId` | `scim_external_id` |
| `displayName`, `name.formatted` | `name` |
| `name.givenName`, `name.familyName` | `scim_given_name`, `scim_family_name` |

An account a directory creates is a `member`, confirmed, with no password
(and `scim_provisioned_at` set): it signs in through SAML, OAuth or a magic
link, or sets a password with a reset link. Its first [SAML](/docs/security/saml)
sign-in links it with `SAML_LINK_EXISTING` off when its `externalId` is the
assertion's NameID, or, by email alone, with `SAML_LINK_DIRECTORY=on` (the
directory and the identity provider are one organisation's) — and, if it still has no
password then, makes it `sso_only`. Deactivating a user here (`active: false`)
is what ends their open sessions and tokens: `sso_only` only closes the ways
in that bypass the identity provider.

### Which accounts a directory can touch

Only the ones it provisioned. An administrator, an account that signed up
here, and another directory's account (`SCIM_TOKENS` names each directory, and
an account remembers which one made it in `scim_directory`) are not there for
it: not listed, not found by a filter, `404` on a read, a `PUT`, a `PATCH` or a
`DELETE`, and never put in a group. So a leaked or over-eager token cannot move
an administrator's address and then reset its password, deactivate the app's
own accounts, or reach across tenants.

A directory that finds no user under a `userName` creates one, and gets a
`409` if a local account already has that address. To let it take such an
account under its management instead — the way Okta and Entra match an
existing user — set `SCIM_ADOPT_EXISTING=on`. Adopting an account **revokes
what it could sign in with** (`User#revoke_credentials`: password, sessions,
API tokens, passkeys, linked identities): whoever created it is not known to
be the person the directory means. An administrator is never adopted.

A write that cannot be saved — a `userName` that is another account's, an
address the model refuses — answers `409` or `400`, never `200`: a directory
that reads `200` records the change, a deactivation included, as made.

Past twenty requests without a valid bearer token from one address in five minutes, the next ones are answered `429`; a request with the right token is always served, so nobody can switch a directory's provisioning off from its address. A token in `SCIM_TOKENS` may contain `=` (the pair is cut at the first one). A value that is not a string — a number, a hash, an array where a name or an address goes — changes nothing (only `remove` clears a field), and a `PUT` whose `userName` is not a string answers `400`. The login address follows `userName`, or the work or primary entry of `emails` — never another entry.

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

`DELETE` removes the account, after revoking its credentials (a linked
identity or an API token left behind would name an account that is gone). To
keep what it owns, change `destroy` in the
controller to deactivate instead — the comment there says how.

## Groups

The directory creates groups and moves users in and out of them. A group gives
nothing by itself; the app says what it means, two ways:

```soli
# In a policy or a controller: read them.
ScimGroup.names_for(user).includes?("Finance")

# Or map them to the account's role, in app/models/scim_group.sl:
const SCIM_GROUP_ROLES = {"Finance": "accountant", "Support": "agent"}
```

With the mapping, a user's `role` follows their groups: in several mapped
groups, the last one listed in the mapping wins; in none, `member`.

- **A directory never makes an administrator.** `admin` is granted in the app:
  a mapping to it is ignored, and an account that is an administrator keeps
  its role whatever its groups say. A compromised or misconfigured directory
  cannot hand out the app's administration.
- **What a group gave is taken back at once** when a user leaves it, when it
  is renamed, or when it is deleted: the role is recomputed on every change.
- Member changes are understood in the three shapes directories send:
  `add` / `remove` on `members` with a list of values, `remove` on
  `members[value eq "<id>"]`, and `replace` on `members` for the whole list.
  A user the app does not know is skipped.
- A user resource lists its `groups`, read-only: membership is changed on the
  group.

## Limits

- **No bulk, sorting, ETags or password change**, and the filters are
  `userName eq` / `externalId eq` on users and `displayName eq` on groups —
  what directories send, not the whole filter grammar.
- **One role per account**, and groups do not nest: a group's members are
  users.
- **Tokens are static**, read from the environment; rotate by listing the new
  one beside the old in `SCIM_TOKENS`, then removing the old.
- A directory's token only opens `/scim/v2`; it is not a user's
  [API token](/docs/security/authentication#api-tokens), and neither is it.

## See also

- [SAML Single Sign-On](/docs/security/saml)
- [Authentication](/docs/security/authentication) — sessions, two-factor, API tokens
