# ============================================================================
# Model Columnar Test Suite
# Tests for the `columnar`/`column` class-body DSL, insert_rows(),
# aggregate() (scalar + grouped), query() projection filters, count, the
# column index lifecycle, and the document-API lockout.
#
# Isolation strategy: `soli test`'s truncate-reset does NOT cover columnar
# stores, and drop/recreate-per-test is not viable either — the server loses
# the FIRST insert into a freshly created store (the request dies after
# ~10s), so cycling the store re-opens that window on every test. Instead
# the store is primed once (absorbing the lost-first-insert window on fresh
# DBs only) and never dropped; tests stay accumulation-proof through
# per-run unique country tags and count/sum deltas.
# ============================================================================

class ColTestView extends Model
    columnar
    column "url", "string"
    column "ms", "int", nullable: true
    column "country", "string", indexed: true
end

# Document-model canary for the DB probe (the probe only detects a reachable
# DB; the columnar store is primed separately below).
class ColCanary extends Model
end

# The `column` DSL rejects unknown column types when the class body runs, so
# the declaration itself must be wrapped — stash the error for the test below.
let __bad_column_msg = "";
try
    class ColBadType extends Model
        columnar
        column "x", "wrongtype"
    end
catch e
    __bad_column_msg = str(e);
end

# Detect DB availability
let __db_available = false;
try
    let __probe = ColCanary.create({ "name": "__probe__" });
    if !__probe.nil? and !__probe._errors
        __db_available = true;
        __probe.delete();
    end
catch e
end

if __db_available
    # Prime the store. On a fresh DB the first call auto-creates the store
    # from the declared schema and the insert itself is eaten by the server
    # (transport error after ~10s) — the rescue'd retry then lands. When the
    # store already exists this costs one cheap extra row (ms stays inside
    # the 80..200 band the avg assertion relies on).
    try
        ColTestView.insert_rows([{ "url": "/__prime__", "ms": 120, "country": "__prime__" }]);
    catch e
        ColTestView.insert_rows([{ "url": "/__prime__", "ms": 120, "country": "__prime__" }]) rescue null;
    end
end

# Per-run unique tag so grouped/filter assertions never collide with rows
# left behind by earlier tests or earlier runs.
let __run_id = str(clock()).replace(".", "_");

# Standard row set under a caller-chosen country tag: <tag>_fr ms 120/80
# (avg 100), <tag>_de ms 200. Top-level (not describe-scope) because test
# closures don't see describe-scope definitions.
fn insert_rows_for(tag) {
    return ColTestView.insert_rows([
        { "url": "/a", "ms": 120, "country": tag + "_fr" },
        { "url": "/b", "ms": 80, "country": tag + "_fr" },
        { "url": "/c", "ms": 200, "country": tag + "_de" }
    ]);
}

# ============================================================================
# Tests that do NOT require a DB connection
# ============================================================================

describe("Columnar DSL validation (no DB)", fn() {
    test("column DSL rejects unknown column types at class-body time", fn() {
        assert(__bad_column_msg.contains("unknown type"));
        assert(__bad_column_msg.contains("wrongtype"));
    });
});

describe("Columnar document-API lockout (no DB)", fn() {
    test("where() raises no document API", fn() {
        let msg = "";
        try
            ColTestView.where({ "country": "FR" });
        catch e
            msg = str(e);
        end
        assert(msg.contains("no document API"));
    });

    test("create() raises no document API", fn() {
        let msg = "";
        try
            ColTestView.create({ "url": "/x" });
        catch e
            msg = str(e);
        end
        assert(msg.contains("no document API"));
    });

    test("find() raises no document API", fn() {
        let msg = "";
        try
            ColTestView.find("x");
        catch e
            msg = str(e);
        end
        assert(msg.contains("no document API"));
    });

    test("all raises no document API", fn() {
        let msg = "";
        try
            ColTestView.all;
        catch e
            msg = str(e);
        end
        assert(msg.contains("no document API"));
    });
});

describe("Columnar argument validation (no DB)", fn() {
    test("query() op in requires an array value", fn() {
        let msg = "";
        try
            ColTestView.query({
                "columns": ["url"],
                "filter": { "column": "country", "op": "in", "value": "FR" }
            });
        catch e
            msg = str(e);
        end
        assert(msg.contains("requires an array value"));
    });

    test("query() rejects unknown filter ops", fn() {
        let msg = "";
        try
            ColTestView.query({
                "columns": ["url"],
                "filter": { "column": "country", "op": "like", "value": "FR" }
            });
        catch e
            msg = str(e);
        end
        assert(msg.contains("unknown filter op"));
    });

    test("aggregate() rejects unknown operations", fn() {
        let msg = "";
        try
            ColTestView.aggregate("ms", "frobnicate");
        catch e
            msg = str(e);
        end
        assert(msg.contains("unknown operation"));
    });
});

# ============================================================================
# Tests that REQUIRE a DB connection.
#
# NOTE: suite extraction is static and only sees top-level describe() calls,
# so wrapping the describes in `if __db_available ... end` would silently
# skip them even WITH a live DB. Instead each test early-returns when no DB
# is reachable (they then pass trivially, contributing 0 assertions).
# ============================================================================

describe("Columnar insert/aggregate/query/count (DB)", fn() {
    test("insert_rows reports the inserted count and row ids", fn() {
        if !__db_available
            return;
        end
        let result = insert_rows_for("ins_" + __run_id);
        assert_eq(result["inserted"], 3);
        assert_eq(len(result["ids"]), 3);
    });

    test("aggregate returns a scalar without group_by", fn() {
        if !__db_available
            return;
        end
        let sum_before = ColTestView.aggregate("ms", "sum");
        insert_rows_for("agg_" + __run_id);
        let sum_after = ColTestView.aggregate("ms", "sum");
        let delta = sum_after - sum_before;
        assert(delta > 399.9);
        assert(delta < 400.1);

        # Every ms this spec ever writes is in 80..200, so the store-wide
        # average must land there too. (The server truncates int-column
        # averages to an Int, so no exact float assertion.)
        let avg_ms = ColTestView.aggregate("ms", "avg");
        assert(avg_ms >= 80);
        assert(avg_ms <= 200);
    });

    test("grouped aggregate returns rows of group keys plus value", fn() {
        if !__db_available
            return;
        end
        let tag = "grp_" + __run_id;
        insert_rows_for(tag);
        let rows = ColTestView.aggregate("ms", "avg", { "group_by": ["country"] });
        assert(len(rows) >= 2);
        assert(rows[0].has_key("country"));
        assert(rows[0].has_key("value"));
        # Known server quirk: grouped string keys may come back JSON-quoted
        # ("\"grp_..._fr\""), so match with contains and assert on the
        # aggregate values.
        let fr_rows = rows.filter(fn(r) str(r["country"]).contains(tag + "_fr"));
        assert_eq(len(fr_rows), 1);
        assert(fr_rows[0]["value"] > 99.9);
        assert(fr_rows[0]["value"] < 100.1);
        let de_rows = rows.filter(fn(r) str(r["country"]).contains(tag + "_de"));
        assert_eq(len(de_rows), 1);
        assert(de_rows[0]["value"] > 199.9);
        assert(de_rows[0]["value"] < 200.1);
    });

    test("query projects columns with eq and gt filters", fn() {
        if !__db_available
            return;
        end
        let tag = "qry_" + __run_id;
        insert_rows_for(tag);
        let fr_rows = ColTestView.query({
            "columns": ["url", "ms"],
            "filter": { "column": "country", "op": "eq", "value": tag + "_fr" },
            "limit": 10
        });
        assert_eq(len(fr_rows), 2);
        assert(fr_rows[0].has_key("url"));
        assert(fr_rows[0].has_key("ms"));

        # gt scans the whole store (single-filter endpoint, no tag conjunct),
        # so with accumulated rows only a lower bound is exact: this test
        # alone contributes two ms > 100 rows (120 and 200).
        let slow_rows = ColTestView.query({
            "columns": ["url"],
            "filter": { "column": "ms", "op": "gt", "value": 100 }
        });
        assert(len(slow_rows) >= 2);
    });

    test("count goes through the columnar engine", fn() {
        if !__db_available
            return;
        end
        let count_before = ColTestView.count;
        insert_rows_for("cnt_" + __run_id);
        assert_eq(ColTestView.count, count_before + 3);
    });

    test("column index lifecycle: add, list, drop", fn() {
        if !__db_available
            return;
        end
        # `url` is NOT schema-indexed, so the full lifecycle runs on it.
        # A leftover index from a crashed earlier run would break add, so
        # clear it first.
        ColTestView.drop_column_index("url") rescue null;
        ColTestView.add_column_index("url", "bitmap");
        let indexes = ColTestView.column_indexes();
        assert(indexes.to_json.contains("url"));
        ColTestView.drop_column_index("url");
    });

    test("schema-declared indexed column already has an index", fn() {
        if !__db_available
            return;
        end
        # `country` is declared `indexed: true`, so the store already
        # carries its index — a second add is rejected.
        let msg = "";
        try
            ColTestView.add_column_index("country", "bitmap");
        catch e
            msg = str(e);
        end
        assert(msg.contains("already has an index"));
    });
});
