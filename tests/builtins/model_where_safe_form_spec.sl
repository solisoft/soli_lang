// ============================================================================
// Model.where Hash form (safe — SEC-005)
//
// `Model.where({field: value, ...})` validates each key as an AQL
// identifier and pushes values through bind parameters. The legacy
// raw-string form `where("doc.foo == @foo", {foo: ...})` still works for
// developer-trusted call sites; only the Hash form is safe to feed
// untrusted input through.
//
// These specs assert the safe form's *gating* behaviour, which doesn't
// require a live DB: the validator must reject injection-shaped keys
// before any query is built, and well-formed keys must succeed in
// producing a QueryBuilder. The chained form (qb_where) is exercised
// through `Model.order(...).where(...)`.
// ============================================================================

class WhereSafeItem extends Model
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

describe("Model.where Hash form (safe)", fn() {
    test("rejects an injection-shaped key", fn() {
        assert_throws("where hash injected key", fn() {
            WhereSafeItem.where({ "name; REMOVE doc": "x" });
        });
    });

    test("rejects a dotted key", fn() {
        assert_throws("where hash dotted key", fn() {
            WhereSafeItem.where({ "user.email": "x" });
        });
    });

    test("accepts an empty hash (no-op, matches all rows)", fn() {
        // An empty Hash adds no constraint — `where({})` mirrors no `.where`
        // at all and returns a QueryBuilder that matches every row.
        let qb = WhereSafeItem.where({});
        assert(!qb.nil?);
    });

    test("rejects a second argument", fn() {
        // Hash form is single-arg; passing a bind hash is meaningless.
        assert_throws("where hash + binds", fn() {
            WhereSafeItem.where({ "name": "x" }, { "extra": 1 });
        });
    });

    test("accepts well-formed keys and returns a QueryBuilder", fn() {
        let qb = WhereSafeItem.where({ "name": "Alice", "active": true });
        assert(!qb.nil?);
    });
});

describe("QueryBuilder.where Hash form (chain — SEC-005)", fn() {
    test("rejects an injected key in the chain form", fn() {
        assert_throws("qb.where injected key", fn() {
            WhereSafeItem.order("name").where({ "x; REMOVE doc": "y" });
        });
    });

    test("accepts well-formed chained Hash filter", fn() {
        let qb = WhereSafeItem.order("name").where({ "active": true });
        assert(!qb.nil?);
    });
});

describe("Model.where String form still works for trusted call sites", fn() {
    test("string filter + binds returns a QueryBuilder", fn() {
        let qb = WhereSafeItem.where("doc.age >= @age", { "age": 18 });
        assert(!qb.nil?);
    });

    test("bare string filter (no binds) returns a QueryBuilder", fn() {
        let qb = WhereSafeItem.where("doc.active == true");
        assert(!qb.nil?);
    });
});
