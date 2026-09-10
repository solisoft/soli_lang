// ============================================================================
// find_each / in_batches — keyset-paged iteration over a whole collection
// Each batch filters `_key > <last key seen>`, so only one batch is ever held
// in memory and concurrent writes cannot make the scan skip or repeat rows.
// ============================================================================

class BatchItem extends Model
end

// Detect DB availability. `create` returns the record itself, so a `_key`
// coming back is what proves the round-trip worked.
let __db_available = false;
try
    let __probe = BatchItem.create({ "name": "__probe__", "n": 0 });
    if __probe._key.present?
        __db_available = true;
        __probe.delete();
    end
catch e
end

def raises(block) {
    let raised = false;
    try
        block();
    catch e
        raised = true;
    end
    return raised;
}

// Seed a known set of rows. Returns false when there is no DB, so the
// DB-backed tests can bail out one line in — a top-level `if __db_available`
// around the `describe`s does *not* work: the blocks inside never register.
def seed(names) {
    if !__db_available
        return false;
    end
    for existing in BatchItem.all()
        existing.delete();
    end
    for name in names
        BatchItem.create({ "name": name });
    end
    return true;
}

// ============================================================================
// Tests that do NOT require a DB connection
// ============================================================================

describe("find_each argument checking", fn() {
    test("a non-callable first argument raises", fn() {
        assert(raises(fn() BatchItem.find_each(42)));
    });

    test("no arguments raises", fn() {
        assert(raises(fn() BatchItem.find_each()));
    });

    test("a non-integer batch_size raises", fn() {
        assert(raises(fn() BatchItem.find_each(fn(r) r, { "batch_size": "many" })));
    });

    test("a zero batch_size raises", fn() {
        assert(raises(fn() BatchItem.find_each(fn(r) r, { "batch_size": 0 })));
    });

    test("a batch_size above the ceiling raises", fn() {
        assert(raises(fn() BatchItem.find_each(fn(r) r, { "batch_size": 10001 })));
    });
});

describe("find_each rejects clauses keyset paging cannot serve", fn() {
    // Refused loudly rather than silently ignored: these methods drive
    // data-correction jobs, where a dropped clause is a wrong run that still
    // reports success.
    test("an explicit .order raises", fn() {
        assert(raises(fn() BatchItem.order("name", "asc").find_each(fn(r) r)));
    });

    test("an explicit .limit raises", fn() {
        assert(raises(fn() BatchItem.limit(5).find_each(fn(r) r)));
    });

    test("an explicit .offset raises", fn() {
        assert(raises(fn() BatchItem.offset(5).find_each(fn(r) r)));
    });

    test(".pluck raises — projected rows carry no _key", fn() {
        assert(raises(fn() BatchItem.pluck("name").find_each(fn(r) r)));
    });

    test("an aggregate raises — it returns one value, not records", fn() {
        assert(raises(fn() BatchItem.sum("n").find_each(fn(r) r)));
    });

    test("inside grouped() raises — batches cannot be coalesced", fn() {
        assert(raises(fn() {
            grouped(fn() {
                BatchItem.find_each(fn(r) r);
            });
        }));
    });
});

// ============================================================================
// Tests that REQUIRE a DB connection (they no-op without one)
// ============================================================================

describe("find_each walks every record", fn() {
    test("visits every record exactly once across several batches", fn() {
        if !seed(["b1", "b2", "b3", "b4", "b5"])
            return;
        end
        // batch_size 2 over 5 rows: three round-trips, the last one short.
        let seen = [];
        BatchItem.find_each(fn(item) {
            seen.push(item.name);
        }, { "batch_size": 2 });

        assert_eq(len(seen), 5);
        assert_eq(len(seen.uniq()), 5);
    });

    test("a batch_size larger than the collection still visits everything", fn() {
        if !seed(["b1", "b2", "b3"])
            return;
        end
        let seen = 0;
        BatchItem.find_each(fn(item) {
            seen = seen + 1;
        }, { "batch_size": 1000 });
        assert_eq(seen, 3);
    });

    test("a batch_size of 1 still terminates", fn() {
        if !seed(["b1", "b2", "b3"])
            return;
        end
        let seen = 0;
        BatchItem.find_each(fn(item) {
            seen = seen + 1;
        }, { "batch_size": 1 });
        assert_eq(seen, 3);
    });

    test("composes with a filter", fn() {
        if !seed(["keep1", "keep2", "skip1"])
            return;
        end
        let seen = [];
        BatchItem.where("name LIKE @n", { "n": "keep%" }).find_each(fn(item) {
            seen.push(item.name);
        }, { "batch_size": 1 });
        assert_eq(len(seen), 2);
    });

    test("an empty result set never calls the block", fn() {
        if !seed(["b1"])
            return;
        end
        let called = false;
        BatchItem.where("name == @n", { "n": "nothing-matches-this" }).find_each(fn(item) {
            called = true;
        });
        assert_eq(called, false);
    });
});

describe("in_batches yields arrays", fn() {
    test("hands the block one array per batch", fn() {
        if !seed(["c1", "c2", "c3", "c4", "c5"])
            return;
        end
        let sizes = [];
        BatchItem.in_batches(fn(batch) {
            sizes.push(len(batch));
        }, { "batch_size": 2 });

        // 5 rows at 2 per batch: 2 + 2 + 1.
        assert_eq(len(sizes), 3);
        assert_eq(sizes[0], 2);
        assert_eq(sizes[2], 1);
    });

    test("find_in_batches is the same method", fn() {
        if !seed(["c1", "c2", "c3", "c4", "c5"])
            return;
        end
        let total = 0;
        BatchItem.find_in_batches(fn(batch) {
            total = total + len(batch);
        }, { "batch_size": 2 });
        assert_eq(total, 5);
    });

    test("deleting inside the block does not skip records", fn() {
        if !seed(["d1", "d2", "d3", "d4", "d5"])
            return;
        end
        // The cursor is read *before* the block runs, so a batch that deletes
        // its own rows still positions the next query correctly. Asserted on
        // the visit count rather than a follow-up `.count`, which SoliDB's
        // read-result cache can answer from before the deletes.
        let seen = 0;
        BatchItem.in_batches(fn(batch) {
            for item in batch
                seen = seen + 1;
                item.delete();
            end
        }, { "batch_size": 2 });

        assert_eq(seen, 5);
    });
});
