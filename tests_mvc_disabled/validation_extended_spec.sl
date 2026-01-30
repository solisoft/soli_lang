// ============================================================================
// Validation Extended Test Suite
// ============================================================================
// Additional tests for V validator methods not covered in validation_spec.sl
// ============================================================================

describe("V.string() Validator", fn() {
    test("V.string() creates string validator", fn() {
        let v = V.string();
        assert_not_null(v);
    });

    test("V.string().required() passes for non-empty string", fn() {
        let schema = hash();
        schema["name"] = V.string().required();
        let data = hash();
        data["name"] = "John";
        let result = validate(data, schema);
        assert(result["valid"]);
    });

    test("V.string().required() fails for empty string", fn() {
        let schema = hash();
        schema["name"] = V.string().required();
        let data = hash();
        data["name"] = "";
        let result = validate(data, schema);
        assert_not(result["valid"]);
    });

    test("V.string().required() fails for null", fn() {
        let schema = hash();
        schema["name"] = V.string().required();
        let data = hash();
        data["name"] = null;
        let result = validate(data, schema);
        assert_not(result["valid"]);
    });

    test("V.string().optional() passes for empty string", fn() {
        let schema = hash();
        schema["name"] = V.string().optional();
        let data = hash();
        data["name"] = "";
        let result = validate(data, schema);
        assert(result["valid"]);
    });

    test("V.string().optional() passes for null", fn() {
        let schema = hash();
        schema["name"] = V.string().optional();
        let data = hash();
        data["name"] = null;
        let result = validate(data, schema);
        assert(result["valid"]);
    });

    test("V.string().nullable() passes for null", fn() {
        let schema = hash();
        schema["name"] = V.string().nullable();
        let data = hash();
        data["name"] = null;
        let result = validate(data, schema);
        assert(result["valid"]);
    });

    test("V.string().default() sets default value", fn() {
        let schema = hash();
        schema["name"] = V.string().default("Anonymous");
        let data = hash();
        let result = validate(data, schema);
        assert(result["valid"]);
    });

    test("V.string().pattern() matches regex", fn() {
        let schema = hash();
        schema["phone"] = V.string().pattern("\\d{3}-\\d{3}-\\d{4}");
        let data = hash();
        data["phone"] = "123-456-7890";
        let result = validate(data, schema);
        assert(result["valid"]);
    });

    test("V.string().pattern() fails for invalid pattern", fn() {
        let schema = hash();
        schema["phone"] = V.string().pattern("\\d{3}-\\d{3}-\\d{4}");
        let data = hash();
        data["phone"] = "not-a-phone";
        let result = validate(data, schema);
        assert_not(result["valid"]);
    });

    test("V.string().url() validates URL", fn() {
        let schema = hash();
        schema["website"] = V.string().url();
        let data = hash();
        data["website"] = "https://example.com";
        let result = validate(data, schema);
        assert(result["valid"]);
    });

    test("V.string().url() fails for invalid URL", fn() {
        let schema = hash();
        schema["website"] = V.string().url();
        let data = hash();
        data["website"] = "not-a-url";
        let result = validate(data, schema);
        assert_not(result["valid"]);
    });

    test("V.string().email() validates email", fn() {
        let schema = hash();
        schema["email"] = V.string().email();
        let data = hash();
        data["email"] = "test@example.com";
        let result = validate(data, schema);
        assert(result["valid"]);
    });

    test("V.string().one_of() accepts valid value", fn() {
        let schema = hash();
        schema["color"] = V.string().one_of(["red", "green", "blue"]);
        let data = hash();
        data["color"] = "red";
        let result = validate(data, schema);
        assert(result["valid"]);
    });

    test("V.string().one_of() rejects invalid value", fn() {
        let schema = hash();
        schema["color"] = V.string().one_of(["red", "green", "blue"]);
        let data = hash();
        data["color"] = "yellow";
        let result = validate(data, schema);
        assert_not(result["valid"]);
    });
});

describe("V.int() Validator - Extended", fn() {
    test("V.int().optional() passes for null", fn() {
        let schema = hash();
        schema["age"] = V.int().optional();
        let data = hash();
        data["age"] = null;
        let result = validate(data, schema);
        assert(result["valid"]);
    });

    test("V.int().nullable() passes for null", fn() {
        let schema = hash();
        schema["age"] = V.int().nullable();
        let data = hash();
        data["age"] = null;
        let result = validate(data, schema);
        assert(result["valid"]);
    });

    test("V.int().positive() accepts positive", fn() {
        let schema = hash();
        schema["value"] = V.int().positive();
        let data = hash();
        data["value"] = 1;
        let result = validate(data, schema);
        assert(result["valid"]);
    });

    test("V.int().positive() rejects zero", fn() {
        let schema = hash();
        schema["value"] = V.int().positive();
        let data = hash();
        data["value"] = 0;
        let result = validate(data, schema);
        assert_not(result["valid"]);
    });

    test("V.int().positive() rejects negative", fn() {
        let schema = hash();
        schema["value"] = V.int().positive();
        let data = hash();
        data["value"] = -1;
        let result = validate(data, schema);
        assert_not(result["valid"]);
    });

    test("V.int().negative() accepts negative", fn() {
        let schema = hash();
        schema["value"] = V.int().negative();
        let data = hash();
        data["value"] = -1;
        let result = validate(data, schema);
        assert(result["valid"]);
    });

    test("V.int().non_negative() accepts zero and positive", fn() {
        let schema = hash();
        schema["value"] = V.int().non_negative();
        let data = hash();
        data["value"] = 0;
        let result = validate(data, schema);
        assert(result["valid"]);
    });
});

describe("V.float() Validator", fn() {
    test("V.float() creates float validator", fn() {
        let v = V.float();
        assert_not_null(v);
    });

    test("V.float().required() passes for number", fn() {
        let schema = hash();
        schema["price"] = V.float().required();
        let data = hash();
        data["price"] = 9.99;
        let result = validate(data, schema);
        assert(result["valid"]);
    });

    test("V.float().optional() passes for null", fn() {
        let schema = hash();
        schema["price"] = V.float().optional();
        let data = hash();
        data["price"] = null;
        let result = validate(data, schema);
        assert(result["valid"]);
    });

    test("V.float().positive() accepts positive float", fn() {
        let schema = hash();
        schema["ratio"] = V.float().positive();
        let data = hash();
        data["ratio"] = 0.5;
        let result = validate(data, schema);
        assert(result["valid"]);
    });
});

describe("V.bool() Validator", fn() {
    test("V.bool() creates boolean validator", fn() {
        let v = V.bool();
        assert_not_null(v);
    });

    test("V.bool().required() passes for boolean", fn() {
        let schema = hash();
        schema["active"] = V.bool().required();
        let data = hash();
        data["active"] = true;
        let result = validate(data, schema);
        assert(result["valid"]);
    });

    test("V.bool().optional() passes for null", fn() {
        let schema = hash();
        schema["active"] = V.bool().optional();
        let data = hash();
        data["active"] = null;
        let result = validate(data, schema);
        assert(result["valid"]);
    });

    test("V.bool().is_true() only accepts true", fn() {
        let schema = hash();
        schema["agreed"] = V.bool().is_true();
        let data = hash();
        data["agreed"] = true;
        let result = validate(data, schema);
        assert(result["valid"]);
    });

    test("V.bool().is_true() rejects false", fn() {
        let schema = hash();
        schema["agreed"] = V.bool().is_true();
        let data = hash();
        data["agreed"] = false;
        let result = validate(data, schema);
        assert_not(result["valid"]);
    });

    test("V.bool().is_false() only accepts false", fn() {
        let schema = hash();
        schema["disabled"] = V.bool().is_false();
        let data = hash();
        data["disabled"] = false;
        let result = validate(data, schema);
        assert(result["valid"]);
    });
});

describe("V.array() Validator", fn() {
    test("V.array() creates array validator", fn() {
        let v = V.array();
        assert_not_null(v);
    });

    test("V.array().required() passes for array", fn() {
        let schema = hash();
        schema["items"] = V.array().required();
        let data = hash();
        data["items"] = [1, 2, 3];
        let result = validate(data, schema);
        assert(result["valid"]);
    });

    test("V.array().min_length() enforces minimum", fn() {
        let schema = hash();
        schema["tags"] = V.array().min_length(2);
        let data = hash();
        data["tags"] = ["a", "b", "c"];
        let result = validate(data, schema);
        assert(result["valid"]);
    });

    test("V.array().min_length() rejects short arrays", fn() {
        let schema = hash();
        schema["tags"] = V.array().min_length(2);
        let data = hash();
        data["tags"] = ["a"];
        let result = validate(data, schema);
        assert_not(result["valid"]);
    });

    test("V.array().max_length() enforces maximum", fn() {
        let schema = hash();
        schema["tags"] = V.array().max_length(3);
        let data = hash();
        data["tags"] = ["a", "b"];
        let result = validate(data, schema);
        assert(result["valid"]);
    });

    test("V.array().length() exact size", fn() {
        let schema = hash();
        schema["coords"] = V.array().length(2);
        let data = hash();
        data["coords"] = [10, 20];
        let result = validate(data, schema);
        assert(result["valid"]);
    });
});

describe("V.hash() Validator", fn() {
    test("V.hash() creates hash validator", fn() {
        let v = V.hash();
        assert_not_null(v);
    });

    test("V.hash().required() passes for hash", fn() {
        let schema = hash();
        schema["user"] = V.hash().required();
        let data = hash();
        data["user"] = hash();
        let result = validate(data, schema);
        assert(result["valid"]);
    });

    test("V.hash().shape() validates nested structure", fn() {
        let user_schema = hash();
        user_schema["name"] = V.string().required();
        user_schema["age"] = V.int().optional();

        let schema = hash();
        schema["user"] = V.hash().shape(user_schema);

        let data = hash();
        let user = hash();
        user["name"] = "Alice";
        data["user"] = user;
        let result = validate(data, schema);
        assert(result["valid"]);
    });

    test("V.hash().min_keys() enforces minimum keys", fn() {
        let schema = hash();
        schema["config"] = V.hash().min_keys(2);
        let data = hash();
        let config = hash();
        config["a"] = 1;
        config["b"] = 2;
        data["config"] = config;
        let result = validate(data, schema);
        assert(result["valid"]);
    });

    test("V.hash().max_keys() enforces maximum keys", fn() {
        let schema = hash();
        schema["config"] = V.hash().max_keys(3);
        let data = hash();
        let config = hash();
        config["a"] = 1;
        config["b"] = 2;
        data["config"] = config;
        let result = validate(data, schema);
        assert(result["valid"]);
    });
});
