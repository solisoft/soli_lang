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
});
