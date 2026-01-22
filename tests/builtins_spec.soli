// ============================================================================
// Solilang Built-in Methods Test Suite
// ============================================================================
// Comprehensive tests for all built-in functions and methods
// ============================================================================

// ----------------------------------------------------------------------------
// Type Conversion Functions
// ----------------------------------------------------------------------------
describe("Type Conversion", fn() {
    test("str() converts values to string", fn() {
        assert_eq(str(42), "42");
        assert_eq(str(3.14), "3.14");
        assert_eq(str(true), "true");
        assert_eq(str(false), "false");
        assert_eq(str(null), "null");
    });

    test("int() converts values to integer", fn() {
        assert_eq(int("42"), 42);
        assert_eq(int(3.14), 3);
        assert_eq(int(3.99), 3);
        assert_eq(int(-3.14), -3);
    });

    test("float() converts values to float", fn() {
        assert_eq(float("3.14"), 3.14);
        assert_eq(float(42), 42.0);
    });

    test("type() returns type name", fn() {
        assert_eq(type(42), "int");
        assert_eq(type(3.14), "float");
        assert_eq(type("hello"), "string");
        assert_eq(type(true), "bool");
        assert_eq(type(null), "null");
        assert_eq(type([1, 2, 3]), "array");
        assert_eq(type(hash()), "hash");
    });
});

// ----------------------------------------------------------------------------
// String Functions
// ----------------------------------------------------------------------------
describe("String Functions", fn() {
    test("len() returns string length", fn() {
        assert_eq(len("hello"), 5);
        assert_eq(len(""), 0);
        assert_eq(len("hello world"), 11);
    });

    test("contains() checks for substring", fn() {
        assert(contains("hello world", "world"));
        assert(contains("hello world", "hello"));
        assert_not(contains("hello world", "foo"));
    });

    test("index_of() finds substring position", fn() {
        assert_eq(index_of("hello world", "world"), 6);
        assert_eq(index_of("hello world", "hello"), 0);
        assert_eq(index_of("hello world", "foo"), -1);
    });

    test("substring() extracts part of string", fn() {
        assert_eq(substring("hello world", 0, 5), "hello");
        assert_eq(substring("hello world", 6, 11), "world");
    });

    test("upcase() converts to uppercase", fn() {
        assert_eq(upcase("hello"), "HELLO");
        assert_eq(upcase("Hello World"), "HELLO WORLD");
    });

    test("downcase() converts to lowercase", fn() {
        assert_eq(downcase("HELLO"), "hello");
        assert_eq(downcase("Hello World"), "hello world");
    });

    test("trim() removes whitespace", fn() {
        assert_eq(trim("  hello  "), "hello");
        assert_eq(trim("\n\thello\t\n"), "hello");
    });

    test("split() splits string into array", fn() {
        let parts = split("a,b,c", ",");
        assert_eq(len(parts), 3);
        assert_eq(parts[0], "a");
        assert_eq(parts[1], "b");
        assert_eq(parts[2], "c");
    });

    test("join() joins array into string", fn() {
        assert_eq(join(["a", "b", "c"], ","), "a,b,c");
        assert_eq(join(["hello", "world"], " "), "hello world");
    });

    test("html_escape() escapes HTML characters", fn() {
        assert_eq(html_escape("<div>"), "&lt;div&gt;");
        assert_eq(html_escape("a & b"), "a &amp; b");
        assert_eq(html_escape("\"quoted\""), "&quot;quoted&quot;");
    });

    test("html_unescape() unescapes HTML entities", fn() {
        assert_eq(html_unescape("&lt;div&gt;"), "<div>");
        assert_eq(html_unescape("a &amp; b"), "a & b");
    });
});

// ----------------------------------------------------------------------------
// Array Functions
// ----------------------------------------------------------------------------
describe("Array Functions", fn() {
    test("len() returns array length", fn() {
        assert_eq(len([1, 2, 3]), 3);
        assert_eq(len([]), 0);
    });

    test("push() adds element to array", fn() {
        let arr = [1, 2];
        push(arr, 3);
        assert_eq(len(arr), 3);
        assert_eq(arr[2], 3);
    });

    test("pop() removes last element", fn() {
        let arr = [1, 2, 3];
        let last = pop(arr);
        assert_eq(last, 3);
        assert_eq(len(arr), 2);
    });

    test("range() creates array of numbers", fn() {
        let r = range(0, 5);
        assert_eq(len(r), 5);
        assert_eq(r[0], 0);
        assert_eq(r[4], 4);
    });

    test("assert_contains works with arrays", fn() {
        let arr = [1, 2, 3];
        assert_contains(arr, 2);
    });
});

// ----------------------------------------------------------------------------
// Hash Functions
// ----------------------------------------------------------------------------
describe("Hash Functions", fn() {
    test("hash() creates empty hash", fn() {
        let h = hash();
        assert_eq(type(h), "hash");
        assert_eq(len(h), 0);
    });

    test("len() returns hash size", fn() {
        let h = hash();
        h["a"] = 1;
        h["b"] = 2;
        assert_eq(len(h), 2);
    });

    test("keys() returns all keys", fn() {
        let h = hash();
        h["a"] = 1;
        h["b"] = 2;
        let k = keys(h);
        assert_eq(len(k), 2);
        assert_contains(k, "a");
        assert_contains(k, "b");
    });

    test("values() returns all values", fn() {
        let h = hash();
        h["a"] = 1;
        h["b"] = 2;
        let v = values(h);
        assert_eq(len(v), 2);
        assert_contains(v, 1);
        assert_contains(v, 2);
    });

    test("has_key() checks for key existence", fn() {
        let h = hash();
        h["a"] = 1;
        assert(has_key(h, "a"));
        assert_not(has_key(h, "b"));
    });

    test("delete() removes key from hash", fn() {
        let h = hash();
        h["a"] = 1;
        h["b"] = 2;
        let deleted = delete(h, "a");
        assert_eq(deleted, 1);
        assert_not(has_key(h, "a"));
        assert_eq(len(h), 1);
    });

    test("merge() combines two hashes", fn() {
        let h1 = hash();
        h1["a"] = 1;
        let h2 = hash();
        h2["b"] = 2;
        let merged = merge(h1, h2);
        assert_eq(len(merged), 2);
        assert_eq(merged["a"], 1);
        assert_eq(merged["b"], 2);
    });

    test("entries() returns key-value pairs", fn() {
        let h = hash();
        h["a"] = 1;
        let e = entries(h);
        assert_eq(len(e), 1);
        assert_eq(e[0][0], "a");
        assert_eq(e[0][1], 1);
    });

    test("from_entries() creates hash from pairs", fn() {
        let pairs = [["a", 1], ["b", 2]];
        let h = from_entries(pairs);
        assert_eq(h["a"], 1);
        assert_eq(h["b"], 2);
    });

    test("clear() removes all entries", fn() {
        let h = hash();
        h["a"] = 1;
        h["b"] = 2;
        clear(h);
        assert_eq(len(h), 0);
    });

    test("assert_hash_has_key works", fn() {
        let h = hash();
        h["key"] = "value";
        assert_hash_has_key(h, "key");
    });
});

// ----------------------------------------------------------------------------
// Math Functions
// ----------------------------------------------------------------------------
describe("Math Functions", fn() {
    test("abs() returns absolute value", fn() {
        assert_eq(abs(-5), 5);
        assert_eq(abs(5), 5);
        assert_eq(abs(-3.14), 3.14);
    });

    test("min() returns minimum value", fn() {
        assert_eq(min(3, 5), 3);
        assert_eq(min(10, 2), 2);
        assert_eq(min(-1, 1), -1);
    });

    test("max() returns maximum value", fn() {
        assert_eq(max(3, 5), 5);
        assert_eq(max(10, 2), 10);
        assert_eq(max(-1, 1), 1);
    });

    test("sqrt() returns square root", fn() {
        assert_eq(sqrt(4), 2.0);
        assert_eq(sqrt(9), 3.0);
        assert_eq(sqrt(2), 1.4142135623730951);
    });

    test("pow() returns power", fn() {
        assert_eq(pow(2, 3), 8.0);
        assert_eq(pow(10, 2), 100.0);
        assert_eq(pow(2, 0), 1.0);
    });
});

// ----------------------------------------------------------------------------
// Clock/Time Functions
// ----------------------------------------------------------------------------
describe("Clock Functions", fn() {
    test("clock() returns current time", fn() {
        let t1 = clock();
        let t2 = clock();
        assert(t2 >= t1);
        assert(t1 > 0);
    });
});

// ----------------------------------------------------------------------------
// Regex Functions
// ----------------------------------------------------------------------------
describe("Regex Functions", fn() {
    test("regex_match() checks pattern match", fn() {
        assert(regex_match("\\d+", "123"));
        assert(regex_match("[a-z]+", "hello"));
        assert_not(regex_match("\\d+", "hello"));
    });

    test("regex_find() finds first match", fn() {
        let result = regex_find("\\d+", "abc123def456");
        assert_not_null(result);
        assert_eq(result["match"], "123");
    });

    test("regex_find_all() finds all matches", fn() {
        let results = regex_find_all("\\d+", "abc123def456");
        assert_eq(len(results), 2);
        assert_eq(results[0]["match"], "123");
        assert_eq(results[1]["match"], "456");
    });

    test("regex_replace() replaces first match", fn() {
        let result = regex_replace("\\d+", "abc123def456", "X");
        assert_eq(result, "abcXdef456");
    });

    test("regex_replace_all() replaces all matches", fn() {
        let result = regex_replace_all("\\d+", "abc123def456", "X");
        assert_eq(result, "abcXdefX");
    });

    test("regex_split() splits by pattern", fn() {
        let parts = regex_split("\\s+", "hello   world  foo");
        assert_eq(len(parts), 3);
        assert_eq(parts[0], "hello");
        assert_eq(parts[1], "world");
        assert_eq(parts[2], "foo");
    });

    test("regex_escape() escapes special characters", fn() {
        let escaped = regex_escape("hello.world");
        assert_eq(escaped, "hello\\.world");
    });
});

// ----------------------------------------------------------------------------
// JSON Functions
// ----------------------------------------------------------------------------
describe("JSON Functions", fn() {
    test("json_parse() parses JSON string", fn() {
        let obj = json_parse("{\"name\": \"test\", \"value\": 42}");
        assert_eq(obj["name"], "test");
        assert_eq(obj["value"], 42);
    });

    test("json_parse() parses arrays", fn() {
        let arr = json_parse("[1, 2, 3]");
        assert_eq(len(arr), 3);
        assert_eq(arr[0], 1);
    });

    test("json_stringify() converts to JSON", fn() {
        let h = hash();
        h["name"] = "test";
        let json = json_stringify(h);
        assert_contains(json, "\"name\"");
        assert_contains(json, "\"test\"");
    });

    test("assert_json validates JSON strings", fn() {
        assert_json("{\"valid\": true}");
        assert_json("[1, 2, 3]");
    });
});

// ----------------------------------------------------------------------------
// Environment Variable Functions
// ----------------------------------------------------------------------------
describe("Environment Variables", fn() {
    test("setenv() and getenv() work together", fn() {
        setenv("SOLI_TEST_VAR", "test_value");
        assert_eq(getenv("SOLI_TEST_VAR"), "test_value");
    });

    test("hasenv() checks for existence", fn() {
        setenv("SOLI_TEST_EXISTS", "yes");
        assert(hasenv("SOLI_TEST_EXISTS"));
        assert_not(hasenv("SOLI_NONEXISTENT_VAR_12345"));
    });

    test("unsetenv() removes variable", fn() {
        setenv("SOLI_TEST_REMOVE", "value");
        assert(hasenv("SOLI_TEST_REMOVE"));
        unsetenv("SOLI_TEST_REMOVE");
        assert_not(hasenv("SOLI_TEST_REMOVE"));
    });

    test("getenv() returns null for missing vars", fn() {
        assert_null(getenv("SOLI_DEFINITELY_NOT_SET_12345"));
    });
});

// ----------------------------------------------------------------------------
// DateTime Functions
// ----------------------------------------------------------------------------
describe("DateTime Functions", fn() {
    test("datetime_now() returns current time", fn() {
        let now = datetime_now();
        assert_not_null(now);
        assert(now.year() >= 2024);
    });

    test("datetime_from_unix() creates from timestamp", fn() {
        let dt = datetime_from_unix(0);
        assert_eq(dt.year(), 1970);
        assert_eq(dt.month(), 1);
        assert_eq(dt.day(), 1);
    });

    test("datetime instance methods work", fn() {
        let dt = datetime_from_unix(1704067200);
        assert(dt.year() >= 2024);
        assert(dt.month() >= 1);
        assert(dt.month() <= 12);
        assert(dt.day() >= 1);
        assert(dt.day() <= 31);
        assert(dt.hour() >= 0);
        assert(dt.hour() <= 23);
        assert(dt.minute() >= 0);
        assert(dt.minute() <= 59);
        assert(dt.second() >= 0);
        assert(dt.second() <= 59);
    });

    test("datetime arithmetic works", fn() {
        let dt = datetime_from_unix(1704067200);
        let later = dt.add_days(1);
        assert(later.timestamp() > dt.timestamp());

        let earlier = dt.subtract_days(1);
        assert(earlier.timestamp() < dt.timestamp());
    });

    test("datetime_to_unix() converts to timestamp", fn() {
        let dt = datetime_from_unix(1704067200);
        let ts = datetime_to_unix(dt);
        assert_eq(ts, 1704067200);
    });

    test("datetime iso8601() formatting", fn() {
        let dt = datetime_from_unix(0);
        let iso = dt.iso8601();
        assert_contains(iso, "1970");
    });
});

// ----------------------------------------------------------------------------
// Validation Functions
// ----------------------------------------------------------------------------
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

    test("validation returns errors for invalid data", fn() {
        let schema = hash();
        schema["name"] = V.string().required();

        let invalid_data = hash();
        let result = validate(invalid_data, schema);
        assert_not(result["valid"]);
        assert(len(result["errors"]) > 0);
    });
});

// ----------------------------------------------------------------------------
// Cryptography Functions
// ----------------------------------------------------------------------------
describe("Cryptography Functions", fn() {
    test("argon2_hash() and argon2_verify() work", fn() {
        let password = "secret123";
        let hashed = argon2_hash(password);
        assert_not_null(hashed);
        assert(argon2_verify(password, hashed));
        assert_not(argon2_verify("wrong", hashed));
    });

    test("password_hash() and password_verify() aliases work", fn() {
        let password = "mysecret";
        let hashed = password_hash(password);
        assert(password_verify(password, hashed));
    });

    test("x25519_keypair() generates key pair", fn() {
        let keypair = x25519_keypair();
        assert_hash_has_key(keypair, "public");
        assert_hash_has_key(keypair, "private");
        assert(len(keypair["public"]) > 0);
        assert(len(keypair["private"]) > 0);
    });

    test("ed25519_keypair() generates signing key pair", fn() {
        let keypair = ed25519_keypair();
        assert_hash_has_key(keypair, "public");
        assert_hash_has_key(keypair, "private");
    });
});

// ----------------------------------------------------------------------------
// JWT Functions
// ----------------------------------------------------------------------------
describe("JWT Functions", fn() {
    test("jwt_encode() and jwt_decode() work", fn() {
        let payload = hash();
        payload["user_id"] = 123;
        payload["role"] = "admin";

        let secret = "my-secret-key";
        let token = jwt_encode(payload, secret);
        assert_not_null(token);
        assert_contains(token, ".");

        let decoded = jwt_decode(token, secret);
        assert_eq(decoded["user_id"], 123);
        assert_eq(decoded["role"], "admin");
    });

    test("jwt_decode() fails with wrong secret", fn() {
        let payload = hash();
        payload["data"] = "test";
        let token = jwt_encode(payload, "secret1");
        let decoded = jwt_decode(token, "wrong-secret");
        assert_null(decoded);
    });
});

// ----------------------------------------------------------------------------
// Factory Functions
// ----------------------------------------------------------------------------
describe("Factory Functions", fn() {
    test("Factory.define() and Factory.create() work", fn() {
        let user_data = hash();
        user_data["name"] = "Test User";
        user_data["email"] = "test@example.com";
        Factory.define("user", user_data);

        let user = Factory.create("user");
        assert_eq(user["name"], "Test User");
        assert_eq(user["email"], "test@example.com");
    });

    test("Factory.create_with() allows overrides", fn() {
        let base = hash();
        base["name"] = "Default";
        base["active"] = true;
        Factory.define("item", base);

        let overrides = hash();
        overrides["name"] = "Custom";
        let item = Factory.create_with("item", overrides);
        assert_eq(item["name"], "Custom");
        assert_eq(item["active"], true);
    });

    test("Factory.create_list() creates multiple", fn() {
        let data = hash();
        data["type"] = "widget";
        Factory.define("widget", data);

        let widgets = Factory.create_list("widget", 3);
        assert_eq(len(widgets), 3);
    });

    test("Factory.sequence() generates incrementing numbers", fn() {
        let seq1 = Factory.sequence("counter");
        let seq2 = Factory.sequence("counter");
        assert_eq(seq2, seq1 + 1);
    });
});

// ----------------------------------------------------------------------------
// Test DSL Functions
// ----------------------------------------------------------------------------
describe("Test DSL", fn() {
    context("with nested context", fn() {
        test("context blocks work", fn() {
            assert(true);
        });
    });

    it("it() is an alias for test()", fn() {
        assert(true);
    });

    specify("specify() is an alias for test()", fn() {
        assert(true);
    });
});

// ----------------------------------------------------------------------------
// Assertion Functions
// ----------------------------------------------------------------------------
describe("Assertion Functions", fn() {
    test("assert() checks truthiness", fn() {
        assert(true);
        assert(1);
        assert("non-empty");
    });

    test("assert_not() checks falsiness", fn() {
        assert_not(false);
        assert_not(null);
    });

    test("assert_eq() checks equality", fn() {
        assert_eq(1, 1);
        assert_eq("hello", "hello");
        assert_eq([1, 2], [1, 2]);
    });

    test("assert_ne() checks inequality", fn() {
        assert_ne(1, 2);
        assert_ne("hello", "world");
    });

    test("assert_null() checks for null", fn() {
        assert_null(null);
    });

    test("assert_not_null() checks for non-null", fn() {
        assert_not_null(1);
        assert_not_null("value");
    });

    test("assert_gt() checks greater than", fn() {
        assert_gt(5, 3);
        assert_gt(10, 1);
    });

    test("assert_lt() checks less than", fn() {
        assert_lt(3, 5);
        assert_lt(1, 10);
    });

    test("assert_match() checks regex match", fn() {
        assert_match("hello@example.com", "@");
        assert_match("123-456-7890", "\\d{3}");
    });

    test("assert_contains() works with strings", fn() {
        assert_contains("hello world", "world");
    });
});

// ----------------------------------------------------------------------------
// I18n Functions
// ----------------------------------------------------------------------------
describe("I18n Functions", fn() {
    test("I18n.locale() returns current locale", fn() {
        let locale = I18n.locale();
        assert_not_null(locale);
    });

    test("I18n.set_locale() changes locale", fn() {
        I18n.set_locale("en");
        assert_eq(I18n.locale(), "en");
    });

    test("I18n.pluralize() returns correct form", fn() {
        assert_eq(I18n.pluralize(1, "item", "items"), "item");
        assert_eq(I18n.pluralize(2, "item", "items"), "items");
        assert_eq(I18n.pluralize(0, "item", "items"), "items");
    });

    test("I18n.number_format() formats numbers", fn() {
        let formatted = I18n.number_format(1234.56);
        assert_not_null(formatted);
    });
});
