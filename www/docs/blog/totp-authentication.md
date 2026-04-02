# Two-Factor Authentication with TOTP in SoliLang

Two-factor authentication (2FA) adds an extra layer of security to your accounts. Instead of just knowing something (password), the user must also have something (their phone). TOTP (Time-based One-Time Password) is the standard used by Google Authenticator, Authy, and most authenticator apps.

## What is TOTP?

TOTP generates a 6-digit code that changes every 30 seconds. It's based on:

- A shared secret key (Base32 encoded)
- The current timestamp
- A cryptographic hash (HMAC-SHA1)

The same code is generated on the server and the authenticator app, so no network connection is needed.

## Generating TOTP Codes

Let's start with generating a TOTP code:

```soli
let secret = "JBSWY3DPEHPK3PXP"
let code = Crypto.totp_generate(secret)

println(code)  # e.g., "123456"
```

The `totp_generate` function takes:
- `secret` (required) - Your Base32-encoded shared secret
- `time` (optional) - Unix timestamp (defaults to current time)
- `period` (optional) - Code validity window in seconds (defaults to 30)

## Verifying TOTP Codes

When a user enters their 2FA code, verify it:

```soli
let secret = "JBSWY3DPEHPK3PXP"
let user_code = request.body["code"]

if Crypto.totp_verify(secret, user_code)
    # Code is valid - user is authenticated
    println("2FA successful!")
else
    # Invalid code
    println("Invalid code, try again")
end
```

The verify function accepts previous, current, and next time windows to handle clock drift between server and phone.

## Creating QR Codes for Easy Setup

The easiest way for users to add 2FA is by scanning a QR code. Generate the URI first:

```soli
let secret = "JBSWY3DPEHPK3PXP"
let uri = Crypto.totp_uri(secret, "user@example.com", "MyApp", 30)

println(uri)
# otpauth://totp/MyApp:user%40example.com?secret=JBSWY3DPEHPK3PXP&issuer=MyApp&algorithm=SHA1&digits=6&period=30
```

Then encode it as a QR code:

```soli
let qr_data = QRCode.encode(uri)
```

Display `qr_data` to the user as an image.

## Complete Example: Adding 2FA to Login

Here's a full example of adding TOTP-based 2FA to a login flow:

```soli
# app/controllers/auth_controller.sl

fn login(req)
    let params = req["all"]
    let email = params["email"]
    let password = params["password"]
    
    let user = User.find_by_email(email)
    
    if user == nil or not Crypto.argon2_verify(password, user["password_hash"])
        return render("auth/login", {"error": "Invalid credentials"})
    end
    
    if user["totp_secret"] != nil
        # User has 2FA enabled - require code
        let temp_token = generate_temp_token(user["id"])
        return render("auth/verify_2fa", {"temp_token": temp_token})
    end
    
    # No 2FA - complete login
    session["user_id"] = user["id"]
    redirect("/dashboard")
end

fn verify_2fa(req)
    let params = req["all"]
    let temp_token = params["temp_token"]
    let code = params["code"]
    
    let user_id = validate_temp_token(temp_token)
    if user_id == nil
        return redirect("/login")
    end
    
    let user = User.find(user_id)
    
    if not Crypto.totp_verify(user["totp_secret"], code)
        return render("auth/verify_2fa", {
            "temp_token": temp_token,
            "error": "Invalid code"
        })
    end
    
    # 2FA verified - complete login
    session["user_id"] = user["id"]
    redirect("/dashboard")
end
```

## Enabling 2FA for a User

To help users set up 2FA, generate their secret and QR code:

```soli
# app/controllers/settings_controller.sl

fn enable_2fa(req)
    let user = User.find(session["user_id"])
    
    # Generate a new random secret
    let keypair = Crypto.x25519_keypair
    let secret = base64_encode(keypair["private"][0..20])  # 20 bytes = 32 Base32 chars
    
    # Save to user (in production, encrypt this!)
    user["totp_secret"] = secret
    user.save
    
    # Generate QR code URI
    let uri = Crypto.totp_uri(secret, user["email"], "MyApp", 30)
    let qr_code = QRCode.encode(uri)
    
    render("settings/2fa_setup", {"qr_code": qr_code, "secret": secret})
end
```

## Security Best Practices

1. **Encrypt secrets at rest** - TOTP secrets are sensitive. Encrypt them in your database.

2. **Rate limit verification** - Prevent brute-force attacks by limiting verification attempts.

3. **Allow backup codes** - Users may lose their phone. Provide backup codes as an alternative.

4. **Use HTTPS** - Always transmit codes over encrypted connections.

## Conclusion

TOTP is a battle-tested 2FA standard that's easy to implement in SoliLang. With just three functions:

- `Crypto.totp_generate()` - Create codes
- `Crypto.totp_verify()` - Validate codes
- `Crypto.totp_uri()` - Generate QR code URIs

You can add professional-grade two-factor authentication to your application in minutes.