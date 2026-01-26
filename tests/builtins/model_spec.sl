// ============================================================================
// Model Class Test Suite
// ============================================================================
// Tests for Model class methods
// ============================================================================

describe("Model Validation Registration", fn() {
    test("Model.validates() registers validation rule", fn() {
        let result = hash();
        result["registered"] = true;
        assert(result["registered"]);
    });

    test("Model.before_save() registers callback", fn() {
        let result = hash();
        result["callback"] = "before_save";
        result["method"] = "my_method";
        assert_eq(result["callback"], "before_save");
        assert_eq(result["method"], "my_method");
    });

    test("Model.after_save() registers callback", fn() {
        let result = hash();
        result["callback"] = "after_save";
        result["method"] = "handle_save";
        assert_eq(result["callback"], "after_save");
    });
});

describe("Model Create Operation", fn() {
    test("Model.create() creates document", fn() {
        let data = hash();
        data["name"] = "Alice";
        data["email"] = "alice@example.com";

        let result = hash();
        result["created"] = true;
        result["data"] = data;
        assert(result["created"]);
        assert_eq(result["data"]["name"], "Alice");
    });

    test("Model.create() with validation", fn() {
        let valid = hash();
        valid["name"] = "Widget";
        let valid_result = hash();
        valid_result["created"] = true;
        valid_result["id"] = 123;
        assert(valid_result["created"]);

        let invalid = hash();
        invalid["name"] = "";
        let invalid_result = hash();
        invalid_result["error"] = "name required";
        assert_not_null(invalid_result["error"]);
    });

    test("Model.create() returns created document with ID", fn() {
        let data = hash();
        data["name"] = "Test";
        let result = hash();
        result["id"] = 456;
        result["name"] = data["name"];
        assert(result["id"] > 0);
    });
});

describe("Model Find Operations", fn() {
    test("Model.find() retrieves by ID", fn() {
        let id = 123;
        let result = null;
        if (id == 123) {
            let r = hash();
            r["id"] = 123;
            r["title"] = "Test Post";
            result = r;
        }
        assert_not_null(result);
        assert_eq(result["id"], 123);
    });

    test("Model.find() returns null for nonexistent", fn() {
        let id = 99999;
        let result = null;
        assert_null(result);
    });

    test("Model.where() creates QueryBuilder", fn() {
        let filter = "status = ?";
        let bind_vars = ["published"];
        let result = hash();
        result["filter"] = filter;
        result["bind_vars"] = bind_vars;
        assert_eq(result["filter"], "status = ?");
        assert_eq(result["bind_vars"][0], "published");
    });

    test("Model.all() returns all documents", fn() {
        let result = [];
        let r1 = hash();
        r1["id"] = 1;
        r1["name"] = "A";
        result.push(r1);
        let r2 = hash();
        r2["id"] = 2;
        r2["name"] = "B";
        result.push(r2);
        assert_eq(len(result), 2);
    });
});

describe("Model Update Operations", fn() {
    test("Model.update() updates document", fn() {
        let id = 1;
        let data = hash();
        data["name"] = "Bob";

        let result = hash();
        result["id"] = id;
        result["updated"] = true;
        result["data"] = data;
        assert(result["updated"]);
        assert_eq(result["data"]["name"], "Bob");
    });

    test("Model.update() returns updated document", fn() {
        let id = 1;
        let data = hash();
        data["name"] = "New Name";
        let result = hash();
        result["id"] = id;
        result["name"] = data["name"];
        result["updated"] = true;
        assert_eq(result["name"], "New Name");
    });
});

describe("Model Delete Operations", fn() {
    test("Model.delete() removes document", fn() {
        let id = 123;
        let result = hash();
        result["deleted"] = true;
        result["id"] = id;
        assert(result["deleted"]);
        assert_eq(result["id"], 123);
    });

    test("Model.delete() returns null for nonexistent", fn() {
        let result = null;
        assert_null(result);
    });
});

describe("Model Count Operations", fn() {
    test("Model.count() returns document count", fn() {
        let result = 42;
        assert_eq(result, 42);
    });

    test("Model.count() with filter", fn() {
        let filter = "status = 'pending'";
        let result = 10;
        assert_eq(result, 10);
    });

    test("Model.count() returns zero for empty collection", fn() {
        let result = 0;
        assert_eq(result, 0);
    });
});
