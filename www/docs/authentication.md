# Authentication with JWT

SoliLang provides built-in JWT (JSON Web Token) functions for stateless authentication.

## Creating Tokens

Use `jwt_sign()` to create tokens for authenticated users:

```soli
def login
  data = req["json"];
  username = data["username"];
  password = data["password"];

  # Verify credentials (example)
  if username == "admin" && password == "secret"
    payload = {
      "sub": username,
      "role": "admin",
      "name": "Administrator"
    };
    secret = getenv("JWT_SECRET");
    token = jwt_sign(payload, secret, {"expires_in": 3600});

    return {
      "status": 200,
      "body": json_stringify({
        "token": token,
        "expires_in": 3600
      })
    };
  end

  {"status": 401, "body": "Invalid credentials"}
end
```

## Verifying Tokens

Use `jwt_verify()` to validate tokens and extract claims:

```soli
def authenticate_middleware
  auth_header = req["headers"]["Authorization"];

  if (auth_header == null || !Regex.matches("^Bearer ", auth_header))
    return {"status": 401, "body": "Missing or invalid Authorization header"};
  end

  token = Regex.replace("^Bearer ", auth_header, "");
  result = jwt_verify(token, getenv("JWT_SECRET"));

  if result["error"] == true
    return {"status": 401, "body": "Invalid token: " + result["message"]};
  end

  # Token is valid - add user info to request
  req["current_user"] = result;
  null  # Continue to next middleware/controller
end
```

## Decoding Tokens (Unsafe — Inspection Only)

`jwt_decode_unsafe()` reads token claims **without** verification. The result is wrapped as `{unverified: true, claims: {...}}` so it cannot be confused with a verified `jwt_verify` response. **Never trust these claims for authentication** — use `jwt_verify(token, secret)` for that.

```soli
def get_token_info
  token = req["headers"]["Authorization"];
  token = Regex.replace("^Bearer ", token, "");

  # SEC-029: explicit "I am NOT verifying" — inspection only.
  let result = jwt_decode_unsafe(token);
  let claims = result["claims"];

  {
    "status": 200,
    "body": json_stringify({
      "unverified": true,
      "subject": claims["sub"],
      "issued_at": claims["iat"],
      "expires": claims["exp"]
    })
  }
end
```

The previous `jwt_decode(token)` builtin returned the same shape as `jwt_verify`, which made `claims["sub"]` a silent auth bypass when the caller forgot the verification step. It was removed in SEC-029; calling it raises a migration error.

## API Reference

### jwt_sign

```soli
jwt_sign(payload, secret)
jwt_sign(payload, secret, options)
```

| Parameter | Type | Description |
|-----------|------|-------------|
| `payload` | Hash | Claims to include in the token |
| `secret` | String | Secret key for signing |
| `options.expires_in` | Int | Expiration in seconds from now (optional) |
| `options.exp` | Int | Expiration as an absolute Unix timestamp (optional, not with `expires_in`) |
| `options.iat` | Int | Issued-at as an absolute Unix timestamp (optional, default: now) |
| `options.algorithm` | String | Algorithm: "HS256", "HS384", "HS512", "RS256", "ES256", "EdDSA" (optional) |
| `options.key` | String | PEM-encoded private key for RS256/ES256/EdDSA (optional; ES256 takes a P-256 key in PKCS#8 form, as Apple's `.p8`) |

### jwt_verify

```soli
jwt_verify(token, secret)
jwt_verify(token, secret, options)
```

| Parameter | Type | Description |
|-----------|------|-------------|
| `token` | String | The JWT token |
| `secret` | String | Secret key for HMAC, or public key material for RS256/ES256/EdDSA |
| `options.key` | String | PEM-encoded public key for RS256/ES256/EdDSA (optional) |

Returns a hash with claims if valid, or `{"error": true, "message": "..."}` if invalid.

### jwt_decode_unsafe

```soli
jwt_decode_unsafe(token)
```

Returns `{unverified: true, claims: {...}}` without verifying signature or expiration. Use only for inspection or debugging; never for authentication. The result wrapper makes it impossible to do `result["sub"]` and silently trust an attacker-forged claim — use `result["claims"]["sub"]` and accept it only if you've verified the token elsewhere.

The legacy `jwt_decode(token)` was removed in SEC-029 because it returned the same shape as `jwt_verify`, making accidental misuse a one-character bug.

## Best Practices

1. **Always use HTTPS** in production
2. **Set appropriate expiration times** for tokens
3. **Store secrets securely** in environment variables
4. **Validate all claims** before trusting token data
5. **Consider refresh tokens** for long-lived sessions

## Two-factor

A second factor for password sign-in: the six-digit code of an authenticator
app (TOTP, RFC 6238 — 1Password, Google Authenticator, Authy…), with backup
codes for a lost device.

### Generate it

```bash
soli generate auth --two-factor
```

On a new app this writes session auth and two-factor together; on an app that
already ran `soli generate auth` it adds only the two-factor files. It combines
with `--passkeys`. **It edits no existing file** — everything below is new, and
nothing is overwritten:

| File | |
|---|---|
| `app/models/two_factor.sl` | `TwoFactor`, one record per user: the secret (encrypted), the last accepted time step, the backup-code digests, the wrong-code counter |
| `app/middleware/two_factor.sl` | the gate: a session that passed one factor only is a guest, and its page loads go to the challenge |
| `app/controllers/two_factor_controller.sl` | `GET /two_factor`, `POST /two_factor/setup`, `POST /two_factor/confirm`, `POST /two_factor/backup_codes`, `DELETE /two_factor`, `GET` and `POST /two_factor/challenge` |
| `app/views/two_factor/*.html.slv` | settings, enrolment (QR code and key), backup codes, challenge |
| `db/migrations/…_create_two_factors.sl` | unique index on `user_id` |

Run `soli db:migrate up`, set `SOLI_ENCRYPTION_KEY` (the secret is stored with
[`encrypts`](/docs/database/models#encrypted-attributes); `Crypto.random_hex(32)`
makes one), and link to `/two_factor` from your account page.
`TWO_FACTOR_ISSUER` is the name authenticator apps show next to the account.

### How it works

- **Enrolling.** `/two_factor` → *Set up* shows the secret as a QR code
  (`qr_svg`, rendered on the server) and as text. Two-factor turns on only once
  the user typed back a first code, and the page then shows ten backup codes,
  once. Turning it on signs every other session of the account out.
- **Signing in.** The password step is unchanged. The middleware runs after
  `load_current_user`: if the user has two-factor on and this session has not
  passed the challenge, the request continues as a guest (`current_user()` is
  `nil`), and a `GET` is redirected to `/two_factor/challenge`. Because the
  gate is on the session rather than in the sign-in action, it covers every way
  a session opens — password, password reset, the remember-me cookie, OAuth —
  and a path you add later.
- **Codes.** A code is accepted once: the record keeps the time step of the
  last accepted code and refuses that step and earlier ones
  ([`Crypto.totp_step`](/docs/builtins/crypto#def-totp-step)). A backup code
  works once, typed with or without its dash. Codes may be typed with spaces.
- **Wrong codes.** The endpoints share the sign-in form's per-IP throttle.
  `AUTH_MAX_FAILED_ATTEMPTS` wrong codes in a row lock the second factor for
  `AUTH_LOCKOUT_SECONDS`; while locked, a right code is answered like a wrong
  one. The counter lives on the `TwoFactor` record, not the user: the password
  step clears the user's own counter, so anyone holding the password could
  otherwise reset it between guesses.
- **Turning it off, new backup codes.** Both ask for the password (an account
  created through OAuth has none, and is not asked); turning it off also asks
  for a code.

### With remember-me, passkeys and OAuth

| Sign-in | Second factor |
|---|---|
| Password | asked |
| Password reset link | asked — a mailbox alone is one factor |
| Remember-me cookie | asked when the session it restores is new: the cookie replaces the password, not the code |
| OAuth (`soli generate oauth`) | asked |
| Passkey (`--passkeys`) | skipped when the passkey verified its user (screen lock, PIN, biometrics): that is already two factors |

A passkey scaffold generated before `--two-factor` existed does not mark its
session, so its sign-in is followed by the challenge; the generator prints the
one line to add to `passkey_sessions#create`:

```soli
session_set("two_factor", "ok:#{user._key}") if result["user_verified"]
```

**Limits.** Authenticator apps only: no SMS or email codes, and no "trust this
device for 30 days". There is no admin screen to reset a user who lost both
device and backup codes — delete their `TwoFactor` record. A session open on
another device when a user turns two-factor *off* keeps its state until it
ends.

## Passkeys

Passkeys (WebAuthn) replace the password with a key pair the user's device
keeps: the server stores only a public key, and a signature over a fresh
challenge proves possession. They cannot be phished — the browser binds every
response to the site's origin — and there is nothing to leak or reuse.

### Generate it

```bash
soli generate auth --passkeys
```

On a new app this writes session auth and passkeys together; on an app that
already ran `soli generate auth` it adds only the passkey files. The flag goes
before or after the folder. It writes, never overwriting a file that exists:

| File | |
|---|---|
| `app/models/passkey_credential.sl` | `PasskeyCredential` (one row per passkey) and the challenge helpers |
| `app/controllers/passkeys_controller.sl` | `GET /passkeys`, `POST /passkeys/options`, `POST /passkeys`, `DELETE /passkeys/:id` — the signed-in user's passkeys |
| `app/controllers/passkey_sessions_controller.sl` | `POST /passkeys/login/options`, `POST /passkeys/login` — sign-in |
| `app/views/passkeys/index.html.slv` | list, add, remove |
| `app/views/passkeys/_sign_in.html.slv` | the sign-in button, for `partial("passkeys/sign_in")` |
| `public/js/passkeys.js` | the browser side, no dependency |
| `db/migrations/…_create_passkey_credentials.sl` | unique index on `credential_id`, index on `user_id` |

On a fresh app the sign-in view already renders the partial and its email
field has `autocomplete="username webauthn"`, so the browser also offers
passkeys in its autofill (conditional UI). An existing sign-in view is left
alone: add `<%- partial("passkeys/sign_in") %>` and the `autocomplete` value
yourself. Run `soli db:migrate up`, sign in, and add a passkey at `/passkeys`.

### Relying party and origins

| Variable | Default | |
|---|---|---|
| `WEBAUTHN_RP_ID` | the host of `APP_BASE_URL` | the domain passkeys belong to; set the parent domain (`example.com`) to share them with `app.example.com` |
| `WEBAUTHN_ORIGINS` | `APP_BASE_URL` | comma-separated, exact origins (scheme, host, port) a ceremony may come from |
| `WEBAUTHN_RP_NAME` | the RP ID | shown by the browser |

Browsers allow passkeys on `https://` and on `http://localhost` only. The
default `APP_BASE_URL` (`http://localhost:5011`) works in development as long
as you browse that exact origin; behind a proxy, set `APP_BASE_URL` to the
public `https://` URL.

### How sign-in works

1. `POST /passkeys/login/options` returns request options with an empty
   `allowCredentials` (discoverable credentials: no username needed) and keeps
   the challenge in the session under `webauthn_challenge`, tagged with its
   purpose and an expiry (`WEBAUTHN_CHALLENGE_TTL`, 300 s).
2. The browser signs it; `passkeys.js` posts the credential's `toJSON()`.
3. `POST /passkeys/login` takes the challenge out of the session *before*
   verifying (a replay or a second tab fails closed), finds the passkey by
   `credential_id` with `find_by`, verifies it with
   `WebAuthn.verify_authentication`, and checks that the user handle is present
   and names the same account.
4. It then opens the session exactly as the password login does: the per-IP
   throttle (`AUTH_ATTEMPTS_PER_IP`), the locked and confirmed gates,
   `session_regenerate()`, `user_id` and `session_version`.

Every failure — unknown credential, bad signature, expired challenge, locked
account, a sign counter that went backwards — gets the same `401`, so the
endpoint reveals nothing about which passkeys exist. The JSON requests carry
the page's CSRF token in `X-CSRF-Token` (the views emit it in
`data-csrf-token`), so they also pass with `SOLI_CSRF_TOKENS=require`.

### Hand-rolled ceremonies

The generated code is a thin layer over the native `WebAuthn` class —
`registration_options`, `verify_registration`, `authentication_options`,
`verify_authentication` — documented in
[Built-ins → WebAuthn](/docs/builtins/webauthn). Use it directly for an API or
a different storage model.

**Limits.** Attestation is never trusted against a vendor root (the options ask for
`"none"`; a packed self-attestation is checked, a certificate chain only
reported), so passkeys cannot be restricted to certain authenticator models.
ES256, EdDSA (Ed25519) and RS256 are supported. Passkeys are an additional credential: an
account still has a password.

**In a native app.** `AppLinks.android(…, {"passkeys": true})` and
`AppLinks.apple(…, {"passkeys": true})` let the app use the site's passkeys,
and `AppLinks.android_origin(fingerprint)` gives the `android:apk-key-hash:…`
origin to add to `WEBAUTHN_ORIGINS` — see
[Deep links → Passkeys in the app](/docs/native/deep-links#passkeys).
