# Users Controller - Authentication, Sessions, and Validation Demo

# Login form
fn login(req)
    render("users/login.html", {
        "title": "Login"
    })
end

# Login handler with session management
fn login_post(req)
    let data = req["json"]

    # Demo validation - in real app, check database
    let email = data["email"]
    let password = data["password"]

    if (email == "admin" + "@" + "example.com" && password == "secret123")
        # Regenerate session for security (prevents session fixation)
        session_regenerate()

        # Set session values
        session_set("user", "admin")
        session_set("email", email)
        session_set("user_id", "user_001")
        session_set("role", "admin")
        session_set("authenticated", true)

        return {
            "status": 200,
            "body": json_stringify({
                "success": true,
                "user": "admin",
                "message": "Login successful"
            })
        }
    end

    {
        "status": 401,
        "body": json_stringify({
            "success": false,
            "error": "Invalid email or password"
        })
    }
end

# Registration form
fn register(req)
    render("users/register.html", {
        "title": "Register"
    })
end

# Registration handler with input validation
fn register_post(req)
    let data = req["json"]

    # Define validation schema
    let schema = {
        "username": V.string().required()
            .min_length(3)
            .max_length(20)
            .pattern("^[a-zA-Z0-9_]+$"),
        "email": V.string().required().email(),
        "password": V.string().required().min_length(8),
        "confirm_password": V.string().required(),
        "age": V.int().optional().min(13).max(150)
    }

    # Validate input
    let result = validate(data, schema)

    if (!result["valid"])
        return {
            "status": 422,
            "body": json_stringify({
                "success": false,
                "errors": result["errors"]
            })
        }
    end

    let validated = result["data"]

    # Check password confirmation
    if (validated["password"] != validated["confirm_password"])
        return {
            "status": 422,
            "body": json_stringify({
                "success": false,
                "errors": [{
                    "field": "confirm_password",
                    "message": "passwords do not match",
                    "code": "mismatch"
                }]
            })
        }
    end

    # In real app: save to database
    # For demo, just show success
    {
        "status": 201,
        "body": json_stringify({
            "success": true,
            "message": "Account created successfully",
            "data": {
                "username": validated["username"],
                "email": validated["email"],
                "age": validated["age"]
            }
        })
    }
end

# Profile page (requires authentication)
fn profile(req)
    if (session_get("authenticated") != true)
        return {
            "status": 302,
            "headers": {"Location": "/users/login"}
        }
    end

    render("users/profile.html", {
        "title": "Profile"
    })
end

# Logout - destroy session
fn logout(req)
    session_destroy()

    {
        "status": 302,
        "headers": {"Location": "/"}
    }
end

# Regenerate session ID
fn regenerate_session(req)
    if (session_get("authenticated") != true)
        return {
            "status": 302,
            "headers": {"Location": "/users/login"}
        }
    end

    let old_id = session_id()
    let new_id = session_regenerate()

    print("Session regenerated: ", old_id, " -> ", new_id)

    {
        "status": 302,
        "headers": {"Location": "/users/profile"}
    }
end

# Validation demo page
fn validation_demo(req)
    render("users/validation-demo.html", {
        "title": "Validation Demo"
    })
end

# Validation API endpoint
fn validate_registration(req)
    let data = req["json"]

    let schema = {
        "username": V.string().required()
            .min_length(3)
            .max_length(20)
            .pattern("^[a-zA-Z0-9_]+$"),
        "email": V.string().required().email(),
        "password": V.string().required().min_length(8),
        "age": V.int().optional().min(13).max(150),
        "website": V.string().optional().url(),
        "role": V.string().optional().one_of(["admin", "user", "guest"])
    }

    let result = validate(data, schema)

    {
        "status": result["valid"] ? 200 : 422,
        "body": json_stringify(result)
    }
end

# JWT Demo: Create token
fn create_token(req)
    let data = req["json"]

    # In real app: verify user credentials first
    let payload = {
        "sub": data["user_id"],
        "name": data["name"],
        "role": data["role"],
        "iat": clock()
    }

    # Handle null values with defaults
    if (payload["sub"] == null)  payload["sub"] = "user_001" end
    if (payload["name"] == null)  payload["name"] = "Demo User" end
    if (payload["role"] == null)  payload["role"] = "user" end

    # Sign JWT with secret (in real app, use environment variable)
    let secret = "demo-secret-key-change-in-production"
    let options = {}
    if (data["expires_in"])
        options["expires_in"] = data["expires_in"]
    end

    let token = jwt_sign(payload, secret, options)

    let expires = 3600
    if (data["expires_in"] != null)
        expires = data["expires_in"]
    end

    {
        "status": 200,
        "body": json_stringify({
            "token": token,
            "type": "Bearer",
            "expires_in": expires
        })
    }
end

# JWT Demo: Verify token
fn verify_token(req)
    let data = req["json"]
    let token = data["token"]

    if (!token)
        return {
            "status": 400,
            "body": json_stringify({
                "error": "token is required"
            })
        }
    end

    let secret = "demo-secret-key-change-in-production"
    let result = jwt_verify(token, secret)

    if (result["error"] == true)
        return {
            "status": 401,
            "body": json_stringify({
                "valid": false,
                "error": result["message"]
            })
        }
    end

    {
        "status": 200,
        "body": json_stringify({
            "valid": true,
            "claims": result
        })
    }
end

# JWT Demo: Decode token (without verification)
fn decode_token(req)
    let data = req["json"]
    let token = data["token"]

    if (!token)
        return {
            "status": 400,
            "body": json_stringify({
                "error": "token is required"
            })
        }
    end

    let claims = jwt_decode(token)

    {
        "status": 200,
        "body": json_stringify({
            "claims": claims
        })
    }
end
