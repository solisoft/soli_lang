// ============================================================================
// grouped(fn() { ... }) — request-coalescing batch
// Reads inside the block are deferred and combined into one round-trip.
// ============================================================================

class GroupItem extends Model
end

// Detect DB availability (same probe pattern as model_advanced_spec).
let __db_available = false;
try
    let __probe = GroupItem.create({ "name": "__probe__", "n": 0 });
    if __probe["valid"]
        __db_available = true;
        __probe["record"].delete();
    end
catch e
end

// ============================================================================
// Tests that do NOT require a DB connection
// ============================================================================

describe("grouped block control flow", fn() {
    test("runs the block and returns its value", fn() {
        let r = grouped(fn() {
            return 1 + 2;
        });
        assert_eq(r, 3);
    });

    test("a block returning null yields null", fn() {
        let r = grouped(fn() {
            return null;
        });
        assert_null(r);
    });

    test("misuse (non-block argument) raises", fn() {
        let raised = false;
        try
            grouped(42);
        catch e
            raised = true;
        end
        assert(raised);
    });
});

// ============================================================================
// Tests that REQUIRE a DB connection
// ============================================================================

if __db_available

describe("grouped coalesces reads", fn() {
    before_each(fn() {
        for it in GroupItem.all()
            it.delete();
        end
        GroupItem.create({ "name": "g1", "n": 1 });
        GroupItem.create({ "name": "g2", "n": 2 });
        GroupItem.create({ "name": "g3", "n": 3 });
    });

    test("a .all and a .count return the same as running them separately", fn() {
        let baseline_all = GroupItem.all();
        let baseline_count = GroupItem.count;

        let result = grouped(fn() {
            let items = GroupItem.all();
            let total = GroupItem.count;
            return { "items": items, "total": total };
        });

        assert_eq(result["items"].length, baseline_all.length);
        assert_eq(result["total"], baseline_count);
    });

    test("several counts (incl. a filtered count) and an .all coalesce correctly", fn() {
        // Mirrors the admin-dashboard shape that regressed: multiple bare
        // `.count` reads (which compile to `RETURN COLLECTION_COUNT(...)`) plus
        // a `where(...).count` (`RETURN LENGTH(FOR ... RETURN 1)`) and an `.all`
        // in one block. Each bare RETURN must be unwrapped when combined.
        let baseline_count = GroupItem.count;
        let baseline_filtered = GroupItem.where({ "n": 2 }).count;
        let baseline_all = GroupItem.all();

        let result = grouped(fn() {
            let total = GroupItem.count;
            let twos = GroupItem.where({ "n": 2 }).count;
            let items = GroupItem.all();
            return { "total": total, "twos": twos, "items": items };
        });

        assert_eq(result["total"], baseline_count);
        assert_eq(result["twos"], baseline_filtered);
        assert_eq(result["items"].length, baseline_all.length);
    });

    test("find_by inside grouped resolves to the right record", fn() {
        let result = grouped(fn() {
            let found = GroupItem.find_by("name", "g2");
            return found;
        });
        assert_not_null(result);
        assert_eq(result.name, "g2");
    });

    test("reading a result mid-block auto-flushes and stays correct", fn() {
        let out = grouped(fn() {
            let items = GroupItem.all();
            let seen = items.length;          // forces a flush here
            let total = GroupItem.count;      // registered into a fresh batch
            return { "seen": seen, "total": total };
        });
        assert_eq(out["seen"], 3);
        assert_eq(out["total"], 3);
    });

    test("find on a missing id inside grouped still raises", fn() {
        let raised = false;
        try
            grouped(fn() { GroupItem.find("does-not-exist-xyz"); });
        catch e
            raised = true;
        end
        assert(raised);
    });
});

end
