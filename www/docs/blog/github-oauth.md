# Adding "Sign in with GitHub" to Your Soli App

Most developer tools ship with GitHub login. It's familiar, fast, and your users already have an account. Here's how to wire it up in Soli — from creating the OAuth app to storing the user in your database.

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
# app/controllers/auth_controller.sl

def github_login(req)
    let state = Crypto.random_hex(32)
    req["session"]["oauth_state"] = state

    let auth_url = "https://github.com/login/oauth/authorize?" +
        "client_id=" + getenv("GITHUB_CLIENT_ID") +
        "&redirect_uri=" + getenv("GITHUB_REDIRECT_URI") +
        "&scope=read:user user:email" +
        "&state=" + state

    {"status": 302, "headers": {"Location": auth_url}}
end

def github_callback(req)
    let params = req["query_params"]

    # Verify state to prevent CSRF
    if params["state"] != req["session"]["oauth_state"]
        return {"status": 403, "body": "Invalid state parameter"}
    end

    # Check for errors (user denied access, etc.)
    if params["error"] != null
        return {"status": 401, "body": "Authorization denied: " + params["error_description"]}
    end

    let code = params["code"]

    # Exchange code for access token
    let token_data = github_exchange_code(code)

    if token_data["error"] != null
        return {"status": 401, "body": "Token exchange failed: " + token_data["error_description"]}
    end

    let access_token = token_data["access_token"]

    # Fetch user profile from GitHub
    let github_user = github_get_user(access_token)

    # If email is private, fetch from the emails endpoint
    if github_user["email"] == null
        let emails = github_get_emails(access_token)
        github_user["email"] = find_primary_email(emails)
    end

    # Find or create user in our database
    let user = find_or_create_github_user(github_user)

    # Create session
    session_regenerate()
    req["session"]["user_id"] = user["id"]

    {"status": 302, "headers": {"Location": "/dashboard"}}
end
```

## Step 4: GitHub API Helpers

```soli
# app/controllers/auth_controller.sl (continued)

def github_exchange_code(code)
    let response = HTTP.post(
        "https://github.com/login/oauth/access_token",
        JSON.stringify({
            "client_id": getenv("GITHUB_CLIENT_ID"),
            "client_secret": getenv("GITHUB_CLIENT_SECRET"),
            "code": code
        }),
        {
            "Content-Type": "application/json",
            "Accept": "application/json"
        }
    )

    JSON.parse(response["body"])
end

def github_get_user(access_token)
    let response = HTTP.get(
        "https://api.github.com/user",
        {
            "Authorization": "Bearer " + access_token,
            "Accept": "application/json"
        }
    )

    JSON.parse(response["body"])
end

def github_get_emails(access_token)
    let response = HTTP.get(
        "https://api.github.com/user/emails",
        {
            "Authorization": "Bearer " + access_token,
            "Accept": "application/json"
        }
    )

    JSON.parse(response["body"])
end

def find_primary_email(emails)
    for email in emails
        if email["primary"] == true && email["verified"] == true
            return email["email"]
        end
    end
    null
end
```

Note the `Accept: application/json` header on the token exchange — without it, GitHub returns the response as a URL-encoded string instead of JSON.

## Step 5: The User Model

```soli
# app/models/user.sl

class User extends Model
    id: Int
    github_id: Int
    username: String
    email: String
    avatar_url: String
    created_at: DateTime

    def find_or_create_github_user(github_data)
        let existing = User.find_by_github_id(github_data["id"])

        return existing if existing != nil

        User.create({
            "github_id": github_data["id"],
            "username": github_data["login"],
            "email": github_data["email"],
            "avatar_url": github_data["avatar_url"]
        })
    end
end
```

## Step 6: Routes

```soli
# config/routes.sl

get "/auth/github", "auth#github_login"
get "/auth/github/callback", "auth#github_callback"
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

def call(req)
    if req["session"]["user_id"] == null
        return {"status": 302, "headers": {"Location": "/auth/github"}}
    end
end
```

```soli
# config/routes.sl

scope "/dashboard", middleware: ["require_login"] do
    get "/", "dashboard#index"
    get "/settings", "dashboard#settings"
end
```

## Adding a Logout

```soli
# app/controllers/auth_controller.sl

def logout(req)
    session_destroy()
    {"status": 302, "headers": {"Location": "/"}}
end
```

```soli
# config/routes.sl
get "/logout", "auth#logout"
```

## Issuing JWT Tokens for API Access

If your app also has an API, you can issue a JWT after OAuth login instead of (or in addition to) a session:

```soli
def github_callback_api(req)
    # ... same OAuth flow as above ...

    let user = find_or_create_github_user(github_user)

    let token = jwt_sign(
        {"sub": user["id"], "username": user["username"]},
        getenv("JWT_SECRET"),
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

- **Always use the `state` parameter.** It prevents CSRF attacks where an attacker tricks a user into linking their account to the attacker's GitHub. Generate it with `Crypto.random_hex(32)` and validate it in the callback.
- **Regenerate the session after login.** `session_regenerate()` prevents session fixation attacks.
- **Check email verification.** GitHub users can have unverified emails. Only trust `verified: true` emails.
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
