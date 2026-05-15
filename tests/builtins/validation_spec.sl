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

    test("V.string().pattern() validates regex pattern", fn() {
        let schema = hash();
        schema["zip"] = V.string().pattern("^\\d{5}$");

        let valid_data = hash();
        valid_data["zip"] = "12345";
        let result = validate(valid_data, schema);
        assert(result["valid"]);

        let invalid_data = hash();
        invalid_data["zip"] = "abc";
        let result2 = validate(invalid_data, schema);
        assert_not(result2["valid"]);
    });

    test("V.string().url() validates URL format", fn() {
        let schema = hash();
        schema["website"] = V.string().url();

        let valid_data = hash();
        valid_data["website"] = "https://example.com";
        let result = validate(valid_data, schema);
        assert(result["valid"]);

        let invalid_data = hash();
        invalid_data["website"] = "not a url";
        let result2 = validate(invalid_data, schema);
        assert_not(result2["valid"]);
    });

    test("V.string().one_of() validates against allowed values", fn() {
        let schema = hash();
        schema["status"] = V.string().one_of(["active", "inactive", "pending"]);

        let valid_data = hash();
        valid_data["status"] = "active";
        let result = validate(valid_data, schema);
        assert(result["valid"]);

        let invalid_data = hash();
        invalid_data["status"] = "deleted";
        let result2 = validate(invalid_data, schema);
        assert_not(result2["valid"]);
    });

    test("V.int().one_of() validates numeric values", fn() {
        let schema = hash();
        schema["priority"] = V.int().one_of([1, 2, 3]);

        let valid_data = hash();
        valid_data["priority"] = 2;
        let result = validate(valid_data, schema);
        assert(result["valid"]);

        let invalid_data = hash();
        invalid_data["priority"] = 5;
        let result2 = validate(invalid_data, schema);
        assert_not(result2["valid"]);
    });

    test("field is optional when .optional() is used", fn() {
        let schema = hash();
        schema["nickname"] = V.string().optional();

        let valid_data = hash();
        valid_data["nickname"] = "nick";
        let result = validate(valid_data, schema);
        assert(result["valid"]);

        let missing_data = hash();
        let result2 = validate(missing_data, schema);
        assert(result2["valid"]);
    });

    test("field can be null when .nullable() is used", fn() {
        let schema = hash();
        schema["middle_name"] = V.string().nullable();

        let valid_data = hash();
        valid_data["middle_name"] = null;
        let result = validate(valid_data, schema);
        assert(result["valid"]);

        let present_data = hash();
        present_data["middle_name"] = "Marie";
        let result2 = validate(present_data, schema);
        assert(result2["valid"]);
    });

    test("field has default value when .default() is used", fn() {
        let schema = hash();
        schema["country"] = V.string().default("US");

        let data_with_value = hash();
        data_with_value["country"] = "FR";
        let result = validate(data_with_value, schema);
        assert(result["valid"]);
        assert_eq(result["data"]["country"], "FR");

        let data_without_value = hash();
        let result2 = validate(data_without_value, schema);
        assert(result2["valid"]);
        assert_eq(result2["data"]["country"], "US");
    });

    test("V.int().default() applies default to missing field", fn() {
        let schema = hash();
        schema["attempts"] = V.int().default(0);

        let data_without_value = hash();
        let result = validate(data_without_value, schema);
        assert(result["valid"]);
        assert_eq(result["data"]["attempts"], 0);
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

    describe("confirmation", fn() {
        test("confirmation passes when values match", fn() {
            let schema = hash();
            schema["password"] = V.string().required();
            schema["confirm_password"] = V.string().required().confirmation("password");
            let valid_data = hash();
            valid_data["password"] = "Secret123!";
            valid_data["confirm_password"] = "Secret123!";
            let result = validate(valid_data, schema);
            assert(result["valid"]);
        });

        test("confirmation fails when values do not match", fn() {
            let schema = hash();
            schema["password"] = V.string().required();
            schema["confirm_password"] = V.string().required().confirmation("password");
            let invalid_data = hash();
            invalid_data["password"] = "Secret123!";
            invalid_data["confirm_password"] = "Different!";
            let result = validate(invalid_data, schema);
            assert_not(result["valid"]);
            assert_contains(result["errors"][0]["message"], "not match");
        });

        test("confirmation fails when confirmed field is missing", fn() {
            let schema = hash();
            schema["password"] = V.string().required();
            schema["confirm_password"] = V.string().required().confirmation("password");
            let invalid_data = hash();
            invalid_data["confirm_password"] = "Secret123!";
            let result = validate(invalid_data, schema);
            assert_not(result["valid"]);
        });

        test("confirmation works with non-string types", fn() {
            let schema = hash();
            schema["email"] = V.string().required().email();
            schema["email_confirm"] = V.string().required().confirmation("email");
            let valid_data = hash();
            valid_data["email"] = "user@example.com";
            valid_data["email_confirm"] = "user@example.com";
            let result = validate(valid_data, schema);
            assert(result["valid"]);
        });
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
