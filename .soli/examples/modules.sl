// ============================================================================
// Module System Examples for AI/LLM Code Generation
// ============================================================================
//
// AI AGENT GUIDE:
// ---------------
// This file documents the module system in Soli.
// Use import/export to organize reusable code.
//
// MODULE STRUCTURE:
// - Modules are .sl files in app/modules/
// - Use 'pub' to export functions/classes
// - Use 'import' to include modules
//
// SYNTAX:
// import "module_name";                           // Import entire module
// import "module_name" as alias;                  // Import with alias
// import { function1, function2 } from "module";  // Import specific items
// pub fn public_function() -> Any { }             // Export function
// pub class PublicClass { }                       // Export class
// pub (default) fn default_fn() -> Any { }        // Default export
//
// ============================================================================

// ============================================================================
// EXAMPLE 1: Basic Module Structure
// ============================================================================

// File: app/modules/math.sl
pub fn add(a: Int, b: Int) -> Int {
    return a + b;
}

pub fn subtract(a: Int, b: Int) -> Int {
    return a - b;
}

pub fn multiply(a: Int, b: Int) -> Int {
    return a * b;
}

pub fn divide(a: Int, b: Int) -> Int {
    if (b == 0) {
        throw ValueError.new("Division by zero");
    }
    return a / b;
}

pub fn power(base: Int, exp: Int) -> Int {
    let result = 1;
    let i = 0;
    while (i < exp) {
        result = result * base;
        i = i + 1;
    }
    return result;
}

// Private function (not exported)
fn validate_number(n: Any) -> Bool {
    return typeof(n) == "number";
}

// ============================================================================
// EXAMPLE 2: Utility Module
// ============================================================================

// File: app/modules/string_helpers.sl
pub fn capitalize(str: String) -> String {
    if (len(str) == 0) { return ""; }
    return upcase(substring(str, 0, 1)) + substring(str, 1);
}

pub fn trim(str: String) -> String {
    let start = 0;
    let end = len(str);

    while (start < end && substring(str, start, 1) == " ") {
        start = start + 1;
    }

    while (end > start && substring(str, end - 1, 1) == " ") {
        end = end - 1;
    }

    return substring(str, start, end - start);
}

pub fn split(str: String, delimiter: String) -> Array {
    let result = [];
    let current = "";
    let i = 0;

    while (i < len(str)) {
        let char = substring(str, i, 1);
        if (char == delimiter) {
            if (current != "") {
                result.push(current);
            }
            current = "";
        } else {
            current = current + char;
        }
        i = i + 1;
    }

    if (current != "") {
        result.push(current);
    }

    return result;
}

pub fn join(array: Array, delimiter: String) -> String {
    let result = "";
    let first = true;

    for item in array {
        if (!first) {
            result = result + delimiter;
        }
        result = result + str(item);
        first = false;
    }

    return result;
}

pub fn contains(str: String, substring: String) -> Bool {
    return index_of(str, substring) >= 0;
}

pub fn starts_with(str: String, prefix: String) -> Bool {
    return index_of(str, prefix) == 0;
}

pub fn ends_with(str: String, suffix: String) -> Bool {
    return len(suffix) > 0 && index_of(str, suffix) == len(str) - len(suffix);
}

pub fn replace(str: String, old: String, new: String) -> String {
    let parts = split(str, old);
    return join(parts, new);
}

// ============================================================================
// EXAMPLE 3: Validation Module
// ============================================================================

// File: app/modules/validators.sl
pub fn is_email(value: String) -> Bool {
    if (typeof(value) != "string") { return false; }
    return contains(value, "@") && contains(value, ".");
}

pub fn is_url(value: String) -> Bool {
    if (typeof(value) != "string") { return false; }
    return starts_with(value, "http://") || starts_with(value, "https://");
}

pub fn is_phone(value: String) -> Bool {
    if (typeof(value) != "string") { return false; }
    let digits = replace(value, " ", "");
    digits = replace(digits, "-", "");
    digits = replace(digits, "(", "");
    digits = replace(digits, ")", "");

    if (len(digits) < 10 || len(digits) > 15) { return false; }

    let i = 0;
    while (i < len(digits)) {
        let char = substring(digits, i, 1);
        if (char < "0" || char > "9") {
            return false;
        }
        i = i + 1;
    }

    return true;
}

pub fn is_date(value: String) -> Bool {
    let parts = split(value, "-");
    if (len(parts) != 3) { return false; }

    let year = to_number(parts[0]);
    let month = to_number(parts[1]);
    let day = to_number(parts[2]);

    if (year < 1900 || year > 2100) { return false; }
    if (month < 1 || month > 12) { return false; }
    if (day < 1 || day > 31) { return false; }

    return true;
}

pub fn validate_required(value: Any) -> Bool {
    return value != null && value != "";
}

pub fn validate_min_length(value: String, min: Int) -> Bool {
    return len(value) >= min;
}

pub fn validate_max_length(value: String, max: Int) -> Bool {
    return len(value) <= max;
}

pub fn validate_range(value: Int, min: Int, max: Int) -> Bool {
    return value >= min && value <= max;
}

pub fn validate_in_list(value: Any, allowed: Array) -> Bool {
    for item in allowed {
        if (item == value) { return true; }
    }
    return false;
}

pub class Validator {
    fn new(rules: Hash) -> Any {
        let v = {};
        v["rules"] = rules;
        return v;
    }

    fn validate(data: Hash) -> Any {
        let errors = [];

        for field in keys(this["rules"]) {
            let value = data[field];
            let rules = this["rules"][field];

            for rule in rules {
                let { type: rule_type, params: rule_params } = rule;

                if (rule_type == "required" && !validate_required(value)) {
                    errors.push({ field: field, message: field + " is required" });
                }

                if (rule_type == "email" && !is_email(value)) {
                    errors.push({ field: field, message: field + " must be an email" });
                }

                if (rule_type == "min_length" && !validate_min_length(value, rule_params["min"])) {
                    errors.push({ field: field, message: field + " must be at least " + str(rule_params["min"]) + " characters" });
                }

                if (rule_type == "max_length" && !validate_max_length(value, rule_params["max"])) {
                    errors.push({ field: field, message: field + " must be at most " + str(rule_params["max"]) + " characters" });
                }

                if (rule_type == "range" && !validate_range(value, rule_params["min"], rule_params["max"])) {
                    errors.push({ field: field, message: field + " must be between " + str(rule_params["min"]) + " and " + str(rule_params["max"]) });
                }

                if (rule_type == "in_list" && !validate_in_list(value, rule_params["allowed"])) {
                    errors.push({ field: field, message: field + " must be one of: " + join(rule_params["allowed"], ", ") });
                }
            }
        }

        return {
            "valid": len(errors) == 0,
            "errors": errors
        };
    }
}

// ============================================================================
// EXAMPLE 4: Date/Time Module
// ============================================================================

// File: app/modules/datetime.sl
pub fn now() -> Hash {
    return {
        "year": 2024,
        "month": 1,
        "day": 15,
        "hour": 10,
        "minute": 30,
        "second": 45,
        "timestamp": clock()
    };
}

pub fn format(dt: Hash, format_str: String) -> String {
    let result = format_str;

    result = replace(result, "%Y", str(dt["year"]));
    result = replace(result, "%m", pad_zero(dt["month"]));
    result = replace(result, "%d", pad_zero(dt["day"]));
    result = replace(result, "%H", pad_zero(dt["hour"]));
    result = replace(result, "%M", pad_zero(dt["minute"]));
    result = replace(result, "%S", pad_zero(dt["second"]));

    return result;
}

pub fn parse(date_str: String) -> Hash {
    let parts = split(date_str, "-");
    if (len(parts) != 3) {
        throw ValueError.new("Invalid date format");
    }

    return {
        "year": to_number(parts[0]),
        "month": to_number(parts[1]),
        "day": to_number(parts[2])
    };
}

pub fn add_days(dt: Hash, days: Int) -> Hash {
    return {
        "year": dt["year"],
        "month": dt["month"],
        "day": dt["day"] + days,
        "hour": dt["hour"],
        "minute": dt["minute"],
        "second": dt["second"]
    };
}

pub fn diff_days(a: Hash, b: Hash) -> Int {
    return b["day"] - a["day"];
}

fn pad_zero(n: Int) -> String {
    if (n < 10) {
        return "0" + str(n);
    }
    return str(n);
}

// ============================================================================
// EXAMPLE 5: Controller Using Modules
// ============================================================================

// File: app/controllers/users_controller.sl
import "modules/string_helpers";
import { Validator } from "modules/validators";
import { now, format } from "modules/datetime";
import "modules/math" as Math;

class UsersController extends Controller {
    static {
        this.layout = "application";
    }

    fn create(req: Any) -> Any {
        let data = req["json"];

        let v = Validator.new({
            "name": [
                { "type": "required" },
                { "type": "min_length", "params": { "min": 2 } }
            ],
            "email": [
                { "type": "required" },
                { "type": "email" }
            ],
            "age": [
                { "type": "required" },
                { "type": "range", "params": { "min": 18, "max": 120 } }
            ]
        });

        let result = v.validate(data);

        if (!result["valid"]) {
            return {
                "status": 422,
                "body": json_stringify({ "errors": result["errors"] })
            };
        }

        let user = solidb_insert(this.db, this.database, "users", {
            "name": string_helpers.trim(data["name"]),
            "email": downcase(data["email"]),
            "age": data["age"],
            "created_at": format(now(), "%Y-%m-%d")
        });

        return {
            "status": 201,
            "body": json_stringify({ "user": user })
        };
    }

    fn index(req: Any) -> Any {
        let users = solidb_query(this.db, this.database, "FOR u IN users RETURN u", {});

        let names = users.map(fn(u) {
            return string_helpers.capitalize(u["name"]);
        });

        return {
            "status": 200,
            "body": json_stringify({ "users": users, "names": names })
        };
    }
}

// ============================================================================
// EXAMPLE 6: Module with Classes
// ============================================================================

// File: app/modules/cache.sl
pub class Cache {
    fn new(ttl: Int) -> Any {
        let c = {};
        c["data"] = {};
        c["timestamps"] = {};
        c["default_ttl"] = ttl ?? 300;
        return c;
    }

    fn get(key: String) -> Any {
        if (!has_key(this["data"], key)) {
            return null;
        }

        let expires = this["timestamps"][key];
        if (clock() > expires) {
            this.delete(key);
            return null;
        }

        return this["data"][key];
    }

    fn set(key: String, value: Any, ttl: Int) -> Any {
        this["data"][key] = value;
        this["timestamps"][key] = clock() + (ttl ?? this["default_ttl"]);
        return this;
    }

    fn delete(key: String) -> Any {
        if (has_key(this["data"], key)) {
            this["data"][key] = null;
            this["timestamps"][key] = null;
        }
        return this;
    }

    fn clear() -> Any {
        this["data"] = {};
        this["timestamps"] = {};
        return    fn has(key this;
    }

: String) -> Bool {
        return this.get(key) != null;
    }
}

// ============================================================================
// EXAMPLE 7: Module with Default Export
// ============================================================================

// File: app/modules/api_client.sl
pub (default) fn create_client(base_url: String, api_key: String) -> Any {
    return {
        "base_url": base_url,
        "api_key": api_key,
        "get": fn(path: String) -> Any {
            return http_get(base_url + path, { "Authorization": "Bearer " + api_key });
        },
        "post": fn(path: String, data: Hash) -> Any {
            return http_post(base_url + path, data, { "Authorization": "Bearer " + api_key });
        }
    };
}

pub fn create_authenticated_client(base_url: String, username: String, password: String) -> Any {
    return create_client(base_url, username + ":" + password);
}

// Using the module
// import api_client from "modules/api_client";
// let client = api_client("https://api.example.com", "key123");

// ============================================================================
// EXAMPLE 8: Re-exporting Modules
// ============================================================================

// File: app/modules/all_helpers.sl
pub import "modules/string_helpers";
pub import "modules/datetime";
pub import "modules/validators";

// Now you can import everything from one place:
// import { capitalize, now, Validator } from "modules/all_helpers";

// ============================================================================
// EXAMPLE 9: Circular Imports and Module Patterns
// ============================================================================

// File: app/modules/base_controller.sl
pub fn create_base_controller() -> Any {
    return {
        "render": fn(template: String, data: Hash) -> Any {
            return { "template": template, "data": data };
        },
        "redirect": fn(url: String) -> Any {
            return { "status": 302, "headers": { "Location": url } };
        },
        "json": fn(data: Hash) -> Any {
            return { "body": json_stringify(data), "headers": { "Content-Type": "application/json" } };
        }
    };
}

// ============================================================================
// EXAMPLE 10: Module Configuration Pattern
// ============================================================================

// File: app/modules/config.sl
let config = {
    "app": {
        "name": "My Soli App",
        "version": "1.0.0",
        "env": "development"
    },
    "database": {
        "host": "localhost",
        "port": 6745,
        "name": "myapp"
    },
    "cache": {
        "default_ttl": 300,
        "max_size": 1000
    }
};

pub fn get(path: String) -> Any {
    let parts = split(path, ".");
    let current = config;
    let key = "";

    for part in parts {
        if (typeof(current) == "hash" && has_key(current, part)) {
            current = current[part];
        } else {
            return null;
        }
    }

    return current;
}

pub fn get_app(key: String) -> Any {
    return config["app"][key];
}

pub fn get_database(key: String) -> Any {
    return config["database"][key];
}

pub fn is_development() -> Bool {
    return config["app"]["env"] == "development";
}

pub fn is_production() -> Bool {
    return config["app"]["env"] == "production";
}

// ============================================================================
// EXAMPLE 11: Using Modules in Controllers
// ============================================================================

// File: app/controllers/posts_controller.sl
import "modules/config" as Config;
import "modules/cache" as Cache;
import { Validator } from "modules/validators";
import { now, format } from "modules/datetime";

class PostsController extends Controller {
    static {
        this.layout = "application";
        this.cache = Cache.new(Config.get("cache.default_ttl"));
    }

    fn index(req: Any) -> Any {
        let cache_key = "posts:index";

        let cached = this.cache.get(cache_key);
        if (cached != null) {
            return { "body": cached };
        }

        let posts = solidb_query(this.db, this.database, "FOR p IN posts SORT p.created_at DESC RETURN p", {});

        let response = {
            "posts": posts,
            "count": len(posts),
            "generated_at": format(now(), "%Y-%m-%d %H:%M:%S")
        };

        this.cache.set(cache_key, json_stringify(response));

        return { "body": json_stringify(response) };
    }

    fn show(req: Any) -> Any {
        let id = req["params"]["id"];

        let v = Validator.new({
            "id": [
                { "type": "required" }
            ]
        });

        let result = v.validate({ "id": id });
        if (!result["valid"]) {
            return { "status": 400, "body": json_stringify({ "error": result["errors"] }) };
        }

        let post = solidb_get(this.db, this.database, "posts", id);

        if (post == null) {
            return { "status": 404, "body": json_stringify({ "error": "Post not found" }) };
        }

        return { "body": json_stringify({ "post": post }) };
    }
}

// ============================================================================
// EXAMPLE 12: Middleware Using Modules
// ============================================================================

// File: app/middleware/authenticate.sl
import "modules/config" as Config;
import "modules/cache" as Cache;

let token_cache = Cache.new(3600);

fn authenticate(req: Any) -> Any {
    let auth_header = req["headers"]["Authorization"];

    if (auth_header == null) {
        return {
            "continue": false,
            "response": {
                "status": 401,
                "body": json_stringify({ "error": "Missing authorization header" })
            }
        };
    }

    let token = replace(auth_header, "Bearer ", "");

    let cached_user = token_cache.get(token);
    if (cached_user != null) {
        req["user"] = cached_user;
        return { "continue": true, "request": req };
    }

    try {
        let user = verify_token(token);
        token_cache.set(token, user);
        req["user"] = user;
        return { "continue": true, "request": req };
    } catch (e) {
        return {
            "continue": false,
            "response": {
                "status": 401,
                "body": json_stringify({ "error": "Invalid token" })
            }
        };
    }
}

fn verify_token(token: String) -> Any {
    return { "id": "user123", "role": "admin", "token": token };
}

// ============================================================================
// EXAMPLE 13: Complete Module Import Patterns
// ============================================================================

// All import patterns together
// ============================================================================
// Pattern 1: Import entire module
// import "modules/string_helpers";
// string_helpers.capitalize("hello");

// Pattern 2: Import with alias
// import "modules/string_helpers" as Str;
// Str.capitalize("hello");

// Pattern 3: Import specific functions
// import { capitalize, trim, split } from "modules/string_helpers";
// capitalize("hello");

// Pattern 4: Import with alias for specific functions
// import { capitalize as cap } from "modules/string_helpers";
// cap("hello");

// Pattern 5: Import multiple from same module
// import { capitalize, trim } from "modules/string_helpers";
// import { now, format } from "modules/datetime";

// Pattern 6: Import default export
// import api_client from "modules/api_client";
// let client = api_client("https://api.example.com", "key");

// Pattern 7: Import all as namespace
// import * as Helpers from "modules/all_helpers";
// Helpers.capitalize("hello");
// Helpers.now();

// ============================================================================
// EXAMPLE 14: Module File Organization
// ============================================================================

// app/modules/
// ├── string_helpers.sl    - String manipulation functions
// ├── datetime.sl          - Date/time utilities
// ├── validators.sl        - Validation helpers
// ├── cache.sl             - Cache implementation
// ├── config.sl            - Configuration management
// ├── math.sl              - Math functions
// ├── api_client.sl        - HTTP client with default export
// ├── all_helpers.sl       - Re-exports for convenience
// └── middleware_helpers.sl - Middleware utilities

// ============================================================================
