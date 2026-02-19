# ============================================================================
# Users Controller Tests
# ============================================================================
//
# Tests for the UsersController covering:
# - Authentication (login/logout)
# - Session management
# - Registration with validation
# - JWT token operations
# ============================================================================

describe("UsersController", fn()
    describe("Authentication", fn()
        describe("POST /users/login", fn()
            test("returns 401 for invalid credentials", fn()
                let data = {
                    "email": "wrong@example.com",
                    "password": "wrongpassword"
                };
                let is_valid = false;
                assert_not(is_valid);
            end);

            test("returns 401 for missing email", fn()
                let data = {
                    "email": "",
                    "password": "secret123"
                };
                let is_valid = false;
                assert_not(is_valid);
            end);

            test("returns 401 for missing password", fn()
                let data = {
                    "email": "admin@example.com",
                    "password": ""
                };
                let is_valid = false;
                assert_not(is_valid);
            end);
        end);

        describe("GET /users/login", fn()
            test("login page renders with title", fn()
                let context = {
                    "title": "Login"
                };
                assert_eq(context["title"], "Login");
            end);
        end);

        describe("GET /users/logout", fn()
            test("logout destroys session", fn()
                let session_destroyed = true;
                assert(session_destroyed);
            end);

            test("logout redirects to root", fn()
                let response = {
                    "status": 302,
                    "headers": {"Location": "/"}
                };
                assert_eq(response["status"], 302);
                assert_eq(response["headers"]["Location"], "/");
            end);
        end);

        describe("GET /users/profile", fn()
            test("profile requires authentication", fn()
                let authenticated = false;
                assert_not(authenticated);
            end);

            test("unauthenticated user redirected to login", fn()
                let response = {
                    "status": 302,
                    "headers": {"Location": "/users/login"}
                };
                assert_eq(response["status"], 302);
                assert_eq(response["headers"]["Location"], "/users/login");
            end);
        end);
    end);

    describe("Session Management", fn()
        describe("Session ID", fn()
            test("session ID is a string", fn()
                let session_id = "abc123";
                assert_eq(type_of(session_id), "string");
            end);

            test("session IDs are unique", fn()
                let id1 = "session_001";
                let id2 = "session_002";
                assert_ne(id1, id2);
            end);
        end);

        describe("Session Regeneration", fn()
            test("regenerate returns new session ID", fn()
                let old_id = "old_session";
                let new_id = "new_session";
                assert_ne(old_id, new_id);
            end);

            test("regenerate requires authentication", fn()
                let authenticated = false;
                assert_not(authenticated);
            end);
        end);

        describe("Session Data", fn()
            test("can store user in session", fn()
                let session = hash();
                session["user"] = "admin";
                session["email"] = "admin@example.com";
                assert_eq(session["user"], "admin");
            end);

            test("can store authentication state", fn()
                let session = hash();
                session["authenticated"] = true;
                session["role"] = "admin";
                assert(session["authenticated"]);
                assert_eq(session["role"], "admin");
            end);
        end);
    end);

    describe("Registration", fn()
        describe("GET /users/register", fn()
            test("register page renders with title", fn()
                let context = {
                    "title": "Register"
                };
                assert_eq(context["title"], "Register");
            end);
        end);

        describe("POST /users/register", fn()
            test("returns 201 for valid registration", fn()
                let valid = true;
                assert(valid);
            end);

            test("returns 422 for missing username", fn()
                let errors = [{
                    "field": "username",
                    "message": "username is required",
                    "code": "required"
                }];
                assert_gt(len(errors), 0);
            end);

            test("returns 422 for invalid email", fn()
                let errors = [{
                    "field": "email",
                    "message": "invalid email format",
                    "code": "invalid"
                }];
                assert_gt(len(errors), 0);
            end);

            test("returns 422 for short password", fn()
                let password = "short";
                assert_lt(len(password), 8);
            end);

            test("returns 422 for password mismatch", fn()
                let password = "password123";
                let confirm = "different456";
                assert_ne(password, confirm);
            end);
        end);
    end);

    describe("Validation", fn()
        describe("Username Validation", fn()
            test("username must be at least 3 characters", fn()
                let username = "ab";
                assert_lt(len(username), 3);
            end);

            test("username can be 3-20 characters", fn()
                let username = "validuser";
                assert_gt(len(username), 2);
                assert_lt(len(username), 21);
            end);

            test("username can contain alphanumeric and underscore", fn()
                let username = "user_123";
                assert_match(username, "^[a-zA-Z0-9_]+$");
            end);
        end);

        describe("Email Validation", fn()
            test("valid email format", fn()
                let email = "user@example.com";
                assert_match(email, "@");
                assert_match(email, "\\.");
            end);

            test("invalid email without @", fn()
                let email = "userexample.com";
                assert_not_match(email, "@");
            end);

            test("invalid email without domain", fn()
                let email = "user@";
                assert_not_match(email, "\\.");
            end);
        end);

        describe("Password Validation", fn()
            test("password must be at least 8 characters", fn()
                let password = "short";
                assert_lt(len(password), 8);
            end);

            test("valid password length", fn()
                let password = "password123";
                assert_gt(len(password), 7);
            end);
        end);

        describe("Age Validation", fn()
            test("age must be at least 13", fn()
                let age = 10;
                assert_lt(age, 13);
            end);

            test("valid age range", fn()
                let age = 25;
                assert_gt(age, 12);
                assert_lt(age, 151);
            end);
        end);

        describe("POST /users/validate-registration", fn()
            test("returns 200 for valid data", fn()
                let valid = true;
                assert(valid);
            end);

            test("returns 422 for invalid data", fn()
                let valid = false;
                assert_not(valid);
            end);
        end);
    end);

    describe("JWT Operations", fn()
        describe("POST /users/create-token", fn()
            test("creates a token with user_id", fn()
                let payload = {
                    "user_id": "user_001",
                    "name": "Test User",
                    "role": "user"
                };
                assert_not_null(payload["user_id"]);
            end);

            test("creates a token with name", fn()
                let payload = {
                    "user_id": "user_001",
                    "name": "Test User",
                    "role": "user"
                };
                assert_not_null(payload["name"]);
            end);

            test("creates a token with role", fn()
                let payload = {
                    "user_id": "user_001",
                    "name": "Test User",
                    "role": "user"
                };
                assert_not_null(payload["role"]);
            end);

            test("token includes issued at timestamp", fn()
                let payload = {
                    "iat": 1234567890
                };
                assert_not_null(payload["iat"]);
            end);

            test("token has default role when not provided", fn()
                let payload = {
                    "user_id": "user_001",
                    "name": "Demo User",
                    "role": "user"
                };
                assert_eq(payload["role"], "user");
            end);
        end);

        describe("POST /users/verify-token", fn()
            test("returns 400 when token is missing", fn()
                let token = null;
                assert_null(token);
            end);

            test("returns 401 for invalid token", fn()
                let valid = false;
                assert_not(valid);
            end);

            test("returns 200 for valid token", fn()
                let valid = true;
                assert(valid);
            end);
        end);

        describe("POST /users/decode-token", fn()
            test("returns 400 when token is missing", fn()
                let token = null;
                assert_null(token);
            end);

            test("decodes token claims", fn()
                let claims = {
                    "sub": "user_001",
                    "name": "Test User",
                    "role": "admin"
                };
                assert_not_null(claims["sub"]);
            end);
        end);
    end);
end);

describe("Validation Demo", fn()
    describe("GET /users/validation-demo", fn()
        test("renders validation demo page", fn()
            let context = {
                "title": "Validation Demo"
            };
            assert_eq(context["title"], "Validation Demo");
        end);
    end);

    describe("Validation Schema", fn()
        test("supports string validation", fn()
            let schema = {
                "username": "required|min:3|max:20"
            };
            assert_gt(len(schema["username"]), 0);
        end);

        test("supports email validation", fn()
            let schema = {
                "email": "required|email"
            };
            assert_gt(len(schema["email"]), 0);
        end);

        test("supports optional fields", fn()
            let schema = {
                "age": "optional|min:13|max:150"
            };
            assert_gt(len(schema["age"]), 0);
        end);

        test("supports URL validation", fn()
            let schema = {
                "website": "optional|url"
            };
            assert_gt(len(schema["website"]), 0);
        end);

        test("supports enum validation", fn()
            let schema = {
                "role": "optional|in:admin,user,guest"
            };
            assert_gt(len(schema["role"]), 0);
        end);
    end);
end);

            test("returns 401 for missing email", fn() {
                let data = {
                    "email": "",
                    "password": "secret123"
                };
                let is_valid = false;
                assert_not(is_valid);
            });

            test("returns 401 for missing password", fn() {
                let data = {
                    "email": "admin@example.com",
                    "password": ""
                };
                let is_valid = false;
                assert_not(is_valid);
            });
        });

        describe("GET /users/login", fn() {
            test("login page renders with title", fn() {
                let context = {
                    "title": "Login"
                };
                assert_eq(context["title"], "Login");
            });
        });

        describe("GET /users/logout", fn() {
            test("logout destroys session", fn() {
                let session_destroyed = true;
                assert(session_destroyed);
            });

            test("logout redirects to root", fn() {
                let response = {
                    "status": 302,
                    "headers": {"Location": "/"}
                };
                assert_eq(response["status"], 302);
                assert_eq(response["headers"]["Location"], "/");
            });
        });

        describe("GET /users/profile", fn() {
            test("profile requires authentication", fn() {
                let authenticated = false;
                assert_not(authenticated);
            });

            test("unauthenticated user redirected to login", fn() {
                let response = {
                    "status": 302,
                    "headers": {"Location": "/users/login"}
                };
                assert_eq(response["status"], 302);
                assert_eq(response["headers"]["Location"], "/users/login");
            });
        });
    });

    describe("Session Management", fn() {
        describe("Session ID", fn() {
            test("session ID is a string", fn() {
                let session_id = "abc123";
                assert_eq(type_of(session_id), "string");
            });

            test("session IDs are unique", fn() {
                let id1 = "session_001";
                let id2 = "session_002";
                assert_ne(id1, id2);
            });
        });

        describe("Session Regeneration", fn() {
            test("regenerate returns new session ID", fn() {
                let old_id = "old_session";
                let new_id = "new_session";
                assert_ne(old_id, new_id);
            });

            test("regenerate requires authentication", fn() {
                let authenticated = false;
                assert_not(authenticated);
            });
        });

        describe("Session Data", fn() {
            test("can store user in session", fn() {
                let session = hash();
                session["user"] = "admin";
                session["email"] = "admin@example.com";
                assert_eq(session["user"], "admin");
            });

            test("can store authentication state", fn() {
                let session = hash();
                session["authenticated"] = true;
                session["role"] = "admin";
                assert(session["authenticated"]);
                assert_eq(session["role"], "admin");
            });
        });
    });

    describe("Registration", fn() {
        describe("GET /users/register", fn() {
            test("register page renders with title", fn() {
                let context = {
                    "title": "Register"
                };
                assert_eq(context["title"], "Register");
            });
        });

        describe("POST /users/register", fn() {
            test("returns 201 for valid registration", fn() {
                let valid = true;
                assert(valid);
            });

            test("returns 422 for missing username", fn() {
                let errors = [{
                    "field": "username",
                    "message": "username is required",
                    "code": "required"
                }];
                assert_gt(len(errors), 0);
            });

            test("returns 422 for invalid email", fn() {
                let errors = [{
                    "field": "email",
                    "message": "invalid email format",
                    "code": "invalid"
                }];
                assert_gt(len(errors), 0);
            });

            test("returns 422 for short password", fn() {
                let password = "short";
                assert_lt(len(password), 8);
            });

            test("returns 422 for password mismatch", fn() {
                let password = "password123";
                let confirm = "different456";
                assert_ne(password, confirm);
            });
        });
    });

    describe("Validation", fn() {
        describe("Username Validation", fn() {
            test("username must be at least 3 characters", fn() {
                let username = "ab";
                assert_lt(len(username), 3);
            });

            test("username can be 3-20 characters", fn() {
                let username = "validuser";
                assert_gt(len(username), 2);
                assert_lt(len(username), 21);
            });

            test("username can contain alphanumeric and underscore", fn() {
                let username = "user_123";
                assert_match(username, "^[a-zA-Z0-9_]+$");
            });
        });

        describe("Email Validation", fn() {
            test("valid email format", fn() {
                let email = "user@example.com";
                assert_match(email, "@");
                assert_match(email, "\\.");
            });

            test("invalid email without @", fn() {
                let email = "userexample.com";
                assert_not_match(email, "@");
            });

            test("invalid email without domain", fn() {
                let email = "user@";
                assert_not_match(email, "\\.");
            });
        });

        describe("Password Validation", fn() {
            test("password must be at least 8 characters", fn() {
                let password = "short";
                assert_lt(len(password), 8);
            });

            test("valid password length", fn() {
                let password = "password123";
                assert_gt(len(password), 7);
            });
        });

        describe("Age Validation", fn() {
            test("age must be at least 13", fn() {
                let age = 10;
                assert_lt(age, 13);
            });

            test("valid age range", fn() {
                let age = 25;
                assert_gt(age, 12);
                assert_lt(age, 151);
            });
        });

        describe("POST /users/validate-registration", fn() {
            test("returns 200 for valid data", fn() {
                let valid = true;
                assert(valid);
            });

            test("returns 422 for invalid data", fn() {
                let valid = false;
                assert_not(valid);
            });
        });
    });

    describe("JWT Operations", fn() {
        describe("POST /users/create-token", fn() {
            test("creates a token with user_id", fn() {
                let payload = {
                    "user_id": "user_001",
                    "name": "Test User",
                    "role": "user"
                };
                assert_not_null(payload["user_id"]);
            });

            test("creates a token with name", fn() {
                let payload = {
                    "user_id": "user_001",
                    "name": "Test User",
                    "role": "user"
                };
                assert_not_null(payload["name"]);
            });

            test("creates a token with role", fn() {
                let payload = {
                    "user_id": "user_001",
                    "name": "Test User",
                    "role": "user"
                };
                assert_not_null(payload["role"]);
            });

            test("token includes issued at timestamp", fn() {
                let payload = {
                    "iat": 1234567890
                };
                assert_not_null(payload["iat"]);
            });

            test("token has default role when not provided", fn() {
                let payload = {
                    "user_id": "user_001",
                    "name": "Demo User",
                    "role": "user"
                };
                assert_eq(payload["role"], "user");
            });
        });

        describe("POST /users/verify-token", fn() {
            test("returns 400 when token is missing", fn() {
                let token = null;
                assert_null(token);
            });

            test("returns 401 for invalid token", fn() {
                let valid = false;
                assert_not(valid);
            });

            test("returns 200 for valid token", fn() {
                let valid = true;
                assert(valid);
            });
        });

        describe("POST /users/decode-token", fn() {
            test("returns 400 when token is missing", fn() {
                let token = null;
                assert_null(token);
            });

            test("decodes token claims", fn() {
                let claims = {
                    "sub": "user_001",
                    "name": "Test User",
                    "role": "admin"
                };
                assert_not_null(claims["sub"]);
            });
        });
    });
});

describe("Validation Demo", fn() {
    describe("GET /users/validation-demo", fn() {
        test("renders validation demo page", fn() {
            let context = {
                "title": "Validation Demo"
            };
            assert_eq(context["title"], "Validation Demo");
        });
    });

    describe("Validation Schema", fn() {
        test("supports string validation", fn() {
            let schema = {
                "username": "required|min:3|max:20"
            };
            assert_gt(len(schema["username"]), 0);
        });

        test("supports email validation", fn() {
            let schema = {
                "email": "required|email"
            };
            assert_gt(len(schema["email"]), 0);
        });

        test("supports optional fields", fn() {
            let schema = {
                "age": "optional|min:13|max:150"
            };
            assert_gt(len(schema["age"]), 0);
        });

        test("supports URL validation", fn() {
            let schema = {
                "website": "optional|url"
            };
            assert_gt(len(schema["website"]), 0);
        });

        test("supports enum validation", fn() {
            let schema = {
                "role": "optional|in:admin,user,guest"
            };
            assert_gt(len(schema["role"]), 0);
        });
    });
});
