# Implementing Google OAuth in SoliLang

This guide walks you through implementing Google OAuth authentication in your SoliLang application.

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
# app/controllers/auth_controller.sl

def google_login(req)
    let client_id = getenv("GOOGLE_CLIENT_ID")
    let redirect_uri = getenv("GOOGLE_REDIRECT_URI")
    let scope = "openid email profile"
    
    let auth_url = "https://accounts.google.com/o/oauth2/v2/auth?" +
        "client_id=" + client_id +
        "&redirect_uri=" + redirect_uri +
        "&response_type=code" +
        "&scope=" + scope +
        "&access_type=offline"
    
    {
        "status": 302,
        "headers": {
            "Location": auth_url
        }
    }
end

def google_callback(req)
    let code = req["query_params"]["code"]
    
    if code == null
        return {"status": 400, "body": "Missing authorization code"}
    end
    
    # Exchange code for tokens
    let token_response = google_exchange_code(code)
    
    if token_response["error"] != null
        return {"status": 401, "body": "Failed to exchange code: " + token_response["error_description"]}
    end
    
    let access_token = token_response["access_token"]
    
    # Get user info from Google
    let user_info = google_get_user_info(access_token)
    
    # Find or create user in database
    let user = find_or_create_google_user(user_info)
    
    # Create session
    req["session"]["user_id"] = user["id"]
    
    {
        "status": 302,
        "headers": {
            "Location": "/dashboard"
        }
    }
end

def google_exchange_code(code)
    let client_id = getenv("GOOGLE_CLIENT_ID")
    let client_secret = getenv("GOOGLE_CLIENT_SECRET")
    let redirect_uri = getenv("GOOGLE_REDIRECT_URI")
    
    let params = {
        "code": code,
        "client_id": client_id,
        "client_secret": client_secret,
        "redirect_uri": redirect_uri,
        "grant_type": "authorization_code"
    }
    
    let response = http_post(
        "https://oauth2.googleapis.com/token",
        json_stringify(params),
        {
            "Content-Type": "application/json"
        }
    )
    
    json_parse(response["body"])
end

def google_get_user_info(access_token)
    let response = http_get(
        "https://www.googleapis.com/oauth2/v2/userinfo",
        {
            "Authorization": "Bearer " + access_token
        }
    )
    
    json_parse(response["body"])
end
```

## Step 4: Create the User Model

```soli
# app/models/user.sl

class User extends Model
    id: Int
    google_id: String
    email: String
    name: String
    avatar_url: String
    created_at: DateTime
    
    def find_or_create_google_user(google_data)
        let existing = User.find_by(google_id: google_data["id"])
        
        return existing if existing != nil
    
    User.create({
        "google_id": google_data["id"],
        "email": google_data["email"],
        "name": google_data["name"],
        "avatar_url": google_data["picture"]
    })
end
end
```

## Step 5: Configure Routes

```soli
# config/routes.sl

get "/auth/google", "auth#google_login"
get "/auth/google/callback", "auth#google_callback"
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

1. **Verify the state parameter** - Prevent CSRF attacks by generating and validating a state parameter
2. **Validate ID tokens** - If using OpenID Connect, validate the ID token on your server
3. **Handle token errors** - Gracefully handle expired or revoked tokens
4. **Secure sessions** - Use secure, httpOnly cookies for sessions

## Example with State Parameter

```soli
def generate_state()
    let random_bytes = crypto_random_bytes(32)
    hex_encode(random_bytes)
end

def google_login(req)
    let state = generate_state()
    req["session"]["oauth_state"] = state
    
    let auth_url = "https://accounts.google.com/o/oauth2/v2/auth?" +
        "client_id=" + getenv("GOOGLE_CLIENT_ID") +
        "&redirect_uri=" + getenv("GOOGLE_REDIRECT_URI") +
        "&response_type=code" +
        "&scope=openid email profile" +
        "&state=" + state
    
    {"status": 302, "headers": {"Location": auth_url}}
end

def google_callback(req)
    let received_state = req["query_params"]["state"]
    let stored_state = req["session"]["oauth_state"]
    
    if received_state != stored_state
        return {"status": 400, "body": "Invalid state parameter"}
    end
    
    # Continue with authentication...
end
```

## Conclusion

Google OAuth integration provides a secure way to authenticate users. This guide covered the essential steps, but you can extend it with:

- Refresh token handling for long-lived sessions
- Profile synchronization
- Multi-provider OAuth (GitHub, Facebook, etc.)
- Email verification requirements
