// ============================================================================
// AQL injection guard on field-name arguments
//
// Every Model.* method that `format!`-interpolates a field name into an
// AQL template now validates the name against
// [A-Za-z_][A-Za-z0-9_]*. Anything else raises before the query is built,
// so a controller passing `req["params"]["field"]` straight through can't
// inject AQL syntax (semicolons, dots, parens, RETURN/REMOVE clauses…).
// ============================================================================

class FieldGuardItem extends Model
end

fn assert_throws(label, body)
    let threw = false;
    try
        body();
    catch e
        threw = true;
    end
    assert(threw);
end

describe("Field-name validator — find_by / first_by / find_or_create_by", fn() {
    test("rejects a name with a space", fn() {
        assert_throws("find_by space", fn() { FieldGuardItem.find_by("name ; REMOVE", "x"); });
    });

    test("rejects a name with a quote", fn() {
        assert_throws("first_by quote", fn() { FieldGuardItem.first_by("name'", "x"); });
    });

    test("rejects a name with a dot (defense-in-depth)", fn() {
        assert_throws("find_or_create_by dot", fn() {
            FieldGuardItem.find_or_create_by("user.email", "x", {});
        });
    });

    test("rejects an empty name", fn() {
        assert_throws("find_by empty", fn() { FieldGuardItem.find_by("", "x"); });
    });

    test("rejects a name that starts with a digit", fn() {
        assert_throws("find_by digit-first", fn() { FieldGuardItem.find_by("1col", "x"); });
    });
});

describe("Field-name validator — order / select / pluck / aggregations / group_by", fn() {
    test("order rejects parens", fn() {
        assert_throws("order parens", fn() { FieldGuardItem.order("name); REMOVE doc"); });
    });

    test("select rejects spaces", fn() {
        assert_throws("select space", fn() { FieldGuardItem.select("a b"); });
    });

    test("pluck rejects semicolons", fn() {
        assert_throws("pluck semi", fn() { FieldGuardItem.pluck("a;b"); });
    });

    test("sum rejects dashes", fn() {
        assert_throws("sum dash", fn() { FieldGuardItem.sum("a-b"); });
    });

    test("group_by rejects an injected agg field", fn() {
        assert_throws("group_by agg", fn() {
            FieldGuardItem.group_by("status", "sum", "amount; REMOVE doc");
        });
    });

    test("group_by rejects an injected group field", fn() {
        assert_throws("group_by group", fn() {
            FieldGuardItem.group_by("status; REMOVE doc", "sum", "amount");
        });
    });
});

describe("Field-name validator — well-formed names still work", fn() {
    test("snake_case names pass", fn() {
        // These build a QueryBuilder; we don't run the query (no DB), but the
        // validator must not raise. Using `select` because it returns
        // synchronously without executing.
        let qb = FieldGuardItem.select("user_id", "_internal", "x9");
        assert(!qb.nil?);
    });

    test("PascalCase passes", fn() {
        let qb = FieldGuardItem.pluck("UserId");
        assert(!qb.nil?);
    });
});

// ============================================================================
// SEC-004a: Model.order direction argument is also restricted to
// asc/desc/ascending/descending. Same guard applies to the chain form
// `Model.where(...).order(field, dir)`.
// ============================================================================

describe("Order direction validator — Model.order entry", fn() {
    test("rejects an injected direction", fn() {
        assert_throws("order injected dir", fn() {
            FieldGuardItem.order("name", "ASC; REMOVE doc IN x");
        });
    });

    test("rejects an arbitrary unknown direction", fn() {
        assert_throws("order weird dir", fn() {
            FieldGuardItem.order("name", "sideways");
        });
    });

    test("accepts asc/desc/ASC/DESC and the long forms", fn() {
        assert(!FieldGuardItem.order("name", "asc").nil?);
        assert(!FieldGuardItem.order("name", "DESC").nil?);
        assert(!FieldGuardItem.order("name", "Ascending").nil?);
        assert(!FieldGuardItem.order("name", "descending").nil?);
    });
});

describe("Order direction validator — QueryBuilder.order chain", fn() {
    test("rejects an injected direction in the chain form", fn() {
        assert_throws("qb.order injected dir", fn() {
            FieldGuardItem.order("name").order("name", "; REMOVE doc");
        });
    });
});
