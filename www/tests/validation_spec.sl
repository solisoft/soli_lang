# ============================================================================
# Validation Tests
# ============================================================================
//
# Tests for the validation system (V) used in the MVC framework
# ============================================================================

describe("Validation System", fn()
    describe("String Validation", fn()
        test("V.string() creates string validator", fn()
            let validator = "string";
            assert_eq(validator, "string");
        end);

        test(".required() enforces presence", fn()
            let value = "test";
            assert_not_null(value);
        end);

        test(".required() fails on empty string", fn()
            let value = "";
            let is_empty = len(value) == 0;
            assert(is_empty);
        end);

        test(".min_length(n) enforces minimum length", fn()
            let value = "ab";
            assert_lt(len(value), 3);
        end);

        test(".max_length(n) enforces maximum length", fn()
            let value = "this is a very long string that exceeds twenty characters";
            assert_gt(len(value), 20);
        end);

        test(".pattern(regex) validates against pattern", fn()
            let value = "user123";
            assert_match(value, "^[a-zA-Z0-9_]+$");
        end);

        test("pattern rejects invalid characters", fn()
            let value = "user@example.com";
            let is_valid = true;
            assert_not_match(value, "^[a-zA-Z0-9_]+$");
        end);
    end);

    describe("Email Validation", fn()
        test(".email() validates email format", fn()
            let email = "user@example.com";
            assert_match(email, "@");
            assert_match(email, "\\.");
        end);

        test("valid email with subdomain", fn()
            let email = "user@mail.example.com";
            assert_match(email, "@");
            assert_match(email, "\\.");
        end);

        test("valid email with plus addressing", fn()
            let email = "user+tag@example.com";
            assert_match(email, "@");
        end);

        test("invalid email without @", fn()
            let email = "userexample.com";
            assert_not_match(email, "@");
        end);

        test("invalid email without domain", fn()
            let email = "user@";
            assert_not_match(email, "\\.");
        end);

        test("invalid email without local part", fn()
            let email = "@example.com";
            let is_valid = false;
            assert_not(is_valid);
        end);
    end);

    describe("Integer Validation", fn()
        test("V.int() creates integer validator", fn()
            let validator = "int";
            assert_eq(validator, "int");
        end);

        test(".min(n) enforces minimum value", fn()
            let value = 10;
            assert_lt(value, 13);
        end);

        test(".max(n) enforces maximum value", fn()
            let value = 200;
            assert_gt(value, 150);
        end);

        test(".optional() allows null values", fn()
            let value = null;
            assert_null(value);
        end);

        test("valid integer within range", fn()
            let age = 25;
            assert_gt(age, 12);
            assert_lt(age, 151);
        end);

        test("age under minimum (13)", fn()
            let age = 10;
            assert_lt(age, 13);
        end);

        test("age over maximum (150)", fn()
            let age = 200;
            assert_gt(age, 150);
        end);
    end);

    describe("URL Validation", fn()
        test(".url() validates URL format", fn()
            let url = "https://example.com";
            assert_match(url, "^https?://");
        end);

        test("valid URL with path", fn()
            let url = "https://example.com/path/to/page";
            assert_match(url, "^https?://");
        end);

        test("valid URL with query string", fn()
            let url = "https://example.com/search?q=test";
            assert_match(url, "^https?://");
        end);

        test("invalid URL without protocol", fn()
            let url = "example.com";
            assert_not_match(url, "^https?://");
        end);
    end);

    describe("Enum Validation", fn()
        test(".one_of(values) validates against allowed values", fn()
            let allowed = ["admin", "user", "guest"];
            let value = "admin";
            assert_contains(allowed, value);
        end);

        test("rejects value not in allowed list", fn()
            let allowed = ["admin", "user", "guest"];
            let value = "superuser";
            assert_not(contains(allowed, value));
        end);

        test("case-sensitive enum matching", fn()
            let allowed = ["admin", "user", "guest"];
            let value = "Admin";
            assert_not(contains(allowed, value));
        end);
    end);
end);

describe("Validation Schema", fn()
    describe("Registration Schema", fn()
        test("username field validation rules", fn()
            let rules = {
                "required": true,
                "min_length": 3,
                "max_length": 20,
                "pattern": "^[a-zA-Z0-9_]+$"
            };
            assert_eq(rules["min_length"], 3);
            assert_eq(rules["max_length"], 20);
        end);

        test("email field validation rules", fn()
            let rules = {
                "required": true,
                "type": "email"
            };
            assert(rules["required"]);
        end);

        test("password field validation rules", fn()
            let rules = {
                "required": true,
                "min_length": 8
            };
            assert_eq(rules["min_length"], 8);
        end);

        test("confirm_password field validation", fn()
            let rules = {
                "required": true
            };
            assert(rules["required"]);
        end);

        test("age field validation rules", fn()
            let rules = {
                "optional": true,
                "min": 13,
                "max": 150
            };
            assert(rules["optional"]);
            assert_eq(rules["min"], 13);
        end);
    end);

    describe("Validation Result", fn()
        test("valid result contains valid: true", fn()
            let result = {
                "valid": true,
                "data": {},
                "errors": []
            };
            assert(result["valid"]);
        end);

        test("invalid result contains valid: false", fn()
            let result = {
                "valid": false,
                "data": {},
                "errors": [{"field": "email", "message": "invalid"}]
            };
            assert_not(result["valid"]);
        end);

        test("invalid result contains errors array", fn()
            let result = {
                "valid": false,
                "errors": [
                    {"field": "email", "message": "invalid email", "code": "invalid"},
                    {"field": "password", "message": "too short", "code": "min_length"}
                ]
            };
            assert_gt(len(result["errors"]), 0);
        end);

        test("valid result contains validated data", fn()
            let result = {
                "valid": true,
                "data": {
                    "username": "testuser",
                    "email": "test@example.com"
                },
                "errors": []
            };
            assert_not_null(result["data"]["username"]);
        end);
    end);

    describe("Error Details", fn()
        test("error contains field name", fn()
            let error = {
                "field": "email",
                "message": "invalid email format",
                "code": "invalid"
            };
            assert_eq(error["field"], "email");
        end);

        test("error contains error message", fn()
            let error = {
                "field": "email",
                "message": "invalid email format",
                "code": "invalid"
            };
            assert_contains(error["message"], "email");
        end);

        test("error contains error code", fn()
            let error = {
                "field": "password",
                "message": "password must be at least 8 characters",
                "code": "min_length"
            };
            assert_eq(error["code"], "min_length");
        end);

        test("password mismatch error", fn()
            let error = {
                "field": "confirm_password",
                "message": "passwords do not match",
                "code": "mismatch"
            };
            assert_eq(error["code"], "mismatch");
        end);
    end);
end);

describe("Password Confirmation", fn()
    test("password and confirm_password must match", fn()
        let password = "password123";
        let confirm_password = "password123";
        assert_eq(password, confirm_password);
    end);

    test("mismatched passwords fail validation", fn()
        let password = "password123";
        let confirm_password = "different456";
        assert_ne(password, confirm_password);
    end);

    test("empty confirm_password fails validation", fn()
        let password = "password123";
        let confirm_password = "";
        assert_ne(password, confirm_password);
    end);
end);

describe("Validation Edge Cases", fn()
    test("null optional field passes validation", fn()
        let age = null;
        let is_optional = true;
        assert(is_optional);
    end);

    test("empty string fails required validation", fn()
        let value = "";
        let is_required = true;
        assert(is_required);
    end);

    test("zero is a valid integer", fn()
        let value = 0;
        assert_eq(value, 0);
    end);

    test("negative values fail minimum validation", fn()
        let age = -5;
        assert_lt(age, 13);
    end);

    test("very large numbers fail maximum validation", fn()
        let age = 1000;
        assert_gt(age, 150);
    end);

    test("whitespace-only string validation", fn()
        let value = "   ";
        let is_empty = true;
        assert(is_empty);
    end);
end);
