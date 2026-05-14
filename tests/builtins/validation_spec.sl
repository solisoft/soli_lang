// ============================================================================
// Validation Functions Test Suite
// ============================================================================

describe("Validation Functions", fn() {
    test("V.string() validates strings", fn() {
        let schema = hash();
        schema["name"] = V.string().required();

        let valid_data = hash();
        valid_data["name"] = "John";
        let result = validate(valid_data, schema);
        assert(result["valid"]);
    });

    test("V.int() validates integers", fn() {
        let schema = hash();
        schema["age"] = V.int().required().min(0);

        let valid_data = hash();
        valid_data["age"] = 25;
        let result = validate(valid_data, schema);
        assert(result["valid"]);
    });

    test("V.string().email() validates email format", fn() {
        let schema = hash();
        schema["email"] = V.string().email();

        let valid_data = hash();
        valid_data["email"] = "test@example.com";
        let result = validate(valid_data, schema);
        assert(result["valid"]);
    });

    test("V.string().min_length() validates minimum length", fn() {
        let schema = hash();
        schema["password"] = V.string().min_length(8);

        let valid_data = hash();
        valid_data["password"] = "longpassword";
        let result = validate(valid_data, schema);
        assert(result["valid"]);

        let invalid_data = hash();
        invalid_data["password"] = "short";
        let result2 = validate(invalid_data, schema);
        assert_not(result2["valid"]);
    });

    test("V.string().max_length() validates maximum length", fn() {
        let schema = hash();
        schema["username"] = V.string().max_length(20);

        let valid_data = hash();
        valid_data["username"] = "validuser";
        let result = validate(valid_data, schema);
        assert(result["valid"]);

        let invalid_data = hash();
        invalid_data["username"] = "averylongusernamethatexceedsthema";
        let result2 = validate(invalid_data, schema);
        assert_not(result2["valid"]);
    });

    test("V.int().max() validates maximum value", fn() {
        let schema = hash();
        schema["quantity"] = V.int().max(100);

        let valid_data = hash();
        valid_data["quantity"] = 50;
        let result = validate(valid_data, schema);
        assert(result["valid"]);

        let invalid_data = hash();
        invalid_data["quantity"] = 101;
        let result2 = validate(invalid_data, schema);
        assert_not(result2["valid"]);
    });

    test("V.int().min() validates minimum value", fn() {
        let schema = hash();
        schema["age"] = V.int().min(18);

        let valid_data = hash();
        valid_data["age"] = 25;
        let result = validate(valid_data, schema);
        assert(result["valid"]);

        let invalid_data = hash();
        invalid_data["age"] = 16;
        let result2 = validate(invalid_data, schema);
        assert_not(result2["valid"]);
    });

    test("validation returns errors for invalid data", fn() {
        let schema = hash();
        schema["name"] = V.string().required();

        let invalid_data = hash();
        let result = validate(invalid_data, schema);
        assert_not(result["valid"]);
        assert(len(result["errors"]) > 0);
    });

    test("chained validators work together", fn() {
        let schema = hash();
        schema["email"] = V.string().required().email();
        schema["age"] = V.int().required().min(0).max(150);

        let valid_data = hash();
        valid_data["email"] = "test@example.com";
        valid_data["age"] = 30;
        let result = validate(valid_data, schema);
        assert(result["valid"]);
    });

    test("V.float() validates floats", fn() {
        let schema = hash();
        schema["price"] = V.float().required();

        let valid_data = hash();
        valid_data["price"] = 9.99;
        let result = validate(valid_data, schema);
        assert(result["valid"]);
    });

    test("V.bool() validates booleans", fn() {
        let schema = hash();
        schema["active"] = V.bool().required();

        let valid_data = hash();
        valid_data["active"] = true;
        let result = validate(valid_data, schema);
        assert(result["valid"]);
    });

    test("V.array() validates arrays", fn() {
        let schema = hash();
        schema["tags"] = V.array().required();

        let valid_data = hash();
        valid_data["tags"] = ["a", "b", "c"];
        let result = validate(valid_data, schema);
        assert(result["valid"]);
    });

    test("V.hash() validates hashes", fn() {
        let schema = hash();
        schema["meta"] = V.hash().required();

        let valid_data = hash();
        valid_data["meta"] = {"key": "value"};
        let result = validate(valid_data, schema);
        assert(result["valid"]);
    });

    describe("to_password_rules_string", fn() {
        test("outputs all password rules in correct order", fn() {
            let rules = V.string()
                .min_length(12)
                .max_length(64)
                .mixed_case()
                .numbers()
                .symbols()
                .to_password_rules_string();
            assert_eq(rules, "minlength: 12; maxlength: 64; required: lower; required: upper; required: digit; required: special;");
        });

        test("returns empty string when no password-relevant rules set", fn() {
            let rules = V.string()
                .email()
                .to_password_rules_string();
            assert_eq(rules, "");
        });

        test("handles letters rule in password rules string", fn() {
            let rules = V.string()
                .letters()
                .to_password_rules_string();
            // letters and mixed_case both map to required: lower; required: upper;
            assert_eq(rules, "required: lower; required: upper;");
        });

        test("validates letters rule rejects value without letters", fn() {
            let schema = hash();
            schema["password"] = V.string().letters();
            let invalid_data = hash();
            invalid_data["password"] = "12345";
            let result = validate(invalid_data, schema);
            assert_not(result["valid"]);
        });

        test("validates letters rule accepts value with letters", fn() {
            let schema = hash();
            schema["password"] = V.string().letters();
            let valid_data = hash();
            valid_data["password"] = "abc123";
            let result = validate(valid_data, schema);
            assert(result["valid"]);
        });

        test("validates mixed_case rule rejects value without mixed case", fn() {
            let schema = hash();
            schema["password"] = V.string().mixed_case();
            let invalid_data = hash();
            invalid_data["password"] = "alllowercase";
            let result = validate(invalid_data, schema);
            assert_not(result["valid"]);
        });

        test("validates mixed_case rule accepts value with mixed case", fn() {
            let schema = hash();
            schema["password"] = V.string().mixed_case();
            let valid_data = hash();
            valid_data["password"] = "MixedCase1";
            let result = validate(valid_data, schema);
            assert(result["valid"]);
        });

        test("validates numbers rule rejects value without digits", fn() {
            let schema = hash();
            schema["password"] = V.string().numbers();
            let invalid_data = hash();
            invalid_data["password"] = "abcdef";
            let result = validate(invalid_data, schema);
            assert_not(result["valid"]);
        });

        test("validates numbers rule accepts value with digits", fn() {
            let schema = hash();
            schema["password"] = V.string().numbers();
            let valid_data = hash();
            valid_data["password"] = "abc123";
            let result = validate(valid_data, schema);
            assert(result["valid"]);
        });

        test("validates symbols rule rejects value without symbols", fn() {
            let schema = hash();
            schema["password"] = V.string().symbols();
            let invalid_data = hash();
            invalid_data["password"] = "abc123";
            let result = validate(invalid_data, schema);
            assert_not(result["valid"]);
        });

        test("validates symbols rule accepts value with symbols", fn() {
            let schema = hash();
            schema["password"] = V.string().symbols();
            let valid_data = hash();
            valid_data["password"] = "abc123!";
            let result = validate(valid_data, schema);
            assert(result["valid"]);
        });

        test("to_password_rules_string is available on non-string validators", fn() {
            let rules = V.int().min(1).to_password_rules_string();
            assert_eq(rules, "");
        });
    });
});
