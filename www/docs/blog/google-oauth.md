# Implementing Google OAuth in SoliLang

<figure style="margin:1.5rem auto;max-width:1024px;">
  <img src="/images/blog/google-oauth.jpg" width="1024" height="576" alt="Implementing Google OAuth in Soli: Sign in with Google button and OAuth 2.0 flow showing redirect to Google, code exchange, and secure session creation." style="display:block;width:100%;height:auto;border-radius:12px;border:1px solid #30363d;background:#0b0d0f;">
</figure>

This guide walks you through implementing Google OAuth authentication in your SoliLang application.

> **The short way:** `soli generate auth && soli generate oauth google` writes
> all of this — plus account linking, re-authentication before linking, and the
> refusals below — see [OAuth Client](/docs/security/oauth-client). This post
> builds it by hand, to show what each piece is for.

## Prerequisites

- A SoliLang project with session support
- A Google Cloud Platform project with OAuth credentials

## Step 1: Set Up Google OAuth Credentials

1. Go to the [Google Cloud Console](https://console.cloud.google.com/)
2. Navigate to **APIs & Services** > **Credentials**
3. Click **Create Credentials** > **OAuth client ID**
4. Configure the OAuth consent screen
5. Create credentials with:
   - **Application type**: Web application
   - **Authorized redirect URIs**: `http://localhost:3000/auth/google/callback`

## Step 2: Store Environment Variables

```bash
GOOGLE_CLIENT_ID=your-client-id.apps.googleusercontent.com
GOOGLE_CLIENT_SECRET=your-client-secret
GOOGLE_REDIRECT_URI=http://localhost:3000/auth/google/callback
```

## Step 3: Create the OAuth Routes

```soli
# app/controllers/google_auth_controller.sl

# GET /auth/google — to Google, with a fresh state and PKCE challenge
def google_login(req)
  state = Crypto.random_hex(32)
  verifier = Crypto.random_hex(32)
  session_set("oauth_state", state)
  session_set("oauth_verifier", verifier)
  # PKCE S256: the unpadded base64url of the SHA-256 of the verifier.
  challenge = Base64.urlsafe_encode(Hex.decode(Crypto.sha256(verifier)))

  query = [
    "client_id=#{url_encode(getenv("GOOGLE_CLIENT_ID").to_s)}",
    "redirect_uri=#{url_encode(getenv("GOOGLE_REDIRECT_URI").to_s)}",
    "response_type=code",
    "scope=#{url_encode("openid email profile")}",
    "state=#{state}",
    "code_challenge=#{challenge}",
    "code_challenge_method=S256"
  ]
  redirect_external("https://accounts.google.com/o/oauth2/v2/auth?#{query.join("&")}")
end

# GET /auth/google/callback
def google_callback(req)
  expected = session_get("oauth_state").to_s
  verifier = session_get("oauth_verifier").to_s
  session_delete("oauth_state")
  session_delete("oauth_verifier")

  # Verify state to prevent CSRF. Both sides must be present — two missing
  # values are equal, so `!=` would wave a forged callback through — and the
  # comparison is constant-time.
  given = params["state"].to_s
  unless expected.present? && given.present? && Crypto.secure_compare(expected, given)
    return {"status": 403, "body": "Invalid state parameter"}
  end

  # Denied, or refused by Google: log what it said, show none of it.
  if params["error"].present? || params["code"].blank?
    Logger.warn("Google refused: #{params["error"]} #{params["error_description"]}")
    return {"status": 401, "body": "Sign-in was cancelled."}
  end

  try
    tokens = _google_exchange_code(params["code"].to_s, verifier)
    profile = _google_userinfo(tokens["access_token"])
    # An address Google did not verify is not proof of anything.
    throw "email not verified" unless profile["email_verified"].to_s == "true"

    user = User.find_or_create_from_google(profile)
    session_regenerate()
    session_set("user_id", user._key)
    return redirect("/dashboard")
  catch error
    Logger.warn("Google sign-in failed: #{error}")
    return {"status": 401, "body": "Sign-in failed. Try again."}
  end
end

# Form-encoded, every value escaped: a secret holding `&`, `+` or `=` would
# otherwise cut the body in the wrong place. `HTTP.request` is the form that
# sends headers and hands back the status with the body.
def _google_exchange_code(code, verifier)
  body = [
    "code=#{url_encode(code)}",
    "client_id=#{url_encode(getenv("GOOGLE_CLIENT_ID").to_s)}",
    "client_secret=#{url_encode(getenv("GOOGLE_CLIENT_SECRET").to_s)}",
    "redirect_uri=#{url_encode(getenv("GOOGLE_REDIRECT_URI").to_s)}",
    "grant_type=authorization_code",
    "code_verifier=#{url_encode(verifier)}"
  ].join("&")
  response = HTTP.request(
    "POST",
    "https://oauth2.googleapis.com/token",
    {"Content-Type": "application/x-www-form-urlencoded", "Accept": "application/json"},
    body
  )
  _google_json(response)
end

def _google_userinfo(access_token)
  response = HTTP.request(
    "GET",
    "https://openidconnect.googleapis.com/v1/userinfo",
    {"Authorization": "Bearer #{access_token}", "Accept": "application/json"}
  )
  _google_json(response)
end

def _google_json(response)
  status = response["status"]
  throw "Google answered HTTP #{status}" unless status >= 200 && status < 300

  json_parse(response["body"].to_s)
end
```

Four things in there are easy to get wrong:

- **State and PKCE live in the session.** `session_set` / `session_get`;
  there is no writable `req["session"]`.
- **Compare the state safely.** Both sides must be present — with `!=`, a
  callback carrying no state matched a session holding none — and
  `Crypto.secure_compare` does not leak where they differ.
- **Encode the token request.** Every value goes through `url_encode`: a
  client secret holding `&`, `+` or `=` would otherwise cut the body
  somewhere else.
- **Check `email_verified`.** Google says whether it verified the address
  (a boolean, or the string `"true"`); an unverified one proves nothing.

## Step 4: Create the User Model

```soli
# app/models/user.sl

class User < Model
  # Fields: google_id, email, name, avatar_url

  # The account linked to this Google `sub` (its stable id), else a new one.
  static def find_or_create_from_google(profile)
    existing = User.find_by("google_id", profile["sub"])
    return existing unless existing.nil?

    User.create({
      "google_id": profile["sub"],
      "email": profile["email"],
      "name": profile["name"],
      "avatar_url": profile["picture"]
    })
  end
end
```

A sign-in finds the account by Google's `sub`, never by address alone.

## Step 5: Configure Routes

```soli
# config/routes.sl

get("/auth/google", "google_auth#google_login")
get("/auth/google/callback", "google_auth#google_callback")
```

## Step 6: Add a Login Button

```html
<!-- app/views/sessions/new.sl.html -->
<a href="/auth/google" class="btn btn-google">
  <img src="/images/google-icon.svg" alt="Google" />
  Sign in with Google
</a>
```

## Security Considerations

1. **Verify the state parameter** — done above: stored with `session_set`,
   compared with `Crypto.secure_compare`, refused when either side is missing.
2. **Use PKCE** — the `S256` challenge binds the code to the browser that asked for it.
3. **Trust only a verified address** — `email_verified` must be true.
4. **Don't echo provider errors** — `error_description` comes from the query
   string; log it, show a generic message.
5. **Secure sessions** — regenerate the session id after sign-in
   (`session_regenerate()`), and serve over HTTPS.

## Conclusion

Google OAuth integration provides a secure way to authenticate users. This guide covered the essential steps, but you can extend it with:

- Refresh token handling for long-lived sessions
- Profile synchronization
- Multi-provider OAuth (GitHub, Facebook, etc.)
- Email verification requirements
