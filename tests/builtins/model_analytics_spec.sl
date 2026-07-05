# ============================================================================
# Model Analytics (grouped aggregation) Test Suite
# Tests for group_by()/aggregate()/having()/order()/limit() grouped chains,
# aggregate terminals (median/stddev/count_distinct), the legacy 3-arg
# group_by shape, and the soft-delete scope in grouped queries.
# ============================================================================

class AnalyticsOrder extends Model
end

# Soft-delete model: grouped queries must exclude soft-deleted rows.
class AnalyticsSale extends Model
    soft_delete
end

# Detect DB availability
let __db_available = false;
try
    let __probe = AnalyticsOrder.create({ "status": "__probe__" });
    if !__probe.nil? and !__probe._errors
        __db_available = true;
        __probe.delete();
    end
catch e
end

# Standard order set. Top-level (not describe-scope) because test closures
# don't see describe-scope definitions. The pending order is the one that
# a where(status: paid) chain must exclude.
fn seed_orders() {
    AnalyticsOrder.create({ "country": "FR", "plan": "basic", "amount": 10, "status": "paid" });
    AnalyticsOrder.create({ "country": "FR", "plan": "basic", "amount": 20, "status": "paid" });
    AnalyticsOrder.create({ "country": "FR", "plan": "pro", "amount": 40, "status": "paid" });
    AnalyticsOrder.create({ "country": "DE", "plan": "basic", "amount": 100, "status": "paid" });
    AnalyticsOrder.create({ "country": "FR", "plan": "basic", "amount": 999, "status": "pending" });
}

# ============================================================================
# Tests that do NOT require a DB connection
# ============================================================================

describe("Grouped query generation (no DB)", fn() {
    test("full chain emits COLLECT, AGGREGATE, post-COLLECT FILTER, SORT, LIMIT", fn() {
        let q = AnalyticsOrder.where({ "status": "paid" })
            .group_by(["country", "plan"])
            .aggregate({ "total": ["sum", "amount"] })
            .having("total > @min", { "min": 50 })
            .order("total", "desc")
            .limit(20)
            .to_query;
        assert(q.contains("COLLECT country = doc.country, plan = doc.plan"));
        assert(q.contains("AGGREGATE total = SUM(doc.amount)"));
        assert(q.contains("FILTER total > @min"));
        # The having-FILTER must come after the COLLECT (post-grouping filter).
        assert(q.index_of("FILTER total > @min") > q.index_of("COLLECT country"));
        assert(q.contains("SORT total DESC"));
        assert(q.contains("LIMIT 20"));
    });

    test("1-arg group_by without aggregates counts implicitly", fn() {
        let q = AnalyticsOrder.group_by("country").to_query;
        assert(q.contains("COLLECT country = doc.country"));
        assert(q.contains("AGGREGATE n = COUNT()"));
        assert(q.contains("RETURN {country: country, n: n}"));
    });

    test("ungrouped aggregate emits a bare COLLECT AGGREGATE", fn() {
        let q = AnalyticsOrder.aggregate({ "total": ["sum", "amount"] }).to_query;
        assert(q.contains("COLLECT AGGREGATE total = SUM(doc.amount)"));
        assert(q.contains("RETURN {total: total}"));
    });

    test("median in a grouped chain goes through COLLECT_LIST", fn() {
        let q = AnalyticsOrder.group_by("country")
            .aggregate({ "med": ["median", "amount"] })
            .to_query;
        assert(q.contains("__soli_vals_med = COLLECT_LIST(doc.amount)"));
        assert(q.contains("med: MEDIAN(__soli_vals_med)"));
    });

    test("legacy 3-arg group_by emission is unchanged", fn() {
        let q = AnalyticsOrder.group_by("country", "sum", "amount").to_query;
        assert_eq(q, "FOR doc IN analytics_orders COLLECT group = doc.country "
            + "AGGREGATE result = SUM(doc.amount) RETURN {group: group, result: result}");
    });

    test("soft-delete model grouped query filters deleted rows", fn() {
        let q = AnalyticsSale.group_by("country").to_query;
        assert(q.contains("FILTER doc.deleted_at == null"));
        assert(q.index_of("FILTER doc.deleted_at == null") < q.index_of("COLLECT country"));
    });
});

describe("Grouped chain validation (no DB)", fn() {
    test("order() must name a group field or aggregate alias", fn() {
        let msg = "";
        try
            AnalyticsOrder.group_by("country").order("amount", "desc").to_query;
        catch e
            msg = str(e);
        end
        assert(msg.contains("group field or aggregate alias"));
    });

    test("percentile is rejected with a clear message", fn() {
        let msg = "";
        try
            AnalyticsOrder.aggregate({ "p95": ["percentile", "amount"] });
        catch e
            msg = str(e);
        end
        assert(msg.contains("percentile"));
        assert(msg.contains("not supported"));
    });

    test("unknown aggregate function raises", fn() {
        let msg = "";
        try
            AnalyticsOrder.aggregate({ "x": ["frobnicate", "amount"] });
        catch e
            msg = str(e);
        end
        assert(msg.contains("unknown function"));
    });

    test("having() requires grouping earlier in the chain", fn() {
        let msg = "";
        try
            AnalyticsOrder.where({ "status": "paid" }).having("total > 1");
        catch e
            msg = str(e);
        end
        assert(msg.contains("requires group_by()/aggregate()"));
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

describe("Grouped execution (DB)", fn() {
    before_each(fn() {
        AnalyticsOrder.delete_all() rescue null;
    });

    test("multi-key grouping with aggregates, having, order and limit", fn() {
        if !__db_available
            return;
        end
        seed_orders();
        let rows = AnalyticsOrder.where({ "status": "paid" })
            .group_by(["country", "plan"])
            .aggregate({ "total": ["sum", "amount"], "n": ["count"] })
            .having("total > @min", { "min": 25 })
            .order("total", "desc")
            .limit(20)
            .all;
        # paid groups: FR/basic 30 (2 rows), FR/pro 40, DE/basic 100 — all > 25.
        assert_eq(len(rows), 3);
        assert_eq(rows[0]["country"], "DE");
        assert_eq(rows[0]["plan"], "basic");
        assert_eq(rows[0]["total"], 100);
        assert_eq(rows[0]["n"], 1);
        assert_eq(rows[1]["country"], "FR");
        assert_eq(rows[1]["plan"], "pro");
        assert_eq(rows[1]["total"], 40);
        assert_eq(rows[2]["country"], "FR");
        assert_eq(rows[2]["plan"], "basic");
        assert_eq(rows[2]["total"], 30);
        assert_eq(rows[2]["n"], 2);
    });

    test("having drops groups below the threshold", fn() {
        if !__db_available
            return;
        end
        seed_orders();
        let rows = AnalyticsOrder.where({ "status": "paid" })
            .group_by(["country", "plan"])
            .aggregate({ "total": ["sum", "amount"] })
            .having("total > @min", { "min": 50 })
            .all;
        # Only DE/basic (100) clears the 50 bar.
        assert_eq(len(rows), 1);
        assert_eq(rows[0]["country"], "DE");
        assert_eq(rows[0]["total"], 100);
    });

    test("1-arg group_by returns implicit-count rows", fn() {
        if !__db_available
            return;
        end
        seed_orders();
        let rows = AnalyticsOrder.group_by("country").all;
        assert_eq(len(rows), 2);
        let fr = rows.filter(fn(r) r["country"] == "FR")[0];
        assert_eq(fr["n"], 4);
        let de = rows.filter(fn(r) r["country"] == "DE")[0];
        assert_eq(de["n"], 1);
    });

    test("ungrouped aggregate .first returns a hash with all aliases", fn() {
        if !__db_available
            return;
        end
        seed_orders();
        let row = AnalyticsOrder.where({ "status": "paid" })
            .aggregate({ "total": ["sum", "amount"], "n": ["count"], "avg_amount": ["avg", "amount"] })
            .first;
        assert_eq(row["total"], 170);
        assert_eq(row["n"], 4);
        assert_eq(row["avg_amount"], 42.5);
    });

    test("median/stddev/count_distinct terminals unwrap with .first", fn() {
        if !__db_available
            return;
        end
        AnalyticsOrder.create({ "country": "FR", "amount": 10 });
        AnalyticsOrder.create({ "country": "FR", "amount": 20 });
        AnalyticsOrder.create({ "country": "DE", "amount": 30 });
        AnalyticsOrder.create({ "country": "US", "amount": 40 });

        let med = AnalyticsOrder.median("amount").first;
        assert(med > 24.9);
        assert(med < 25.1);

        # Population (11.18) vs sample (12.91) stddev both land in this range.
        let sd = AnalyticsOrder.stddev("amount").first;
        assert(sd > 11.0);
        assert(sd < 13.0);

        let distinct = AnalyticsOrder.count_distinct("country").first;
        assert_eq(distinct, 3);
    });

    test("legacy 3-arg group_by rows keep the {group, result} shape", fn() {
        if !__db_available
            return;
        end
        AnalyticsOrder.create({ "country": "FR", "amount": 10 });
        AnalyticsOrder.create({ "country": "FR", "amount": 20 });
        AnalyticsOrder.create({ "country": "DE", "amount": 5 });

        let rows = AnalyticsOrder.group_by("country", "sum", "amount").all;
        assert_eq(len(rows), 2);
        let fr = rows.filter(fn(r) r["group"] == "FR")[0];
        assert(fr.has_key("group"));
        assert(fr.has_key("result"));
        assert_eq(fr["result"], 30);
        let de = rows.filter(fn(r) r["group"] == "DE")[0];
        assert_eq(de["result"], 5);
    });
});

describe("Soft-delete grouped execution (DB)", fn() {
    before_each(fn() {
        AnalyticsSale.delete_all() rescue null;
    });

    test("grouped query excludes soft-deleted rows", fn() {
        if !__db_available
            return;
        end
        let doomed = AnalyticsSale.create({ "country": "FR", "amount": 10 });
        AnalyticsSale.create({ "country": "FR", "amount": 20 });
        AnalyticsSale.create({ "country": "DE", "amount": 30 });
        doomed.delete();

        let rows = AnalyticsSale.group_by("country").all;
        assert_eq(len(rows), 2);
        let fr = rows.filter(fn(r) r["country"] == "FR")[0];
        assert_eq(fr["n"], 1);
        let de = rows.filter(fn(r) r["country"] == "DE")[0];
        assert_eq(de["n"], 1);
    });
});
