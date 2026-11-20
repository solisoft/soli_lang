# Adding "Sign in with GitHub" to Your Soli App

<figure style="margin:1.5rem auto;max-width:1024px;">
  <img src="/images/blog/github-oauth.jpg" width="1024" height="576" alt="Adding Sign in with GitHub to a Soli app: OAuth 2.0 flow showing user clicking the button, redirect to GitHub, code exchange, and session creation." style="display:block;width:100%;height:auto;border-radius:12px;border:1px solid #30363d;background:#0b0d0f;">
</figure>

Most developer tools ship with GitHub login. It's familiar, fast, and your users already have an account. Here's how to wire it up in Soli — from creating the OAuth app to storing the user in your database.

> **The short way:** `soli generate auth && soli generate oauth github` writes
> all of this — plus account linking, re-authentication before linking, and the
> refusals below — see [OAuth Client](/docs/security/oauth-client). This post
> builds it by hand, to show what each piece is for.

## How OAuth 2.0 Works

Before we write code, here's the flow:

1. User clicks "Sign in with GitHub"
2. Your app redirects them to GitHub's authorization page
3. User approves, GitHub redirects back with a `code`
4. Your server exchanges that `code` for an `access_token`
5. Your server uses the token to fetch the user's profile
6. You create a session and redirect to the app

The user never gives you their password. GitHub handles authentication, you handle authorization.

## Step 1: Create a GitHub OAuth App

1. Go to [GitHub Developer Settings](https://github.com/settings/developers)
2. Click **New OAuth App**
3. Fill in:
   - **Application name**: Your app name
   - **Homepage URL**: `http://localhost:3000`
   - **Authorization callback URL**: `http://localhost:3000/auth/github/callback`
4. Copy the **Client ID** and generate a **Client Secret**

## Step 2: Environment Variables

```bash
GITHUB_CLIENT_ID=your-client-id
GITHUB_CLIENT_SECRET=your-client-secret
GITHUB_REDIRECT_URI=http://localhost:3000/auth/github/callback
```

Load these with `getenv()` — never hardcode secrets.

## Step 3: The Auth Controller

```soli
# app/controllers/github_auth_controller.sl

# GET /auth/github — to GitHub, with a fresh state and PKCE challenge
def github_login(req)
  state = Crypto.random_hex(32)
  verifier = Crypto.random_hex(32)
  session_set("oauth_state", state)
  session_set("oauth_verifier", verifier)
  # PKCE S256: the unpadded base64url of the SHA-256 of the verifier.
  challenge = Base64.urlsafe_encode(Hex.decode(Crypto.sha256(verifier)))

  query = [
    "client_id=#{url_encode(getenv("GITHUB_CLIENT_ID").to_s)}",
    "redirect_uri=#{url_encode(getenv("GITHUB_REDIRECT_URI").to_s)}",
    "scope=#{url_encode("read:user user:email")}",
    "state=#{state}",
    "code_challenge=#{challenge}",
    "code_challenge_method=S256"
  ]
  redirect_external("https://github.com/login/oauth/authorize?#{query.join("&")}")
end

# GET /auth/github/callback
def github_callback(req)
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

  # Denied, or refused by GitHub: log what it said, show none of it.
  if params["error"].present? || params["code"].blank?
    Logger.warn("GitHub refused: #{params["error"]} #{params["error_description"]}")
    return {"status": 401, "body": "Sign-in was cancelled."}
  end

  try
    token_data = _github_exchange_code(params["code"].to_s, verifier)
    access_token = token_data["access_token"]
    github_user = _github_get(access_token, "/user")
    email = _verified_primary_email(_github_get(access_token, "/user/emails"))
    user = User.find_or_create_from_github(github_user, email)

    session_regenerate()
    session_set("user_id", user._key)
    return redirect("/dashboard")
  catch error
    Logger.warn("GitHub sign-in failed: #{error}")
    return {"status": 401, "body": "Sign-in failed. Try again."}
  end
end

# Form-encoded, every value escaped. `HTTP.request` is the form that sends
# headers and hands back the status with the body.
def _github_exchange_code(code, verifier)
  body = [
    "client_id=#{url_encode(getenv("GITHUB_CLIENT_ID").to_s)}",
    "client_secret=#{url_encode(getenv("GITHUB_CLIENT_SECRET").to_s)}",
    "code=#{url_encode(code)}",
    "redirect_uri=#{url_encode(getenv("GITHUB_REDIRECT_URI").to_s)}",
    "code_verifier=#{url_encode(verifier)}"
  ].join("&")
  response = HTTP.request(
    "POST",
    "https://github.com/login/oauth/access_token",
    {"Content-Type": "application/x-www-form-urlencoded", "Accept": "application/json"},
    body
  )
  token_data = _github_json(response)
  throw "token exchange: #{token_data["error"]}" if token_data["error"]

  token_data
end

def _github_get(access_token, path)
  response = HTTP.request(
    "GET",
    "https://api.github.com#{path}",
    {"Authorization": "Bearer #{access_token}", "Accept": "application/json", "User-Agent": "my-app"}
  )
  _github_json(response)
end

def _github_json(response)
  status = response["status"]
  throw "GitHub answered HTTP #{status}" unless status >= 200 && status < 300

  json_parse(response["body"].to_s)
end

# The primary address, and only when GitHub verified it. The profile's public
# `email` is whatever the user typed in their profile: never trust it.
def _verified_primary_email(emails)
  primary = emails.find { |entry| entry["primary"] == true && entry["verified"] == true }
  primary.nil? ? nil : primary["email"]
end
```

The state and the PKCE verifier live in the session (`session_set` /
`session_get` — there is no writable `req["session"]`). The state check needs
both sides present: with `!=`, a callback carrying no state matched a session
holding none. GitHub supports PKCE, so the code is bound to this browser too.

Note the `Accept: application/json` header on the token exchange — without it,
GitHub returns the response as a URL-encoded string instead of JSON.

## Step 4: Which Address to Trust

The profile's `email` field is the public address the user typed in their
profile; GitHub never checked it. `/user/emails` lists the addresses with a
`verified` flag, and only the one that is both `primary` and `verified` is
taken (scope `user:email`). A user without one simply has no address on file.

## Step 5: The User Model

```soli
# app/models/user.sl

class User < Model
  # Fields: github_id, username, email, avatar_url

  # The account linked to this GitHub id, else a new one. An address alone
  # never opens an existing account: matching on it would hand that account
  # to whoever gets the address onto a GitHub profile.
  static def find_or_create_from_github(github_user, email)
    existing = User.find_by("github_id", github_user["id"])
    return existing unless existing.nil?

    User.create({
      "github_id": github_user["id"],
      "username": github_user["login"],
      "email": email,
      "avatar_url": github_user["avatar_url"]
    })
  end
end
```

A sign-in finds the account by GitHub id, never by address: linking on an
address would hand an existing account to whoever gets that address onto a
GitHub profile.

## Step 6: Routes

```soli
# config/routes.sl

get("/auth/github", "github_auth#github_login")
get("/auth/github/callback", "github_auth#github_callback")
```

## Step 7: The Login Button

```html
<!-- app/views/sessions/new.html.slv -->
<a href="/auth/github" class="btn">
  Sign in with GitHub
</a>
```

That's it. Click the link, approve on GitHub, land on your dashboard.

## Protecting Routes with Middleware

Once users can log in, you need to protect routes that require authentication:

```soli
# app/middleware/require_login.sl

# scope_only: true
def require_login(req)
  return redirect("/auth/github") if session_get("user_id").nil?

  req
end
```

```soli
# config/routes.sl

middleware("require_login", -> {
  get("/dashboard", "dashboard#index")
  get("/dashboard/settings", "dashboard#settings")
})
```

## Adding a Logout

Signing out changes state, so it is a `POST`: a `GET /logout` would sign out
anyone whose browser loads `<img src="https://your.app/logout">` on another
site. The POST goes through the CSRF gate like any other form.

```soli
# app/controllers/auth_controller.sl

# POST /logout
def logout
  session_destroy()
  redirect("/")
end
```

```soli
# config/routes.sl
post("/logout", "auth#logout")
```

```erb
<%- button_to("Sign out", "/logout") %>
```

## Issuing JWT Tokens for API Access

If your app also has an API, you can issue a JWT after OAuth login instead of (or in addition to) a session:

```soli
def github_callback_api
  # ... same OAuth flow as above ...

  user = User.find_or_create_from_github(github_user, email)

  # Never sign with a missing secret: an empty key is one anyone can use.
  secret = getenv("JWT_SECRET").to_s
  throw "JWT_SECRET is not set (32+ random characters)" if secret.length < 32

  token = jwt_sign(
    {"sub": user._key, "username": user.username},
    secret,
    {"expires_in": 86400}
  )

  {"status": 200, "json": {"token": token, "user": user}}
end
```

Clients send it on subsequent requests:

```
Authorization: Bearer eyJhbGciOiJIUzI1NiJ9...
```

## Security Checklist

- **Always use the `state` parameter.** It prevents CSRF attacks where an attacker tricks a user into linking their account to the attacker's GitHub. Generate it with `Crypto.random_hex(32)`, keep it with `session_set`, and compare it with `Crypto.secure_compare` once both sides are known to be present.
- **Regenerate the session after login.** `session_regenerate()` prevents session fixation attacks.
- **Check email verification.** GitHub users can have unverified emails, and the profile's public `email` is never checked. Only trust the address `/user/emails` marks `primary` and `verified`.
- **Use PKCE.** GitHub supports `code_challenge_method=S256`; it binds the code to the browser that asked for it.
- **Log provider errors, don't echo them.** `error_description` comes from the query string — anyone can write it.
- **Use HTTPS in production.** OAuth tokens in transit over HTTP can be intercepted.
- **Keep secrets out of code.** Use environment variables for `GITHUB_CLIENT_SECRET` and `JWT_SECRET`.

## What About Other Providers?

The OAuth 2.0 flow is nearly identical across providers. The only things that change are:

| | GitHub | Google | Discord |
|---|---|---|---|
| **Auth URL** | `github.com/login/oauth/authorize` | `accounts.google.com/o/oauth2/v2/auth` | `discord.com/oauth2/authorize` |
| **Token URL** | `github.com/login/oauth/access_token` | `oauth2.googleapis.com/token` | `discord.com/api/oauth2/token` |
| **User URL** | `api.github.com/user` | `googleapis.com/oauth2/v2/userinfo` | `discord.com/api/users/@me` |
| **Scopes** | `read:user user:email` | `openid email profile` | `identify email` |

The Soli code stays the same — just swap the URLs, scopes, and field names.
