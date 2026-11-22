# SAML Single Sign-On

Sign users in through their organisation's identity provider — Okta, Microsoft
Entra ID, Google Workspace, Keycloak, ADFS — over SAML 2.0. The application is
the **service provider** (SP): it sends the browser to the identity provider
(IdP) with a request, and the IdP posts a signed response back.

```bash
soli generate auth    # SAML signs a User in
soli generate saml
```

The cryptography and the protocol checks are the native `Saml` class; the
generated files are the application's part, and yours to edit.

## What it generates

| File | |
|---|---|
| `app/services/saml_sso.sl` | `SamlSso`: where the identity provider is, the request in flight, which account an assertion maps to |
| `app/controllers/saml_controller.sl` | `GET /saml/login`, `POST /saml/acs`, `GET /saml/metadata` |
| `app/models/saml_identity.sl` | `SamlIdentity`: the link between a `User` and `(issuer, NameID)` |
| `app/models/saml_assertion.sl` | `SamlAssertion`: the assertions that already signed someone in, kept until they expire |
| `db/migrations/…_create_saml_identities.sl` | unique indexes on `(issuer, name_id)` and on `(user_id, issuer)` — one identity per account at each IdP |
| `db/migrations/…_create_saml_assertions.sl` | index on `expires_at` |
| `config/routes.sl` | the three routes, and `skip_csrf("/saml/acs")` |

Nothing existing is overwritten. Add the link to your sign-in view:

```erb
<a href="/saml/login">Sign in with SSO</a>
```

## Setup

1. `soli db:migrate up`
2. Set `APP_BASE_URL` to the `https://` origin the identity provider reaches
   the application at, and `SOLI_SESSION_SECRET` (32+ characters). **https is
   required**, except on `localhost`: the request cookie must be
   `SameSite=None`, which a browser accepts only with `Secure`, and it keeps a
   `Secure` cookie over plain `http://` on `localhost` alone. On any other
   `http://` origin the cookie is dropped and every sign-in fails with "no
   sign-in in progress" (`/saml/login` logs a warning).
3. Give the identity provider's administrator `https://your.app/saml/metadata`.
   It states the two values every IdP asks for:

   | | Default |
   |---|---|
   | Entity ID (audience) | `APP_BASE_URL/saml/metadata` — override with `SAML_SP_ENTITY_ID` |
   | ACS URL (reply URL) | `APP_BASE_URL/saml/acs` |
   | NameID format | `persistent` — `SAML_NAMEID_FORMAT` in `app/services/saml_sso.sl` |

   **Ask for a persistent NameID.** A sign-in is matched on `(issuer, NameID)`,
   so the NameID must name the person for good. A `persistent` NameID is an
   opaque id the IdP never gives anyone else. With the `emailAddress` format the
   NameID *is* the mailbox, and the application inherits the IdP's address
   reuse: when the IdP hands `alice@corp.example` to a new employee, the new
   employee signs in to the old Alice's account. Configure the emailAddress
   format only if the IdP can send nothing else, and only if it never reuses
   addresses.

4. Point the application at the identity provider, either way:

   | Variable | |
   |---|---|
   | `SAML_IDP_METADATA_FILE` | path to the IdP's metadata XML, inside the app (the server reads no file outside it) |
   | `SAML_IDP_ENTITY_ID`, `SAML_IDP_SSO_URL`, `SAML_IDP_CERTIFICATE` | or its three values: entity id, single sign-on URL (HTTP-Redirect binding), signing certificate (PEM; `\n` for newlines) |

   Without an entity id `SamlSso.identity_provider` raises, and no sign-in
   starts or finishes: it is what the `Issuer` of every response is checked
   against.

5. Decide who gets an account:

   | Variable | |
   |---|---|
   | `SAML_PROVISION` | `on`: a first sign-in creates the account; `off`: it does not. Unset, it follows `AUTH_SIGNUP` — `closed` provisions nobody, so the identity provider is no way around a sign-up you closed. Only `on` and `off` are read: any other value (`yes`, `true`) counts as `off` |
   | `SAML_LINK_EXISTING` | `on`: a first sign-in may link the account that already has the asserted email. **Off unless set** |
   | `SAML_LINK_DIRECTORY` | `on`: a first sign-in may link, by email, an account a [SCIM](/docs/security/scim) directory provisioned. **Off unless set**; off, such an account still links when its SCIM `externalId` is the assertion's NameID |
   | `SAML_SESSION_TTL` | seconds a SAML sign-in lasts here, cut shorter by the IdP's `SessionNotOnOrAfter`. Default `28800` (8 hours) |

   Outside development, with `AUTH_SIGNUP=first_user`, no SAML sign-in creates an
   account until the first one exists: the first account is the admin, made at
   `/signup` with `AUTH_SETUP_TOKEN`, and a SAML member created before it would
   make the app non-empty and close `/signup`. The refusal is logged
   (`SAML sign-in refused: no account exists yet: create the first one (the
   admin) at /signup with AUTH_SETUP_TOKEN`).

The IdP must sign the assertion or the response (or both) with RSA-SHA256 —
the metadata asks for signed assertions (`WantAssertionsSigned="true"`), and
a response signed as a whole is accepted too. It must send an email address:
in an attribute named `email`, `mail`, `emailaddress`, or the usual claim URIs
(the first of `SAML_EMAIL_ATTRIBUTES`, in that order, that the assertion
carries with a value — not whichever the response happens to list first),
or as the NameID when its format is
`urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress` (a NameID in any
other format — a username, an opaque id — is never taken for an address, even
with an `@` in it). An address with anything but ASCII in it is refused:
lowercasing folds some Unicode letters into ASCII ones (the Kelvin sign,
U+212A, into `k`), and two different addresses would reach the same account.

An `emailAddress`-format NameID is stored and looked up trimmed and
lowercased, so an IdP that starts sending `Alice@corp.example` for the
identity stored as `alice@corp.example` still signs her in. An identity stored
exactly by an older version of the scaffold is found too (the lookup tries the
normalised NameID, then the exact one). A NameID in any other format is opaque
and matched exactly.

## How a sign-in runs

1. `/saml/login` builds the request (`Saml.auth_request`) and redirects to the
   IdP. The request id and the `return_to` path go in an encrypted cookie.
2. The user authenticates at the IdP, which posts `SAMLResponse` to
   `/saml/acs`.
3. `SamlSso.finish` reads the request id from the cookie and
   `Saml.verify_response` checks the response against it. Only once it
   verifies is the cookie cleared — a request is answered once — so a forged
   post to `/saml/acs`, which any site can make a browser send, does not
   cancel the sign-in in progress. `SamlSso.spend` then records the
   assertion's id until it expires: a response posted again, even with a copy
   of the request cookie, is refused.
4. `SamlSso.user_for` finds the account: the one already linked to this
   `(issuer, NameID)`; else the one with that email, linked from now on, when
   `SAML_LINK_EXISTING=on`, when a directory provisioned it and named this
   identity (or `SAML_LINK_DIRECTORY=on`), or when nobody
   confirmed its address (see below) — a take-over of that kind that stopped
   before its identity was stored is finished by the next sign-in; else a new
   one (see `SAML_PROVISION`).
   Otherwise an address that already has an account is refused: link it by
   hand (`SamlIdentity.create({"user_id", "issuer", "name_id", "email"})`).
   To take a link away for good, use `user.revoke_credentials` (or clear
   `saml_link_pending` with the identity row): a marked account without an
   identity is linked again by its address. The
   link must be stored, or the sign-in is refused (and an account just created
   for it is deleted). The password lockout (`AUTH_MAX_FAILED_ATTEMPTS`) does
   not apply here: it counts wrong passwords, and the IdP authenticated the
   user.
   - **A new account** is a `member`, confirmed, with no password, and
     `sso_only: true`: it signs in through the IdP only (see
     [What `sso_only` blocks](#what-sso_only-blocks)).
   - **Linking by email** (`SAML_LINK_EXISTING=on`) trusts the IdP with every
     address it asserts. Turn it on only when its users cannot edit that
     attribute. Even then, an admin's account is never linked by its email,
     nor an account already linked to another identity at the same IdP: the
     address now names someone else there (a recycled mailbox), not the
     account's owner. A linked confirmed account stays as it was — its
     password, if it has one, still works, and it is not marked `sso_only`.
   - **A directory account** — one [SCIM](/docs/security/scim) created
     (`scim_provisioned_at`) or gave an `externalId` — is linked by its first
     SAML sign-in with `SAML_LINK_EXISTING` off when its `externalId` is the
     assertion's NameID: the directory named this very identity. By email
     alone it is linked only with `SAML_LINK_DIRECTORY=on`, which says the
     directory and the IdP are one organisation's and its users cannot edit
     their email attribute — otherwise an IdP user who sets someone's address
     would take that account. With no password at that point it becomes
     `sso_only`. An admin's is still refused.
   - **An unconfirmed account** is linked with linking off only where it
     cannot have been used: when `auth_require_confirmed_email` is `true`
     (nobody signs in to an unconfirmed account, so it is a placeholder
     someone typed) and a first sign-in may create accounts
     (`SAML_PROVISION`). It then loses every credential
     (`SamlSso.take_over_unconfirmed` calls `User#revoke_credentials`, which
     the OAuth scaffold uses too) and becomes `sso_only`. With the default —
     unconfirmed accounts sign in — most real accounts are "unconfirmed", and
     handing one to whoever the identity provider says has that address is the
     takeover linking-off exists to prevent: the sign-in is refused, and the
     log says to confirm or delete the account, or to require confirmation.
     With open sign-up (`AUTH_SIGNUP=open`) confirmation is always required,
     so an address registered in wait never blocks its owner. An
     admin's account is refused either way.

### What `revoke_credentials` removes

`User#revoke_credentials` deletes, where the app has the model, every row
tied to the account — `OauthIdentity`, `SamlIdentity`, `PasskeyCredential`,
`ApiToken`, `TwoFactor`, `MagicLink`, `Device` (push registrations), by
`user_id`; the OpenID Connect provider's `OauthConsent`,
`OauthAuthorizationCode`, `OauthDeviceCode`, `OauthRefreshToken`, by
`user_key` — then clears the password, a pending reset link and the
remember-me digest, and bumps `session_version`, which ends every open
session. The list is `USER_CREDENTIAL_MODELS` in `app/models/user.sl`, as
`[class, key field]` pairs: add your own models there. The rows go first and
the account last, and a failure raises: a takeover that stops halfway leaves
the account as it was and links nothing, and running it again finishes it.

### What `sso_only` blocks

An `sso_only` account signs in through the identity provider only. It gets
no password sign-in (refused with the same answer and the same Argon2id cost
as a wrong password, and wrong passwords typed for it are not counted, so
nobody can lock it), no password reset, no magic link (asking answers as for
anyone, and a link issued before is refused), no new API token (`403`) and no
use of one made before, no passkey registration (`403`) and no sign-in with
one registered before.

It does **not** end what is already open at once. A session opened through
the IdP lasts until its end after the user is removed there — `SAML_SESSION_TTL`
(8 hours by default) or the IdP's `SessionNotOnOrAfter`, whichever is sooner;
the ACS stores it as `sign_in_expires_at`, which `load_current_user` enforces
— and an OAuth identity already linked to the account (`soli generate oauth`)
still signs in. Ending those at once takes a [SCIM](/docs/security/scim)
deactivation (every request of a deactivated user is a guest's) or
`revoke_credentials`. An app whose auth scaffold predates `sign_in_expires_at`
needs the check copied into `app/middleware/current_user.sl` (see a freshly
generated one).

5. A new session is opened and the browser goes to `return_to` (a path on this
   site, or `/`).

A refused response renders the sign-in page with a generic message and a 401;
the reason goes to the log (`SAML sign-in refused: …`).

**Why a cookie and not the session.** The response is a cross-site POST, and a
`SameSite=Lax` session cookie does not accompany one. The request cookie is
`SameSite=None; Secure; HttpOnly; Path=/`, encrypted, and lives ten minutes.
With an `https://` `APP_BASE_URL` it is named `__Host-saml_request`: a browser
then keeps it only Secure, for this host alone, with no `Domain`, so a sibling
subdomain cannot plant or overwrite it. Over plain `http://` (local
development on `localhost`) a browser drops a `__Host-` cookie, so there
it is plain `saml_request` — still `Secure`, which `localhost` accepts and
every other plain-http origin does not. For the same reason `/saml/acs` is exempt from the CSRF origin gate:
what authenticates that request is the signature.

**With two-factor.** A user who turned on [two-factor](/docs/security/authentication#two-factor)
is asked for their code after a SAML sign-in too. If the identity provider
already enforces it, mark the session in `SamlController#acs`:
`session_set("two_factor", "ok:#{user._key}")`.

## What is verified

`Saml.verify_response` raises unless all of this holds:

- **Signature.** The assertion, the response, or both are signed, and every
  signature present verifies with a certificate you configured — never with
  the one the message carries. A certificate with an RSA key under 2048 bits
  is refused.
- **No signature wrapping.** A signature is only looked for as a direct child
  of the response or of its one assertion, it must reference that parent's
  `ID`, and the claims are read from the very node the digest was computed
  over. A second assertion, a duplicated `ID`, or a signature that points
  elsewhere is refused.
- **For this application.** `Audience` is your entity id; `Recipient` and
  `Destination` are your ACS URL; `Issuer` is the IdP you name in
  `idp_entity_id`. That key is required: an empty string raises (an unset
  environment variable is a missing setting, not "any IdP"), and accepting any
  issuer the certificate signs for takes `any_issuer: true`, said by name.
- **For this request.** `InResponseTo` is the id of the request this browser
  started. A response nobody asked for (IdP-initiated) is refused unless you
  pass `allow_unsolicited: true`.
- **Now.** `NotBefore` / `NotOnOrAfter` on the conditions and on the subject
  confirmation, with 120 seconds of clock skew (`clock_skew`, up to 600).
- **Successful.** The status is `Success`; otherwise the error names the status.
- **Reasonable in shape.** Before any of the above is walked, a response with
  more than 10,000 elements, more than 64 distinct namespace prefixes or more
  than 30,000 namespace declarations is refused. A real one has a few dozen
  elements and half a dozen prefixes (an IdP that repeats `xmlns:xs` and
  `xmlns:xsi` on every `AttributeValue` makes two declarations per value, and
  passes); through 2.39.2 a 1 MiB unsigned response with thousands of prefixes
  cost tens of seconds of CPU. A signature's `SignedInfo` must hold its
  `CanonicalizationMethod`, its `SignatureMethod` and one `Reference`, nothing
  else, and an `InclusiveNamespaces` `PrefixList` may name at most 64
  prefixes: before that limit, an unsigned response whose transform listed a
  hundred thousand prefixes cost minutes of CPU before the key was consulted.

Comments inside a value do not split it: `user@example.com<!---->.evil.com`
reads as the whole signed text. A namespace prefix bound to an empty URI
(`xmlns:saml=""`, which XML 1.0 forbids) or used without a declaration is
refused: the canonical form cannot express either, so through 2.38.0 such a
declaration could hide a signed element from the reader without changing the
digest.

`verify_response` checks one response; it keeps no state. That an assertion
signs in only once is the application's to enforce — `soli generate saml`
does it with `SamlSso.spend`, keyed on `assertion_id`.

## The `Saml` class

Stateless, like [`WebAuthn`](/docs/builtins/webauthn). Every failed check
raises.

### Saml.auth_request(options)

```soli
request = Saml.auth_request({
  "idp_sso_url": "https://idp.example.com/sso",
  "sp_entity_id": "https://app.example.com/saml/metadata",
  "acs_url": "https://app.example.com/saml/acs"
})
request["id"]    # "_3f1c…" — keep it; the response must answer it
request["url"]   # redirect the browser here (HTTP-Redirect binding)
request["xml"]   # the <samlp:AuthnRequest>
```

Options: `relay_state` (up to 80 bytes, returned by the IdP as `RelayState`),
`force_authn: true`, `name_id_format`.

### Saml.verify_response(saml_response, expected)

`saml_response` is the `SAMLResponse` form field as posted (base64).

```soli
assertion = Saml.verify_response(params["SAMLResponse"], {
  "certificates": [idp_certificate],          # PEM or bare base64; or "certificate"
  "idp_entity_id": "https://idp.example.com/metadata",
  "audience": "https://app.example.com/saml/metadata",
  "acs_url": "https://app.example.com/saml/acs",
  "request_id": request_id
})
```

| Key | |
|---|---|
| `name_id`, `name_id_format` | the subject |
| `name_qualifier`, `sp_name_qualifier` | the NameID's `NameQualifier` / `SPNameQualifier` (`nil` when absent): what scopes a persistent or transient id — the same value under another qualifier is another principal |
| `attributes` | `{name: [values]}` — always arrays |
| `issuer` | the identity provider's entity id |
| `session_index`, `session_expires_at` | the IdP session (`nil` when not stated; times are Unix seconds) |
| `authn_context` | the `AuthnContextClassRef` |
| `assertion_id`, `in_response_to` | ids |
| `response_id` | the response's id — `nil` when only the assertion is signed, since nothing vouches for it then |
| `expires_at` | when the assertion stops being acceptable: the earlier of the conditions' and the bearer confirmation's `NotOnOrAfter` |
| `signed` | `"assertion"`, `"response"` or `"both"` |
| `certificate_expires_at`, `certificate_expired` | the `notAfter` (Unix seconds) of the certificate whose key signed the claims, and whether it has passed — watch it and install the IdP's next certificate before it lapses |

Other `expected` keys: `allow_unsolicited`, `any_issuer` (skip the `Issuer`
check when `idp_entity_id` is not given), `clock_skew`, `allow_sha1` (SHA-1
signatures are refused otherwise), `require_valid_certificate` (refuse a
signature made with a certificate outside its validity period — off by
default, because SAML treats the certificate as a key container and identity
providers do sign with expired ones; the result reports it either way), `now`
(Unix seconds, for specs). Listing
several certificates accepts any of them — that is how an IdP's certificate is
rotated without downtime.

The certificate is a pinned key, not a chain: its validity dates are not
checked, so an expired certificate you still list keeps verifying. Remove a
certificate from the list to stop trusting it.

### Saml.metadata(options)

The service provider's metadata XML, from `sp_entity_id` and `acs_url`.
`name_id_format` defaults to `emailAddress`; the generated `SamlSso.metadata`
passes `persistent` (see [Setup](#setup)).

### Saml.parse_metadata(xml)

```soli
idp = Saml.parse_metadata(File.read("idp-metadata.xml"))
idp["entity_id"]      # "https://idp.example.com/metadata"
idp["sso_url"]        # the HTTP-Redirect SingleSignOnService
idp["certificates"]   # signing certificates, PEM
```

The metadata's own signature is not checked: read it from a file you were
given or an `https://` URL you trust.

### Saml.test_response(options)

A signed response, as an identity provider would post it — the stand-in a spec
signs in with. It signs what it is told with a key you supply
(`openssl req -x509 -newkey rsa:2048 -nodes -keyout idp.key -out idp.crt -days 3650 -subj "/CN=test"`).

```soli
response = Saml.test_response({
  "private_key": File.read("tests/fixtures/idp.key"),
  "issuer": "https://idp.test/metadata",
  "acs_url": "https://app.test/saml/acs",
  "audience": "https://app.test/saml/metadata",
  "name_id": "ada@example.com",
  "request_id": request_id,
  "attributes": {"groups": ["admin", "staff"]}
})
```

Options: `sign` (`"assertion"` by default, `"response"`, `"both"`, `"none"`),
`name_id_format`, `session_index`, `status`, `now`, `lifetime` (seconds, 300),
`response_id`, `assertion_id`.

## Limits

- **One identity provider per application.** For one per customer, make
  `SamlSso.identity_provider` read it from the tenant instead of the
  environment — and scope the accounts per tenant too (a database per tenant,
  or a tenant key on `User` and `SamlIdentity` that `user_for` filters on).
  Keep `SAML_LINK_EXISTING` off there: each customer's IdP asserts whatever
  addresses its own administrator types, so linking by email would let one
  tenant's IdP sign in to another tenant's accounts.
- **Not supported, and refused by name:** encrypted assertions and
  `EncryptedID` (turn assertion encryption off at the IdP), ECDSA and RSA-PSS
  signatures, SHA-384, inclusive canonicalization. `SignatureMethod` takes
  only `rsa-sha256`/`rsa-sha512` (and `rsa-sha1` with `allow_sha1`),
  `DigestMethod` only `sha256`/`sha512` (and `sha1`, also only with
  `allow_sha1`).
- **Requests are not signed**, and the HTTP-POST binding is not offered for
  them. Most identity providers accept unsigned requests.
- **No single logout.** Signing out of the application does not sign out of
  the identity provider, and the reverse. A SAML session here ends after
  `SAML_SESSION_TTL` instead.
- **No account deprovisioning.** A user removed at the IdP can no longer sign
  in through it, and an `sso_only` account has no other way in; but the
  account and its open sessions remain — that is what
  [SCIM](/docs/security/scim) is for.
- **Attributes are not mapped to roles.** `assertion["attributes"]` is there
  in `SamlSso.user_for` if you want to.

## See also

- [Authentication](/docs/security/authentication) — sessions, two-factor, passkeys
- [OAuth Client](/docs/security/oauth-client) — "Sign in with Google / Microsoft / GitHub"
- [OIDC Provider](/docs/security/oidc-provider) — being the identity provider
