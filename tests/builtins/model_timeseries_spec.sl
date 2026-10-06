# The `timeseries` model declaration: insert-only enforcement, time_bucket()
# aggregation, and prune() retention.

class TsTestMetric < Model
  timeseries(retention: "30d")
end

class TsTestReading < Model
  timeseries(retention: "90d", timestamp: "recorded_at")
end

class TsBareEvent < Model
  timeseries
end

# Control model WITHOUT a timeseries declaration.
class TsTestPlain < Model
end

INSERT_ONLY = "TsTestMetric is a timeseries model: records are insert-only."

# Sum one column over every bucket row: fresh docs are stamped by the server
# with the current time, so a 1d bucket holds them all except on the rare run
# that straddles midnight UTC — a total is the same either way.
def sum_of(values)
  values.reduce(fn(total, value) { total + value }, 0)
end

def column_total(rows, column)
  sum_of(rows.map { |row| row[column] })
end

describe("time_bucket() query generation") do
  test("the static form buckets on _created_at with the aggregate") do
    assert_eq(TsTestMetric.time_bucket("1h", {"avg": "value"}).to_query,
      "FOR doc IN ts_test_metrics COLLECT bucket = TIME_BUCKET(doc._created_at, \"1h\") " +
      "AGGREGATE avg = AVG(doc.value) SORT bucket RETURN {bucket: bucket, avg: avg}")
  end

  test("chains after where() and keeps the filter") do
    query = TsTestMetric.where("device = @d", {"d": "srv1"})
      .time_bucket("5m", {"avg": "value", "max": "value"})
      .to_query

    assert_match(query, "^FOR doc IN ts_test_metrics FILTER doc.device == @d COLLECT bucket = " +
      "TIME_BUCKET\\(doc._created_at, \"5m\"\\) AGGREGATE avg = AVG\\(doc.value\\), max = MAX\\(doc.value\\) ")
  end

  test("the keyword style builds the same query as the hash form") do
    assert_eq(TsTestMetric.time_bucket("1h", avg: "value").to_query,
      TsTestMetric.time_bucket("1h", {"avg": "value"}).to_query)
  end

  test("a declared timestamp: field replaces _created_at") do
    assert_contains(TsTestReading.time_bucket("5m", {"avg": "value"}).to_query,
      "COLLECT bucket = TIME_BUCKET(doc.recorded_at, \"5m\")")
  end

  test("a bare time_bucket counts rows per bucket") do
    assert_eq(TsBareEvent.time_bucket("1d").to_query,
      "FOR doc IN ts_bare_events COLLECT bucket = TIME_BUCKET(doc._created_at, \"1d\") " +
      "AGGREGATE count = COUNT() SORT bucket RETURN {bucket: bucket, count: count}")
  end

  test("count: true is an explicit COUNT") do
    assert_contains(TsTestMetric.time_bucket("1d", count: true).to_query, "AGGREGATE count = COUNT()")
  end

  test("min and sum map to MIN and SUM") do
    assert_contains(TsTestMetric.time_bucket("1d", {"min": "value", "sum": "value"}).to_query,
      "AGGREGATE min = MIN(doc.value), sum = SUM(doc.value)")
  end
end

describe("time_bucket() validation") do
  test("an invalid interval unit raises") do
    assert_raises("time_bucket() invalid interval \"5x\": expected <number><unit> with unit s/m/h/d") do
      TsTestMetric.time_bucket("5x")
    end
  end

  test("a zero interval raises") do
    assert_raises("time_bucket() invalid interval \"0m\"") do
      TsTestMetric.time_bucket("0m", {"avg": "value"})
    end
  end

  test("an unknown aggregate raises") do
    assert_raises("unknown aggregate 'median': expected sum, avg, min, max, or count") do
      TsTestMetric.time_bucket("1h", {"median": "value"})
    end
  end

  test("a non-timeseries model raises on the static time_bucket") do
    assert_raises("TsTestPlain.time_bucket() requires a `timeseries` declaration") do
      TsTestPlain.time_bucket("1h", {"avg": "value"})
    end
  end
end

describe("Insert-only enforcement (no DB round trip)") do
  test("static update raises insert-only") do
    assert_raises("#{INSERT_ONLY} update is not supported — use prune() for retention.") do
      TsTestMetric.update("some_key", {"value": 1})
    end
  end

  test("static upsert raises insert-only") do
    assert_raises("#{INSERT_ONLY} upsert is not supported") do
      TsTestMetric.upsert("some_key", {"value": 1})
    end
  end

  test("instance update raises insert-only") do
    metric = TsTestMetric.new({"device": "srv1", "value": 1})

    assert_raises("#{INSERT_ONLY} update is not supported") do
      metric.update({"value": 2})
    end
  end

  test("increment raises insert-only") do
    metric = TsTestMetric.new({"device": "srv1", "value": 1})

    assert_raises("#{INSERT_ONLY} increment is not supported") do
      metric.increment("value")
    end
  end

  test("update_all through a where() chain raises insert-only") do
    assert_raises("#{INSERT_ONLY} update_all is not supported") do
      TsTestMetric.where({"device": "srv1"}).update_all({"value": 0})
    end
  end
end

describe("prune() argument validation") do
  test("a non-timeseries model raises") do
    assert_raises("TsTestPlain.prune() requires a `timeseries` declaration") do
      TsTestPlain.prune
    end
  end

  test("a garbage argument raises naming both accepted forms") do
    assert_raises("expects a duration (\"30d\") or an RFC3339 timestamp, got \"not-a-date\"") do
      TsTestMetric.prune("not-a-date")
    end
  end

  test("a bare timeseries model without retention needs an argument") do
    assert_raises("TsBareEvent.prune requires an argument or a retention: declaration") do
      TsBareEvent.prune
    end
  end
end

describe("Timeseries against the database") do
  before_each() do
    requires_solidb()
  end

  after_each() do
    TsTestMetric.delete_all()
    TsTestReading.delete_all()
  end

  describe("create and delete") do
    test("create works normally and returns a persisted instance") do
      metric = TsTestMetric.create({"device": "srv1", "value": 0.5})

      assert_null(metric._errors)
      reloaded = TsTestMetric.find(metric._key)
      assert_eq(reloaded.device, "srv1")
      assert_eq(reloaded.value, 0.5)
    end

    test("save on a persisted record raises insert-only") do
      metric = TsTestMetric.create({"device": "srv1", "value": 1})

      assert_raises("#{INSERT_ONLY} save is not supported") do
        metric.save
      end
    end

    test("instance delete still works") do
      metric = TsTestMetric.create({"device": "gone", "value": 1})
      metric.delete

      assert_eq(TsTestMetric.where({"device": "gone"}).count, 0)
    end

    test("static delete still works") do
      metric = TsTestMetric.create({"device": "gone2", "value": 1})
      TsTestMetric.delete(metric._key)

      assert_eq(TsTestMetric.where({"device": "gone2"}).count, 0)
    end
  end

  describe("time_bucket() execution") do
    test("aggregates the seeded docs into bucket rows") do
      [10, 20, 30].each do |value|
        TsTestMetric.create({"device": "srv1", "value": value})
      end

      rows = TsTestMetric.time_bucket("1d", {"avg": "value", "count": true}).all

      assert_eq(rows[0].keys.sort(), ["avg", "bucket", "count"])
      assert_match(rows[0]["bucket"], "^\\d{4}-\\d{2}-\\d{2}T00:00:00")
      assert_eq(column_total(rows, "count"), 3)
      # The count-weighted average over every bucket is the plain average.
      assert_eq(sum_of(rows.map { |row| row["avg"] * row["count"] }) / 3, 20)
    end

    test("a where() chain restricts the bucketed docs") do
      TsTestMetric.create({"device": "srv1", "value": 10})
      TsTestMetric.create({"device": "srv1", "value": 20})
      TsTestMetric.create({"device": "srv2", "value": 99})

      rows = TsTestMetric.where("device = @d", {"d": "srv1"})
        .time_bucket("1d", {"max": "value", "count": true})
        .all

      assert_eq(column_total(rows, "count"), 2)
      assert_eq(rows.map { |row| row["max"] }.max, 20)
    end

    test("a bare time_bucket returns the count per bucket") do
      TsTestMetric.create({"device": "srv1", "value": 1})
      TsTestMetric.create({"device": "srv1", "value": 2})

      assert_eq(column_total(TsTestMetric.time_bucket("1d").all, "count"), 2)
    end

    test("a declared timestamp: buckets on the custom field") do
      # recorded_at drives the buckets, so historical timestamps make the
      # layout fully deterministic: two docs at 10:00, one at 11:00.
      ["2024-01-15T10:05:00Z", "2024-01-15T10:25:00Z", "2024-01-15T11:05:00Z"].each do |recorded_at|
        TsTestReading.create({"sensor": "s1", "value": 1, "recorded_at": recorded_at})
      end

      rows = TsTestReading.time_bucket("1h", {"count": true}).all

      assert_eq(rows.map { |row| row["count"] }, [2, 1])
      assert_match(rows[0]["bucket"], "^2024-01-15T10:00:00")
      assert_match(rows[1]["bucket"], "^2024-01-15T11:00:00")
    end
  end

  describe("prune() execution") do
    # Post-prune checks use `.all.length` rather than `.count`: COLLECTION_COUNT's
    # O(1) metadata has undercounted after the prune endpoint.
    test("a future RFC3339 cutoff deletes every doc") do
      3.times do |i|
        TsTestMetric.create({"device": "old", "value": i})
      end

      assert_eq(TsTestMetric.prune("2100-01-01T00:00:00Z"), 3)
      assert_eq(TsTestMetric.all.length, 0)
    end

    test("a past RFC3339 cutoff deletes nothing") do
      TsTestMetric.create({"device": "fresh", "value": 1})

      assert_eq(TsTestMetric.prune("2000-01-01T00:00:00Z"), 0)
      assert_eq(TsTestMetric.all.length, 1)
    end

    test("a duration cutoff only deletes docs older than the window") do
      # _created_at is stamped by the server, so freeze_time cannot age a doc:
      # this one test waits out the window on the wall clock.
      TsTestMetric.create({"device": "old", "value": 1})
      TsTestMetric.create({"device": "old", "value": 2})
      sleep(1.1)
      TsTestMetric.create({"device": "fresh", "value": 3})

      assert_eq(TsTestMetric.prune("1s"), 2)
      assert_eq(TsTestMetric.all.map { |metric| metric.device }, ["fresh"])
    end

    test("no argument uses the declared retention") do
      TsTestMetric.create({"device": "fresh", "value": 1})

      # retention: "30d" — fresh data survives, and the call returns an Int.
      assert_eq(TsTestMetric.prune, 0)
      assert_eq(TsTestMetric.all.length, 1)
    end
  end
end
