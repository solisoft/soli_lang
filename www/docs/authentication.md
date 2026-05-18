# Authentication with JWT

SoliLang provides built-in JWT (JSON Web Token) functions for stateless authentication.

## Creating Tokens

Use `jwt_sign()` to create tokens for authenticated users:

```soli
fn login
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
fn authenticate_middleware
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
fn get_token_info
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
| `options.expires_in` | Int | Expiration in seconds (optional) |
| `options.algorithm` | String | Algorithm: "HS256", "HS384", "HS512", "RS256", "EdDSA" (optional) |
| `options.key` | String | PEM-encoded private key for RS256/EdDSA (optional) |

### jwt_verify

```soli
jwt_verify(token, secret)
jwt_verify(token, secret, options)
```

| Parameter | Type | Description |
|-----------|------|-------------|
| `token` | String | The JWT token |
| `secret` | String | Secret key for HMAC, or public key material for RS256/EdDSA |
| `options.key` | String | PEM-encoded public key for RS256/EdDSA (optional) |

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
