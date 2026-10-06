# Grouped aggregation: group_by()/aggregate()/having()/order()/limit() chains,
# the median/stddev/count_distinct terminals, the legacy 3-arg group_by shape,
# and the soft-delete scope in grouped queries.

class AnalyticsOrder < Model
end

# Grouped queries on a soft-delete model must exclude soft-deleted rows.
class AnalyticsSale < Model
  soft_delete
end

# The pending order is the one a where(status: paid) chain must exclude.
def seed_orders
  [
    {"country": "FR", "plan": "basic", "amount": 10, "status": "paid"},
    {"country": "FR", "plan": "basic", "amount": 20, "status": "paid"},
    {"country": "FR", "plan": "pro", "amount": 40, "status": "paid"},
    {"country": "DE", "plan": "basic", "amount": 100, "status": "paid"},
    {"country": "FR", "plan": "basic", "amount": 999, "status": "pending"}
  ].each do |order|
    AnalyticsOrder.create(order)
  end
end

def seed_amounts
  AnalyticsOrder.create({"country": "FR", "amount": 10})
  AnalyticsOrder.create({"country": "FR", "amount": 20})
  AnalyticsOrder.create({"country": "DE", "amount": 30})
  AnalyticsOrder.create({"country": "US", "amount": 40})
end

def row_for(rows, field, value)
  rows.filter { |row| row[field] == value }[0]
end

describe("Grouped query generation (no DB)") do
  test("full chain emits COLLECT, AGGREGATE, post-COLLECT FILTER, SORT, LIMIT") do
    query = AnalyticsOrder.where({"status": "paid"})
      .group_by(["country", "plan"])
      .aggregate({"total": ["sum", "amount"]})
      .having("total > @min", {"min": 50})
      .order("total", "desc")
      .limit(20)
      .to_query

    # The having-FILTER comes after the COLLECT: it filters groups, not rows.
    assert_contains(
      query,
      "FOR doc IN analytics_orders FILTER doc.status == @status__eq_1 "
      + "COLLECT country = doc.country, plan = doc.plan AGGREGATE total = SUM(doc.amount) "
      + "FILTER total > @min SORT total DESC LIMIT 20 RETURN {country: country, plan: plan, total: total}"
    )
  end

  test("1-arg group_by without aggregates counts implicitly") do
    assert_eq(
      AnalyticsOrder.group_by("country").to_query,
      "FOR doc IN analytics_orders COLLECT country = doc.country AGGREGATE n = COUNT() RETURN {country: country, n: n}"
    )
  end

  test("ungrouped aggregate emits a bare COLLECT AGGREGATE") do
    assert_eq(
      AnalyticsOrder.aggregate({"total": ["sum", "amount"]}).to_query,
      "FOR doc IN analytics_orders COLLECT AGGREGATE total = SUM(doc.amount) RETURN {total: total}"
    )
  end

  test("median in a grouped chain goes through COLLECT_LIST") do
    query = AnalyticsOrder.group_by("country").aggregate({"med": ["median", "amount"]}).to_query

    assert_contains(query, "AGGREGATE __soli_vals_med = COLLECT_LIST(doc.amount)")
    assert_contains(query, "RETURN {country: country, med: MEDIAN(__soli_vals_med)}")
  end

  test("legacy 3-arg group_by emission is unchanged") do
    assert_eq(
      AnalyticsOrder.group_by("country", "sum", "amount").to_query,
      "FOR doc IN analytics_orders COLLECT group = doc.country "
      + "AGGREGATE result = SUM(doc.amount) RETURN {group: group, result: result}"
    )
  end

  test("soft-delete model grouped query filters deleted rows before grouping") do
    assert_eq(
      AnalyticsSale.group_by("country").to_query,
      "FOR doc IN analytics_sales FILTER doc.deleted_at == null "
      + "COLLECT country = doc.country AGGREGATE n = COUNT() RETURN {country: country, n: n}"
    )
  end
end

describe("Grouped chain validation (no DB)") do
  test("order() must name a group field or aggregate alias") do
    message = assert_raises("must name a group field or aggregate alias") do
      AnalyticsOrder.group_by("country").order("amount", "desc").to_query
    end

    assert_contains(message, "(have: country, n)")
  end

  test("percentile is rejected with a clear message") do
    assert_raises("aggregate(): percentile is not supported by SolidB") do
      AnalyticsOrder.aggregate({"p95": ["percentile", "amount"]})
    end
  end

  test("unknown aggregate function raises") do
    assert_raises("aggregate() unknown function 'frobnicate'") do
      AnalyticsOrder.aggregate({"x": ["frobnicate", "amount"]})
    end
  end

  test("having() requires grouping earlier in the chain") do
    assert_raises("having() requires group_by()/aggregate() earlier in the chain") do
      AnalyticsOrder.where({"status": "paid"}).having("total > 1")
    end
  end
end

describe("Grouped execution (DB)") do
  before_each() do
    requires_solidb()
  end

  after_each() do
    AnalyticsOrder.delete_all()
  end

  test("multi-key grouping with aggregates, having, order and limit") do
    seed_orders()

    rows = AnalyticsOrder.where({"status": "paid"})
      .group_by(["country", "plan"])
      .aggregate({"total": ["sum", "amount"], "n": ["count"]})
      .having("total > @min", {"min": 25})
      .order("total", "desc")
      .limit(20)
      .all

    # Paid groups: FR/basic 30 (2 rows), FR/pro 40, DE/basic 100 — all > 25.
    assert_eq(rows, [
      {"country": "DE", "plan": "basic", "total": 100, "n": 1},
      {"country": "FR", "plan": "pro", "total": 40, "n": 1},
      {"country": "FR", "plan": "basic", "total": 30, "n": 2}
    ])
  end

  test("limit cuts the sorted groups") do
    seed_orders()

    rows = AnalyticsOrder.where({"status": "paid"})
      .group_by("country")
      .aggregate({"total": ["sum", "amount"]})
      .order("total", "desc")
      .limit(1)
      .all

    assert_eq(rows, [{"country": "DE", "total": 100}])
  end

  test("having drops groups below the threshold") do
    seed_orders()

    rows = AnalyticsOrder.where({"status": "paid"})
      .group_by(["country", "plan"])
      .aggregate({"total": ["sum", "amount"]})
      .having("total > @min", {"min": 50})
      .all

    # Only DE/basic (100) clears the 50 bar.
    assert_eq(rows, [{"country": "DE", "plan": "basic", "total": 100}])
  end

  test("1-arg group_by returns implicit-count rows") do
    seed_orders()

    rows = AnalyticsOrder.group_by("country").all

    assert_eq(rows.length, 2)
    assert_eq(row_for(rows, "country", "FR")["n"], 4)
    assert_eq(row_for(rows, "country", "DE")["n"], 1)
  end

  test("grouping an empty match returns no rows") do
    seed_orders()

    assert_eq(AnalyticsOrder.where({"country": "ZZ"}).group_by("country").all, [])
  end

  test("ungrouped aggregate .first returns a hash with all aliases") do
    seed_orders()

    row = AnalyticsOrder.where({"status": "paid"}).aggregate({
      "total": ["sum", "amount"],
      "n": ["count"],
      "avg_amount": ["avg", "amount"]
    }).first

    assert_eq(row, {"total": 170, "n": 4, "avg_amount": 42.5})
  end

  test("ungrouped aggregate over an empty match is zero") do
    seed_orders()

    row = AnalyticsOrder.where({"country": "ZZ"}).aggregate({"total": ["sum", "amount"], "n": ["count"]}).first

    assert_eq(row, {"total": 0, "n": 0})
  end

  test("min and max aggregate together") do
    seed_amounts()

    assert_eq(AnalyticsOrder.aggregate({"lo": ["min", "amount"], "hi": ["max", "amount"]}).first, {"lo": 10, "hi": 40})
  end

  test("median/stddev/count_distinct terminals unwrap with .first") do
    seed_amounts()

    assert_eq(AnalyticsOrder.median("amount").first, 25)
    # The population standard deviation of 10, 20, 30, 40 is sqrt(125).
    stddev = AnalyticsOrder.stddev("amount").first
    assert_gt(stddev, 11.1803)
    assert_lt(stddev, 11.1804)
    assert_eq(AnalyticsOrder.count_distinct("country").first, 3)
  end

  test("median of an empty collection is nil") do
    assert_null(AnalyticsOrder.median("amount").first)
  end

  test("legacy 3-arg group_by rows keep the {group, result} shape") do
    AnalyticsOrder.create({"country": "FR", "amount": 10})
    AnalyticsOrder.create({"country": "FR", "amount": 20})
    AnalyticsOrder.create({"country": "DE", "amount": 5})

    rows = AnalyticsOrder.group_by("country", "sum", "amount").all

    assert_eq(rows.length, 2)
    assert_eq(row_for(rows, "group", "FR"), {"group": "FR", "result": 30})
    assert_eq(row_for(rows, "group", "DE"), {"group": "DE", "result": 5})
  end
end

describe("Soft-delete grouped execution (DB)") do
  before_each() do
    requires_solidb()
  end

  after_each() do
    AnalyticsSale.delete_all()
  end

  test("grouped query excludes soft-deleted rows") do
    doomed = AnalyticsSale.create({"country": "FR", "amount": 10})
    AnalyticsSale.create({"country": "FR", "amount": 20})
    AnalyticsSale.create({"country": "DE", "amount": 30})
    doomed.delete

    rows = AnalyticsSale.group_by("country").all

    assert_eq(rows.length, 2)
    assert_eq(row_for(rows, "country", "FR")["n"], 1)
    assert_eq(row_for(rows, "country", "DE")["n"], 1)
  end
end
