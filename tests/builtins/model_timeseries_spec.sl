# ============================================================================
# Model Timeseries Test Suite
# Tests for the `timeseries` declaration: insert-only enforcement,
# time_bucket() aggregation, and prune() retention.
# ============================================================================

class TsTestMetric extends Model
    timeseries retention: "30d"
end

class TsTestReading extends Model
    timeseries retention: "90d", timestamp: "recorded_at"
end

class TsBareEvent extends Model
    timeseries
end

# Control model WITHOUT a timeseries declaration.
class TsTestPlain extends Model
end

# Detect DB availability
let __db_available = false;
try
    let __probe = TsTestMetric.create({ "device": "__probe__", "value": 0 });
    if !__probe.nil? and !__probe._errors
        __db_available = true;
        __probe.delete();
    end
catch e
end

# ============================================================================
# Tests that do NOT require a DB connection
# ============================================================================

describe("time_bucket() query generation", fn() {
    test("static form buckets on _created_at with the aggregate", fn() {
        let q = TsTestMetric.time_bucket("1h", { "avg": "value" }).to_query;
        assert(q.contains("TIME_BUCKET(doc._created_at, \"1h\")"));
        assert(q.contains("AGGREGATE avg = AVG(doc.value)"));
        assert(q.contains("SORT bucket"));
    });

    test("chains after where() and keeps the filter", fn() {
        let q = TsTestMetric.where("device = @d", { "d": "srv1" })
            .time_bucket("5m", { "avg": "value", "max": "value" })
            .to_query;
        assert(q.contains("FILTER doc.device == @d"));
        assert(q.contains("avg = AVG(doc.value)"));
        assert(q.contains("max = MAX(doc.value)"));
        assert(q.contains("TIME_BUCKET(doc._created_at, \"5m\")"));
    });

    test("keyword style works like the hash form", fn() {
        let q = TsTestMetric.time_bucket("1h", avg: "value").to_query;
        assert(q.contains("AGGREGATE avg = AVG(doc.value)"));
    });

    test("declared timestamp: field replaces _created_at", fn() {
        let q = TsTestReading.time_bucket("5m", { "avg": "value" }).to_query;
        assert(q.contains("TIME_BUCKET(doc.recorded_at, \"5m\")"));
    });

    test("bare time_bucket counts rows per bucket", fn() {
        let q = TsBareEvent.time_bucket("1d").to_query;
        assert(q.contains("TIME_BUCKET(doc._created_at, \"1d\")"));
        assert(q.contains("count = COUNT()"));
    });

    test("count: true aggregate is explicit COUNT", fn() {
        let q = TsTestMetric.time_bucket("1d", count: true).to_query;
        assert(q.contains("count = COUNT()"));
    });
});

describe("time_bucket() validation", fn() {
    test("invalid interval unit raises", fn() {
        let msg = "";
        try
            TsTestMetric.time_bucket("5x");
        catch e
            msg = str(e);
        end
        assert(msg.contains("invalid interval"));
    });

    test("zero interval raises", fn() {
        let msg = "";
        try
            TsTestMetric.time_bucket("0m", { "avg": "value" });
        catch e
            msg = str(e);
        end
        assert(msg.contains("invalid interval"));
    });

    test("unknown aggregate raises", fn() {
        let msg = "";
        try
            TsTestMetric.time_bucket("1h", { "median": "value" });
        catch e
            msg = str(e);
        end
        assert(msg.contains("unknown aggregate"));
    });

    test("non-timeseries model raises on static time_bucket", fn() {
        let msg = "";
        try
            TsTestPlain.time_bucket("1h", { "avg": "value" });
        catch e
            msg = str(e);
        end
        assert(msg.contains("requires a `timeseries` declaration"));
    });
});

describe("Insert-only enforcement (no DB round trip)", fn() {
    test("static update raises insert-only", fn() {
        let msg = "";
        try
            TsTestMetric.update("some_key", { "value": 1 });
        catch e
            msg = str(e);
        end
        assert(msg.contains("insert-only"));
    });

    test("static upsert raises insert-only", fn() {
        let msg = "";
        try
            TsTestMetric.upsert("some_key", { "value": 1 });
        catch e
            msg = str(e);
        end
        assert(msg.contains("insert-only"));
    });

    test("instance update raises insert-only", fn() {
        let metric = TsTestMetric.new({ "device": "srv1", "value": 1 });
        let msg = "";
        try
            metric.update({ "value": 2 });
        catch e
            msg = str(e);
        end
        assert(msg.contains("insert-only"));
    });

    test("increment raises insert-only", fn() {
        let metric = TsTestMetric.new({ "device": "srv1", "value": 1 });
        let msg = "";
        try
            metric.increment("value");
        catch e
            msg = str(e);
        end
        assert(msg.contains("insert-only"));
    });

    test("update_all through a where() chain raises insert-only", fn() {
        let msg = "";
        try
            TsTestMetric.where({ "device": "srv1" }).update_all({ "value": 0 });
        catch e
            msg = str(e);
        end
        assert(msg.contains("insert-only"));
    });
});

describe("prune() argument validation", fn() {
    test("non-timeseries model raises", fn() {
        let msg = "";
        try
            TsTestPlain.prune;
        catch e
            msg = str(e);
        end
        assert(msg.contains("requires a `timeseries` declaration"));
    });

    test("garbage argument raises mentioning duration/RFC3339", fn() {
        let msg = "";
        try
            TsTestMetric.prune("not-a-date");
        catch e
            msg = str(e);
        end
        assert(msg.contains("RFC3339"));
        assert(msg.contains("duration"));
    });

    test("bare timeseries model without retention needs an argument", fn() {
        let msg = "";
        try
            TsBareEvent.prune;
        catch e
            msg = str(e);
        end
        assert(msg.contains("requires an argument or a retention:"));
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

describe("Timeseries create/delete (DB)", fn() {
    before_each(fn() {
        TsTestMetric.delete_all() rescue null;
    });

    test("create works normally and returns a persisted instance", fn() {
        if !__db_available
            return;
        end
        let metric = TsTestMetric.create({ "device": "srv1", "value": 0.5 });
        assert_null(metric._errors);
        assert_not_null(metric._key);

        let reloaded = TsTestMetric.find(metric._key);
        assert_eq(reloaded.device, "srv1");
    });

    test("save on a persisted record raises insert-only", fn() {
        if !__db_available
            return;
        end
        let metric = TsTestMetric.create({ "device": "srv1", "value": 1 });
        assert_not_null(metric._key);

        let msg = "";
        try
            metric.save();
        catch e
            msg = str(e);
        end
        assert(msg.contains("insert-only"));
    });

    test("instance delete still works", fn() {
        if !__db_available
            return;
        end
        let metric = TsTestMetric.create({ "device": "gone", "value": 1 });
        metric.delete();
        assert_eq(TsTestMetric.where({ "device": "gone" }).count, 0);
    });

    test("static delete still works", fn() {
        if !__db_available
            return;
        end
        let metric = TsTestMetric.create({ "device": "gone2", "value": 1 });
        TsTestMetric.delete(metric._key);
        assert_eq(TsTestMetric.where({ "device": "gone2" }).count, 0);
    });
});

describe("time_bucket() execution (DB)", fn() {
    before_each(fn() {
        TsTestMetric.delete_all() rescue null;
        TsTestReading.delete_all() rescue null;
    });

    test("aggregates seeded docs into bucket rows", fn() {
        if !__db_available
            return;
        end
        TsTestMetric.create({ "device": "srv1", "value": 10 });
        TsTestMetric.create({ "device": "srv1", "value": 20 });
        TsTestMetric.create({ "device": "srv1", "value": 30 });

        # A 1d bucket keeps all fresh docs in one (very rarely two) buckets;
        # summing over rows makes the assertions deterministic either way.
        let rows = TsTestMetric.time_bucket("1d", { "avg": "value", "count": true }).all;
        assert(len(rows) >= 1);
        assert(rows[0].has_key("bucket"));
        assert_not_null(rows[0]["bucket"]);
        assert(rows[0].has_key("avg"));

        let total = 0;
        for row in rows
            total = total + row["count"];
        end
        assert_eq(total, 3);
    });

    test("where() chain restricts the bucketed docs", fn() {
        if !__db_available
            return;
        end
        TsTestMetric.create({ "device": "srv1", "value": 10 });
        TsTestMetric.create({ "device": "srv1", "value": 20 });
        TsTestMetric.create({ "device": "srv2", "value": 99 });

        let rows = TsTestMetric.where("device = @d", { "d": "srv1" })
            .time_bucket("1d", { "max": "value", "count": true })
            .all;

        let total = 0;
        let max_seen = 0;
        for row in rows
            total = total + row["count"];
            if row["max"] > max_seen
                max_seen = row["max"];
            end
        end
        assert_eq(total, 2);
        assert_eq(max_seen, 20);
    });

    test("bare time_bucket returns count per bucket", fn() {
        if !__db_available
            return;
        end
        TsTestMetric.create({ "device": "srv1", "value": 1 });
        TsTestMetric.create({ "device": "srv1", "value": 2 });

        let rows = TsTestMetric.time_bucket("1d").all;
        let total = 0;
        for row in rows
            total = total + row["count"];
        end
        assert_eq(total, 2);
    });

    test("declared timestamp: buckets on the custom field", fn() {
        if !__db_available
            return;
        end
        # recorded_at drives the buckets, so historical timestamps make the
        # bucket layout fully deterministic: two docs in the 10:00 hour,
        # one in the 11:00 hour.
        TsTestReading.create({ "sensor": "s1", "value": 1, "recorded_at": "2024-01-15T10:05:00Z" });
        TsTestReading.create({ "sensor": "s1", "value": 2, "recorded_at": "2024-01-15T10:25:00Z" });
        TsTestReading.create({ "sensor": "s1", "value": 3, "recorded_at": "2024-01-15T11:05:00Z" });

        let rows = TsTestReading.time_bucket("1h", { "count": true }).all;
        assert_eq(len(rows), 2);
        # SORT bucket → the 10:00 bucket (2 docs) comes before 11:00 (1 doc).
        assert_eq(rows[0]["count"], 2);
        assert_eq(rows[1]["count"], 1);
    });
});

describe("prune() execution (DB)", fn() {
    before_each(fn() {
        TsTestMetric.delete_all() rescue null;
    });

    test("future ISO cutoff deletes all seeded docs", fn() {
        if !__db_available
            return;
        end
        TsTestMetric.create({ "device": "old", "value": 1 });
        TsTestMetric.create({ "device": "old", "value": 2 });
        TsTestMetric.create({ "device": "old", "value": 3 });

        let deleted = TsTestMetric.prune("2100-01-01T00:00:00Z");
        assert(deleted >= 3);
        # NOTE: post-prune assertions use len(.all) instead of .count —
        # COLLECTION_COUNT's O(1) metadata undercounts after the prune
        # endpoint (server-side quirk); real queries see the true state.
        assert_eq(len(TsTestMetric.all), 0);
    });

    test("duration cutoff only deletes docs older than the window", fn() {
        if !__db_available
            return;
        end
        TsTestMetric.create({ "device": "old", "value": 1 });
        TsTestMetric.create({ "device": "old", "value": 2 });
        sleep(1.1);
        TsTestMetric.create({ "device": "fresh", "value": 3 });

        let deleted = TsTestMetric.prune("1s");
        assert_eq(deleted, 2);
        let survivors = TsTestMetric.all;
        assert_eq(len(survivors), 1);
        assert_eq(survivors[0].device, "fresh");
    });

    test("no argument uses the declared retention", fn() {
        if !__db_available
            return;
        end
        TsTestMetric.create({ "device": "fresh", "value": 1 });

        # retention: "30d" — fresh data survives, and the call returns an Int.
        let deleted = TsTestMetric.prune;
        assert_eq(deleted, 0);
        assert_eq(len(TsTestMetric.all), 1);
    });
});
