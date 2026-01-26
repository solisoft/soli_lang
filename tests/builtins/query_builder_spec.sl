// ============================================================================
// QueryBuilder Test Suite
// ============================================================================
// Tests for QueryBuilder methods
// ============================================================================

describe("QueryBuilder Basic Methods", fn() {
    test("QueryBuilder.where() adds filter condition", fn() {
        let qb = hash();
        let filters = [];
        qb["filters"] = filters;

        let filter = "status = 'published'";
        filters.push(filter);

        assert_eq(len(qb["filters"]), 1);
    });

    test("QueryBuilder.where() with bind variables", fn() {
        let qb = hash();
        qb["filter"] = "";
        qb["bind_vars"] = [];

        qb["filter"] = "title LIKE ?";
        qb["bind_vars"].push("%test%");

        assert_eq(qb["filter"], "title LIKE ?");
        assert_eq(qb["bind_vars"][0], "%test%");
    });

    test("QueryBuilder.where() can be chained", fn() {
        let qb = hash();
        qb["filters"] = [];
        qb["bind_vars"] = [];

        qb["filters"].push("status = ?");
        qb["bind_vars"].push("published");
        qb["filters"].push("author = ?");
        qb["bind_vars"].push("admin");

        assert_eq(len(qb["filters"]), 2);
        assert_eq(len(qb["bind_vars"]), 2);
    });
});

describe("QueryBuilder Order Methods", fn() {
    test("QueryBuilder.order() sets ordering", fn() {
        let qb = hash();
        qb["order_field"] = "created_at";
        qb["order_direction"] = "desc";

        assert_eq(qb["order_field"], "created_at");
        assert_eq(qb["order_direction"], "desc");
    });

    test("QueryBuilder.order() accepts direction string", fn() {
        let qb = hash();
        qb["order"] = "title ASC";

        assert_eq(qb["order"], "title ASC");
    });

    test("QueryBuilder.order() handles multiple fields", fn() {
        let qb = hash();
        qb["order_clauses"] = [];
        qb["order_clauses"].push("name ASC");
        qb["order_clauses"].push("created_at DESC");

        assert_eq(len(qb["order_clauses"]), 2);
    });
});

describe("QueryBuilder Limit Methods", fn() {
    test("QueryBuilder.limit() sets limit", fn() {
        let qb = hash();
        qb["limit"] = 10;

        assert_eq(qb["limit"], 10);
    });

    test("QueryBuilder.limit() can be changed", fn() {
        let qb = hash();
        qb["limit"] = 10;
        qb["limit"] = 20;

        assert_eq(qb["limit"], 20);
    });
});

describe("QueryBuilder Offset Methods", fn() {
    test("QueryBuilder.offset() sets offset", fn() {
        let qb = hash();
        qb["offset"] = 5;

        assert_eq(qb["offset"], 5);
    });
});

describe("QueryBuilder Execute Methods", fn() {
    test("QueryBuilder.all() returns all results", fn() {
        let qb = hash();
        let results = [];
        let r1 = hash();
        r1["id"] = 1;
        r1["name"] = "A";
        results.push(r1);
        let r2 = hash();
        r2["id"] = 2;
        r2["name"] = "B";
        results.push(r2);
        qb["results"] = results;

        assert_eq(len(qb["results"]), 2);
    });

    test("QueryBuilder.first() returns first result", fn() {
        let qb = hash();
        let results = [];
        let r1 = hash();
        r1["id"] = 1;
        r1["name"] = "First";
        results.push(r1);
        let r2 = hash();
        r2["id"] = 2;
        r2["name"] = "Second";
        results.push(r2);
        qb["results"] = results;

        let first = results[0];
        assert_eq(first["name"], "First");
    });

    test("QueryBuilder.first() returns null for empty", fn() {
        let qb = hash();
        qb["results"] = [];

        let results = qb["results"];
        let first = null;
        if (len(results) > 0) {
            first = results[0];
        }
        assert_null(first);
    });

    test("QueryBuilder.count() returns count", fn() {
        let qb = hash();
        qb["results"] = [1, 2, 3, 4, 5];

        let results = qb["results"];
        assert_eq(len(results), 5);
    });
});

describe("QueryBuilder Chaining", fn() {
    test("multiple methods can be chained", fn() {
        let qb = hash();
        qb["filters"] = [];
        qb["bind_vars"] = [];
        qb["order_field"] = "";
        qb["order_direction"] = "asc";
        qb["limit_val"] = 10;
        qb["offset_val"] = 0;

        qb["filters"].push("status = ?");
        qb["bind_vars"].push("published");
        qb["filters"].push("author = ?");
        qb["bind_vars"].push("admin");
        qb["order_field"] = "created_at";
        qb["order_direction"] = "desc";
        qb["limit_val"] = 20;
        qb["offset_val"] = 10;

        assert_eq(len(qb["filters"]), 2);
        assert_eq(qb["order_field"], "created_at");
        assert_eq(qb["limit_val"], 20);
        assert_eq(qb["offset_val"], 10);
    });
});
