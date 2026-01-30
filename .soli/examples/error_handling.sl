// ============================================================================
// Error Handling Examples for AI/LLM Code Generation
// ============================================================================
//
// AI AGENT GUIDE:
// ---------------
// This file documents error handling patterns in Soli.
// Use try/catch/throw for robust error handling.
//
// ERROR TYPES:
// - Error - Base error type
// - ValueError - Invalid value or input
// - TypeError - Type mismatch
// - KeyError - Missing key in hash/object
// - IndexError - Index out of bounds
// - RuntimeError - Runtime errors
//
// SYNTAX:
// try { /* code */ } catch (error) { /* handle */ }
// try { /* code */ } catch (error) { /* handle */ } finally { /* cleanup */ }
// throw "error message"
// throw Error.new("message", { code: 500, details: data })
//
// ============================================================================

// ============================================================================
// EXAMPLE 1: Basic try/catch
// ============================================================================

class ErrorHandlingController extends Controller {
    fn basic_example(req: Any) -> Any {
        let input = req["json"];

        try {
            let result = risky_operation(input);
            return { "status": 200, "body": json_stringify(result) };
        } catch (e) {
            return {
                "status": 500,
                "body": json_stringify({ "error": e["message"] })
            };
        }
    }

    fn with_finally(req: Any) -> Any {
        let resource = null;

        try {
            resource = acquire_resource();
            let result = process(resource);
            return { "status": 200, "body": json_stringify(result) };
        } catch (e) {
            return {
                "status": 500,
                "body": json_stringify({ "error": e["message"] })
            };
        } finally {
            if (resource != null) {
                release_resource(resource);
            }
        }
    }
}

// ============================================================================
// EXAMPLE 2: Specific Error Type Catching
// ============================================================================

class ValidationController extends Controller {
    fn validate_user(req: Any) -> Any {
        let data = req["json"];

        try {
            let user = validate_user_data(data);
            return { "status": 200, "body": json_stringify(user) };
        } catch (e: ValueError) {
            return {
                "status": 400,
                "body": json_stringify({
                    "error": "Validation failed",
                    "message": e["message"],
                    "field": e["field"]
                })
            };
        } catch (e: TypeError) {
            return {
                "status": 400,
                "body": json_stringify({
                    "error": "Type error",
                    "message": e["message"]
                })
            };
        } catch (e) {
            return {
                "status": 500,
                "body": json_stringify({
                    "error": "Internal error",
                    "message": "An unexpected error occurred"
                })
            };
        }
    }

    fn access_nested_data(req: Any) -> Any {
        let data = req["json"];

        try {
            let city = data["user"]["address"]["city"];
            return { "status": 200, "body": json_stringify({ "city": city }) };
        } catch (e: KeyError) {
            return {
                "status": 404,
                "body": json_stringify({
                    "error": "Missing data",
                    "message": e["message"],
                    "missing_key": e["key"]
                })
            };
        }
    }

    fn access_array_element(req: Any) -> Any {
        let items = req["json"]["items"];
        let index = req["params"]["index"];

        try {
            let item = items[index];
            return { "status": 200, "body": json_stringify({ "item": item }) };
        } catch (e: IndexError) {
            return {
                "status": 404,
                "body": json_stringify({
                    "error": "Index out of bounds",
                    "message": e["message"],
                    "index": index,
                    "size": len(items)
                })
            };
        }
    }
}

// ============================================================================
// EXAMPLE 3: Throwing Custom Errors
// ============================================================================

class CustomErrorController extends Controller {
    fn create_resource(req: Any) -> Any {
        let data = req["json"];

        try {
            validate_required(data, ["name", "email"]);
            validate_email(data["email"]);
            validate_password(data["password"]);

            let user = create_user(data);
            return { "status": 201, "body": json_stringify(user) };
        } catch (e: ValueError) {
            return {
                "status": 422,
                "body": json_stringify({
                    "error": "Validation failed",
                    "messages": e["messages"]
                })
            };
        }
    }

    fn throw_basic(req: Any) -> Any {
        let id = req["params"]["id"];

        if (id == null) {
            throw "ID is required";
        }

        return { "status": 200 };
    }

    fn throw_with_details(req: Any) -> Any {
        let id = req["params"]["id"];

        if (id == null) {
            throw Error.new("ID is required", {
                "code": "MISSING_ID",
                "status": 400
            });
        }

        return { "status": 200 };
    }

    fn throw_specific_types(req: Any) -> Any {
        let data = req["json"];

        if (!has_key(data, "name")) {
            throw ValueError.new("Name is required", {
                "field": "name",
                "expected": "string"
            });
        }

        if (typeof(data["name"]) != "string") {
            throw TypeError.new("Name must be a string", {
                "field": "name",
                "actual": typeof(data["name"])
            });
        }

        return { "status": 200 };
    }
}

// ============================================================================
// EXAMPLE 4: Database Error Handling
// ============================================================================

class DatabaseController extends Controller {
    static {
        this.db = solidb_connect("localhost", 6745, "api-key");
        this.database = "myapp";
    }

    fn handle_database_errors(req: Any) -> Any {
        let id = req["params"]["id"];

        try {
            let result = solidb_get(this.db, this.database, "users", id);

            if (result == null) {
                throw KeyError.new("User not found", {
                    "collection": "users",
                    "key": id
                });
            }

            return { "status": 200, "body": json_stringify(result) };
        } catch (e: KeyError) {
            return {
                "status": 404,
                "body": json_stringify({
                    "error": "Not found",
                    "message": e["message"]
                })
            };
        } catch (e) {
            return {
                "status": 500,
                "body": json_stringify({
                    "error": "Database error",
                    "message": e["message"]
                })
            };
        }
    }

    fn handle_transaction(req: Any) -> Any {
        let data = req["json"];

        try {
            let result = solidb_transaction(this.db, this.database, fn(tx) {
                let user = solidb_get(tx, this.database, "users", data["user_id"]);

                if (user == null) {
                    throw KeyError.new("User not found", { "user_id": data["user_id"] });
                }

                if (user["balance"] < data["amount"]) {
                    throw RuntimeError.new("Insufficient balance", {
                        "current": user["balance"],
                        "requested": data["amount"]
                    });
                }

                solidb_update(tx, this.database, "users", data["user_id"], {
                    "balance": user["balance"] - data["amount"]
                });

                solidb_insert(tx, this.database, "transactions", {
                    "user_id": data["user_id"],
                    "amount": data["amount"],
                    "type": "withdrawal",
                    "created_at": DateTime.now()
                });

                return { "success": true, "new_balance": user["balance"] - data["amount"] };
            });

            return { "status": 200, "body": json_stringify(result) };
        } catch (e: KeyError) {
            return {
                "status": 404,
                "body": json_stringify({
                    "error": "User not found",
                    "message": e["message"]
                })
            };
        } catch (e: RuntimeError) {
            return {
                "status": 400,
                "body": json_stringify({
                    "error": "Transaction failed",
                    "message": e["message"],
                    "details": e["details"]
                })
            };
        } catch (e) {
            return {
                "status": 500,
                "body": json_stringify({
                    "error": "Transaction error",
                    "message": e["message"]
                })
            };
        }
    }
}

// ============================================================================
// EXAMPLE 5: Validation Helper Functions
// ============================================================================

class Validators {
    fn validate_required(data: Hash, fields: Array) -> Any {
        let errors = [];

        for field in fields {
            if (!has_key(data, field) || data[field] == null || data[field] == "") {
                errors.push(ValueError.new(
                    "Field '" + field + "' is required",
                    { "field": field }
                ));
            }
        }

        if (len(errors) > 0) {
            throw ValueError.new("Validation failed", { "errors": errors });
        }
    }

    fn validate_email(email: String) -> Any {
        if (email == null || !contains(email, "@")) {
            throw ValueError.new("Invalid email format", { "field": "email" });
        }
    }

    fn validate_password(password: String) -> Any {
        if (len(password) < 8) {
            throw ValueError.new("Password must be at least 8 characters", {
                "field": "password"
            });
        }

        if (!contains_any(password, ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"])) {
            throw ValueError.new("Password must contain a number", { "field": "password" });
        }
    }

    fn validate_range(value: Int, min: Int, max: Int, field: String) -> Any {
        if (value < min || value > max) {
            throw ValueError.new(
                field + " must be between " + str(min) + " and " + str(max),
                { "field": field, "min": min, "max": max, "actual": value }
            );
        }
    }

    fn validate_in_list(value: Any, allowed: Array, field: String) -> Any {
        let found = false;
        for item in allowed {
            if (item == value) {
                found = true;
                break;
            }
        }

        if (!found) {
            throw ValueError.new(
                field + " must be one of: " + join(allowed, ", "),
                { "field": field, "allowed": allowed, "actual": value }
            );
        }
    }
}

fn create_user(data: Hash) -> Any {
    let validator = Validators.new();

    validator.validate_required(data, ["name", "email", "password"]);
    validator.validate_email(data["email"]);
    validator.validate_password(data["password"]);

    let db = solidb_connect("localhost", 6745, "api-key");
    let result = solidb_insert(db, "myapp", "users", {
        "name": data["name"],
        "email": data["email"],
        "password": hash_password(data["password"]),
        "created_at": DateTime.now()
    });

    return result;
}

// ============================================================================
// EXAMPLE 6: Nested try/catch Blocks
// ============================================================================

class NestedErrorController extends Controller {
    fn complex_operation(req: Any) -> Any {
        let data = req["json"];

        try {
            let processed = process_data(data);
            let saved = save_to_database(processed);
            let cached = cache_result(saved);
            return { "status": 200, "body": json_stringify({ "result": saved }) };
        } catch (e: ValueError) {
            print("[VALIDATION ERROR]", e["message"]);
            return {
                "status": 400,
                "body": json_stringify({ "error": "Invalid input", "message": e["message"] })
            };
        } catch (e: KeyError) {
            print("[NOT FOUND ERROR]", e["message"]);
            return {
                "status": 404,
                "body": json_stringify({ "error": "Resource not found", "message": e["message"] })
            };
        } catch (e: RuntimeError) {
            print("[RUNTIME ERROR]", e["message"]);
            return {
                "status": 500,
                "body": json_stringify({ "error": "Operation failed", "message": e["message"] })
            };
        }
    }

    fn process_data(data: Hash) -> Any {
        try {
            let cleaned = sanitize(data);
            let transformed = transform(cleaned);
            return transformed;
        } catch (e) {
            throw ValueError.new("Failed to process data", { "original": e["message"] });
        }
    }

    fn sanitize(data: Hash) -> Any {
        for key in keys(data) {
            if (typeof(data[key]) == "string") {
                data[key] = trim(data[key]);
            }
        }
        return data;
    }

    fn transform(data: Hash) -> Any {
        let result = {};
        for key in keys(data) {
            result[key] = normalize(data[key]);
        }
        return result;
    }
}

// ============================================================================
// EXAMPLE 7: Error Handling in Loops
// ============================================================================

class BatchController extends Controller {
    fn process_batch(req: Any) -> Any {
        let items = req["json"]["items"];
        let results = [];
        let errors = [];

        for item in items {
            try {
                let result = process_item(item);
                results.push({ "item": item, "success": true, "result": result });
            } catch (e) {
                errors.push({
                    "item": item,
                    "error": e["message"],
                    "type": typeof(e)
                });
            }
        }

        return {
            "status": 200,
            "body": json_stringify({
                "processed": len(results),
                "failed": len(errors),
                "results": results,
                "errors": errors
            })
        };
    }

    fn process_item(item: Hash) -> Any {
        if (!has_key(item, "id")) {
            throw ValueError.new("Item missing id", { "item": item });
        }

        validate_item(item);

        return { "id": item["id"], "processed": true };
    }

    fn validate_item(item: Hash) -> Any {
        if (has_key(item, "status")) {
            let allowed = ["pending", "processing", "completed"];
            let found = false;
            for s in allowed {
                if (s == item["status"]) {
                    found = true;
                    break;
                }
            }
            if (!found) {
                throw ValueError.new("Invalid status", { "status": item["status"] });
            }
        }
    }
}

// ============================================================================
// EXAMPLE 8: Custom Error Classes
// ============================================================================

class ApiError extends Error {
    fn new(message: String, details: Hash) -> Any {
        let error = Error.new(message, details);
        error["type"] = "ApiError";
        error["code"] = details["code"] ?? "API_ERROR";
        error["status"] = details["status"] ?? 500;
        return error;
    }
}

class AuthenticationError extends Error {
    fn new(message: String) -> Any {
        let error = Error.new(message, {});
        error["type"] = "AuthenticationError";
        error["code"] = "AUTH_ERROR";
        error["status"] = 401;
        return error;
    }
}

class AuthorizationError extends Error {
    fn new(message: String) -> Any {
        let error = Error.new(message, {});
        error["type"] = "AuthorizationError";
        error["code"] = "FORBIDDEN";
        error["status"] = 403;
        return error;
    }
}

class SecureController extends Controller {
    fn protected_action(req: Any) -> Any {
        let user = req["user"];

        try {
            authenticate_user(user);
            authorize_action(user, "protected_action");
            return { "status": 200, "body": json_stringify({ "data": "secret" }) };
        } catch (e: AuthenticationError) {
            return {
                "status": 401,
                "body": json_stringify({ "error": "Authentication required" })
            };
        } catch (e: AuthorizationError) {
            return {
                "status": 403,
                "body": json_stringify({ "error": "Access denied" })
            };
        } catch (e) {
            return {
                "status": 500,
                "body": json_stringify({ "error": "Internal error" })
            };
        }
    }

    fn authenticate_user(user: Any) -> Any {
        if (user == null || !has_key(user, "id")) {
            throw AuthenticationError.new("Invalid session");
        }
    }

    fn authorize_action(user: Any, action: String) -> Any {
        let permissions = user["permissions"] ?? [];
        let allowed = false;
        for p in permissions {
            if (p == action || p == "admin") {
                allowed = true;
                break;
            }
        }
        if (!allowed) {
            throw AuthorizationError.new("User cannot perform " + action);
        }
    }
}

// ============================================================================
// EXAMPLE 9: Global Error Handler Middleware
// ============================================================================

fn error_handler(req: Any) -> Any {
    try {
        return handle_request(req);
    } catch (e: ValueError) {
        return {
            "status": 400,
            "body": json_stringify({
                "error": "Bad Request",
                "message": e["message"]
            })
        };
    } catch (e: KeyError) {
        return {
            "status": 404,
            "body": json_stringify({
                "error": "Not Found",
                "message": e["message"]
            })
        };
    } catch (e: AuthenticationError) {
        return {
            "status": 401,
            "body": json_string.stringify({
                "error": "Unauthorized",
                "message": e["message"]
            })
        };
    } catch (e: AuthorizationError) {
        return {
            "status": 403,
            "body": json_stringify({
                "error": "Forbidden",
                "message": e["message"]
            })
        };
    } catch (e: Error) {
        print("[ERROR]", e["type"], ":", e["message"]);
        return {
            "status": e["status"] ?? 500,
            "body": json_stringify({
                "error": "Internal Server Error",
                "message": "An unexpected error occurred"
            })
        };
    } catch (e) {
        print("[UNKNOWN ERROR]", e);
        return {
            "status": 500,
            "body": json_stringify({
                "error": "Internal Server Error",
                "message": "An unknown error occurred"
            })
        };
    }
}

fn handle_request(req: Any) -> Any {
    return {"status": 200};
}

// ============================================================================
// EXAMPLE 10: Error Recovery Patterns
// ============================================================================

class RecoveryController extends Controller {
    fn with_retry(req: Any) -> Any {
        let max_attempts = 3;
        let attempt = 0;
        let last_error = null;

        while (attempt < max_attempts) {
            attempt = attempt + 1;
            try {
                return unstable_operation(req);
            } catch (e) {
                last_error = e;
                print("[RETRY] Attempt", attempt, "failed:", e["message"]);
                if (attempt < max_attempts) {
                    sleep(1000 * attempt);
                }
            }
        }

        return {
            "status": 500,
            "body": json_stringify({
                "error": "Operation failed after " + str(max_attempts) + " attempts",
                "message": last_error["message"]
            })
        };
    }

    fn with_fallback(req: Any) -> Any {
        let primary = null;
        let fallback = null;

        try {
            primary = call_primary_service(req);
            return primary;
        } catch (e) {
            print("[FALLBACK] Primary failed, trying fallback");
            try {
                fallback = call_fallback_service(req);
                return fallback;
            } catch (e2) {
                return {
                    "status": 503,
                    "body": json_stringify({
                        "error": "Service unavailable",
                        "message": "All services failed"
                    })
                };
            }
        }
    }

    fn unstable_operation(req: Any) -> Any {
        if (clock() % 3 == 0) {
            throw RuntimeError.new("Random failure");
        }
        return { "success": true };
    }

    fn call_primary_service(req: Any) -> Any {
        return { "source": "primary", "data": "result" };
    }

    fn call_fallback_service(req: Any) -> Any {
        return { "source": "fallback", "data": "cached result" };
    }
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

fn contains_any(str: String, chars: Array) -> Bool {
    for c in chars {
        if (contains(str, c)) {
            return true;
        }
    }
    return false;
}

fn has_key(hash: Hash, key: String) -> Bool {
    return hash != null && typeof(hash) == "hash" && key in hash;
}

fn hash_password(password: String) -> String {
    return sha256(password + "salt");
}

// ============================================================================
