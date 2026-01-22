// Users Controller - Authentication, Sessions, and Validation Demo

// Login form
fn login(req: Any) -> Any {
    return render("users/login.html", {
        "title": "Login"
    });
}

// Login handler with session management
fn login_post(req: Any) -> Any {
    let data = req["json"];

    // Demo validation - in real app, check database
    let email = data["email"];
    let password = data["password"];

    if (email == "admin" + "@" + "example.com" && password == "secret123") {
        // Regenerate session for security (prevents session fixation)
        session_regenerate();

        // Set session values
        session_set("user", "admin");
        session_set("email", email);
        session_set("user_id", "user_001");
        session_set("role", "admin");
        session_set("authenticated", true);

        return {
            "status": 200,
            "body": json_stringify({
                "success": true,
                "user": "admin",
                "message": "Login successful"
            })
        };
    }

    return {
        "status": 401,
        "body": json_stringify({
            "success": false,
            "error": "Invalid email or password"
        })
    };
}

// Registration form
fn register(req: Any) -> Any {
    return render("users/register.html", {
        "title": "Register"
    });
}

// Registration handler with input validation
fn register_post(req: Any) -> Any {
    let data = req["json"];

    // Define validation schema
    let schema = {
        "username": V.string().required()
            .min_length(3)
            .max_length(20)
            .pattern("^[a-zA-Z0-9_]+$"),
        "email": V.string().required().email(),
        "password": V.string().required().min_length(8),
        "confirm_password": V.string().required(),
        "age": V.int().optional().min(13).max(150)
    };

    // Validate input
    let result = validate(data, schema);

    if (!result["valid"]) {
        return {
            "status": 422,
            "body": json_stringify({
                "success": false,
                "errors": result["errors"]
            })
        };
    }

    let validated = result["data"];

    // Check password confirmation
    if (validated["password"] != validated["confirm_password"]) {
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
        };
    }

    // In real app: save to database
    // For demo, just show success
    return {
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
    };
}

// Profile page (requires authentication)
fn profile(req: Any) -> Any {
    if (session_get("authenticated") != true) {
        return {
            "status": 302,
            "headers": {"Location": "/users/login"}
        };
    }

    return render("users/profile.html", {
        "title": "Profile"
    });
}

// Logout - destroy session
fn logout(req: Any) -> Any {
    session_destroy();

    return {
        "status": 302,
        "headers": {"Location": "/"}
    };
}

// Regenerate session ID
fn regenerate_session(req: Any) -> Any {
    if (session_get("authenticated") != true) {
        return {
            "status": 302,
            "headers": {"Location": "/users/login"}
        };
    }

    let old_id = session_id();
    let new_id = session_regenerate();

    print("Session regenerated: ", old_id, " -> ", new_id);

    return {
        "status": 302,
        "headers": {"Location": "/users/profile"}
    };
}

// Validation demo page
fn validation_demo(req: Any) -> Any {
    return render("users/validation-demo.html", {
        "title": "Validation Demo"
    });
}

// Validation API endpoint
fn validate_registration(req: Any) -> Any {
    let data = req["json"];

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
    };

    let result = validate(data, schema);

    return {
        "status": result["valid"] ? 200 : 422,
        "body": json_stringify(result)
    };
}

// JWT Demo: Create token
fn create_token(req: Any) -> Any {
    let data = req["json"];

    // In real app: verify user credentials first
    let payload = {
        "sub": data["user_id"],
        "name": data["name"],
        "role": data["role"],
        "iat": clock()
    };

    // Handle null values with defaults
    if (payload["sub"] == null) { payload["sub"] = "user_001"; }
    if (payload["name"] == null) { payload["name"] = "Demo User"; }
    if (payload["role"] == null) { payload["role"] = "user"; }

    // Sign JWT with secret (in real app, use environment variable)
    let secret = "demo-secret-key-change-in-production";
    let options = {};
    if (data["expires_in"]) {
        options["expires_in"] = data["expires_in"];
    }

    let token = jwt_sign(payload, secret, options);

    let expires = 3600;
    if (data["expires_in"] != null) {
        expires = data["expires_in"];
    }

    return {
        "status": 200,
        "body": json_stringify({
            "token": token,
            "type": "Bearer",
            "expires_in": expires
        })
    };
}

// JWT Demo: Verify token
fn verify_token(req: Any) -> Any {
    let data = req["json"];
    let token = data["token"];

    if (!token) {
        return {
            "status": 400,
            "body": json_stringify({
                "error": "token is required"
            })
        };
    }

    let secret = "demo-secret-key-change-in-production";
    let result = jwt_verify(token, secret);

    if (result["error"] == true) {
        return {
            "status": 401,
            "body": json_stringify({
                "valid": false,
                "error": result["message"]
            })
        };
    }

    return {
        "status": 200,
        "body": json_stringify({
            "valid": true,
            "claims": result
        })
    };
}

// JWT Demo: Decode token (without verification)
fn decode_token(req: Any) -> Any {
    let data = req["json"];
    let token = data["token"];

    if (!token) {
        return {
            "status": 400,
            "body": json_stringify({
                "error": "token is required"
            })
        };
    }

    let claims = jwt_decode(token);

    return {
        "status": 200,
        "body": json_stringify({
            "claims": claims
        })
    };
}
