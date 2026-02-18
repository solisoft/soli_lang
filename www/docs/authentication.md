# Authentication with JWT

SoliLang provides built-in JWT (JSON Web Token) functions for stateless authentication.

## Creating Tokens

Use `jwt_sign()` to create tokens for authenticated users:

```soli
fn login(req) {
    let data = req["json"];
    let username = data["username"];
    let password = data["password"];

    // Verify credentials (example)
    if username == "admin" && password == "secret" {
        let payload = {
            "sub": username,
            "role": "admin",
            "name": "Administrator"
        };
        let secret = getenv("JWT_SECRET");
        let token = jwt_sign(payload, secret, {"expires_in": 3600});

        return {
            "status": 200,
            "body": json_stringify({
                "token": token,
                "expires_in": 3600
            })
        };
    }

    {"status": 401, "body": "Invalid credentials"}
}
```

## Verifying Tokens

Use `jwt_verify()` to validate tokens and extract claims:

```soli
fn authenticate_middleware(req) {
    let auth_header = req["headers"]["Authorization"];

    if (auth_header == null || !Regex.matches("^Bearer ", auth_header)) {
        return {"status": 401, "body": "Missing or invalid Authorization header"};
    }

    let token = Regex.replace("^Bearer ", auth_header, "");
    let result = jwt_verify(token, getenv("JWT_SECRET"));

    if result["error"] == true {
        return {"status": 401, "body": "Invalid token: " + result["message"]};
    }

    // Token is valid - add user info to request
    req["current_user"] = result;
    null  // Continue to next middleware/controller
}
```

## Decoding Tokens

Use `jwt_decode()` to read token claims without verification:

```soli
fn get_token_info(req) {
    let token = req["headers"]["Authorization"];
    let token = Regex.replace("^Bearer ", token, "");

    // Decode without verification (for reading only)
    let claims = jwt_decode(token);

    {
        "status": 200,
        "body": json_stringify({
            "subject": claims["sub"],
            "issued_at": claims["iat"],
            "expires": claims["exp"]
        })
    }
}
```

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
| `options.expires_in` | Int | Expiration in seconds (optional) |
| `options.algorithm` | String | Algorithm: "HS256", "HS384", "HS512" (optional) |

### jwt_verify

```soli
jwt_verify(token, secret)
```

Returns a hash with claims if valid, or `{"error": true, "message": "..."}` if invalid.

### jwt_decode

```soli
jwt_decode(token)
```

Returns claims without verification. Useful for reading token data without a secret.

## Best Practices

1. **Always use HTTPS** in production
2. **Set appropriate expiration times** for tokens
3. **Store secrets securely** in environment variables
4. **Validate all claims** before trusting token data
5. **Consider refresh tokens** for long-lived sessions
