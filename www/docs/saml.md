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
| `db/migrations/…_create_saml_identities.sl` | unique index on `(issuer, name_id)`, index on `user_id` |
| `db/migrations/…_create_saml_assertions.sl` | index on `expires_at` |
| `config/routes.sl` | the three routes, and `skip_csrf("/saml/acs")` |

Nothing existing is overwritten. Add the link to your sign-in view:

```erb
<a href="/saml/login">Sign in with SSO</a>
```

## Setup

1. `soli db:migrate up`
2. Set `APP_BASE_URL` to the `https://` origin the identity provider reaches
   the application at, and `SOLI_SESSION_SECRET` (32+ characters).
3. Give the identity provider's administrator `https://your.app/saml/metadata`.
   It states the two values every IdP asks for:

   | | Default |
   |---|---|
   | Entity ID (audience) | `APP_BASE_URL/saml/metadata` — override with `SAML_SP_ENTITY_ID` |
   | ACS URL (reply URL) | `APP_BASE_URL/saml/acs` |

4. Point the application at the identity provider, either way:

   | Variable | |
   |---|---|
   | `SAML_IDP_METADATA_FILE` | path to the IdP's metadata XML, inside the app (the server reads no file outside it) |
   | `SAML_IDP_ENTITY_ID`, `SAML_IDP_SSO_URL`, `SAML_IDP_CERTIFICATE` | or its three values: entity id, single sign-on URL (HTTP-Redirect binding), signing certificate (PEM; `\n` for newlines) |

The IdP must sign the assertion or the response (or both) with RSA-SHA256, and
send an email address — as the NameID, or in an attribute named `email`,
`mail`, `emailaddress`, or the usual claim URIs.

## How a sign-in runs

1. `/saml/login` builds the request (`Saml.auth_request`) and redirects to the
   IdP. The request id and the `return_to` path go in an encrypted cookie.
2. The user authenticates at the IdP, which posts `SAMLResponse` to
   `/saml/acs`.
3. `SamlSso.finish` takes the request id out of the cookie — once — and
   `Saml.verify_response` checks the response against it. `SamlSso.spend`
   then records the assertion's id until it expires: a response posted again,
   even with a copy of the request cookie, is refused.
4. `SamlSso.user_for` finds the account: the one already linked to this
   `(issuer, NameID)`; else the one with that email, linked from now on; else
   a new one. A new account is a `member` with no password — it signs in
   through the IdP only. `SAML_PROVISION=off` refuses anyone with no account.
   An account whose email was never confirmed loses its password, remember-me
   cookie and sessions when it is linked (`SamlSso.take_over_unconfirmed`, as
   in the OAuth scaffold): with open sign-up, whoever typed that address first
   kept a way into the account the real owner now uses.
   An admin's account is never linked by its email: link it by hand
   (`SamlIdentity.create({"user_id", "issuer", "name_id", "email"})`).
   `SAML_LINK_EXISTING=off` links no existing account at all — set it when the
   IdP lets users edit the email attribute it sends.
5. A new session is opened and the browser goes to `return_to` (a path on this
   site, or `/`).

A refused response renders the sign-in page with a generic message and a 401;
the reason goes to the log (`SAML sign-in refused: …`).

**Why a cookie and not the session.** The response is a cross-site POST, and a
`SameSite=Lax` session cookie does not accompany one. The request cookie is
`SameSite=None; Secure; HttpOnly`, scoped to `/saml`, encrypted, and lives ten
minutes. For the same reason `/saml/acs` is exempt from the CSRF origin gate:
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
  `Destination` are your ACS URL; `Issuer` is the configured IdP.
- **For this request.** `InResponseTo` is the id of the request this browser
  started. A response nobody asked for (IdP-initiated) is refused unless you
  pass `allow_unsolicited: true`.
- **Now.** `NotBefore` / `NotOnOrAfter` on the conditions and on the subject
  confirmation, with 120 seconds of clock skew (`clock_skew`, up to 600).
- **Successful.** The status is `Success`; otherwise the error names the status.

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
| `attributes` | `{name: [values]}` — always arrays |
| `issuer` | the identity provider's entity id |
| `session_index`, `session_expires_at` | the IdP session (`nil` when not stated; times are Unix seconds) |
| `authn_context` | the `AuthnContextClassRef` |
| `assertion_id`, `response_id`, `in_response_to` | ids |
| `expires_at` | the conditions' `NotOnOrAfter` |
| `signed` | `"assertion"`, `"response"` or `"both"` |

Other `expected` keys: `allow_unsolicited`, `clock_skew`, `allow_sha1`
(SHA-1 signatures are refused otherwise), `now` (Unix seconds, for specs).
Listing several certificates accepts any of them — that is how an IdP's
certificate is rotated without downtime.

### Saml.metadata(options)

The service provider's metadata XML, from `sp_entity_id` and `acs_url`
(`name_id_format` defaults to `emailAddress`).

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
  environment.
- **Not supported, and refused by name:** encrypted assertions and
  `EncryptedID` (turn assertion encryption off at the IdP), ECDSA and RSA-PSS
  signatures, SHA-384, inclusive canonicalization.
- **Requests are not signed**, and the HTTP-POST binding is not offered for
  them. Most identity providers accept unsigned requests.
- **No single logout.** Signing out of the application does not sign out of
  the identity provider, and the reverse.
- **No account deprovisioning.** A user removed at the IdP can no longer sign
  in, but their account and open sessions remain — that is what
  [SCIM](/docs/security/scim) is for.
- **Attributes are not mapped to roles.** `assertion["attributes"]` is there
  in `SamlSso.user_for` if you want to.

## See also

- [Authentication](/docs/security/authentication) — sessions, two-factor, passkeys
- [OAuth Client](/docs/security/oauth-client) — "Sign in with Google / Microsoft / GitHub"
- [OIDC Provider](/docs/security/oidc-provider) — being the identity provider
