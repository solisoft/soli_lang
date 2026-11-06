# The app metrics API. `soli test` sets APP_ENV=test, which keeps samples even
# with no sink configured, so Metrics.value reads them back.

describe("Metrics") do
  before_each() do
    Metrics.reset
  end

  test("is on under soli test") do
    assert(Metrics.enabled?)
  end

  test("increment counts, by one or by a value, per tag set") do
    Metrics.increment("orders.created")
    Metrics.increment("orders.created", 2)
    Metrics.increment("orders.created", tags: {"plan": "pro"})
    Metrics.increment("orders.created", 4, {"plan": "pro"})
    assert_eq(Metrics.value("orders.created"), 3)
    assert_eq(Metrics.value("orders.created", {"plan": "pro"}), 5)
    assert_eq(Metrics.value("orders.created", tags: {"plan": "free"}), nil)
  end

  test("increment takes the tags in the value's place") do
    Metrics.increment("signups", {"source": "ad"})
    assert_eq(Metrics.value("signups", {"source": "ad"}), 1)
  end

  test("increment adds fractions") do
    Metrics.increment("credits", 0.5)
    Metrics.increment("credits", 0.25)
    assert_eq(Metrics.value("credits"), 0.75)
  end

  test("gauge keeps the last value") do
    Metrics.gauge("queue.depth", 12, tags: {"queue": "mail"})
    Metrics.gauge("queue.depth", 3, tags: {"queue": "mail"})
    assert_eq(Metrics.value("queue.depth", {"queue": "mail"}), 3)
  end

  test("tag values may be numbers, bools or nil") do
    Metrics.gauge("mixed", 1, {"n": 2, "ok": true, "none": nil})
    assert_eq(Metrics.value("mixed", {"n": "2", "ok": "true", "none": ""}), 1)
  end

  test("histogram counts, sums and buckets observations") do
    Metrics.histogram("cart.items", 0.003)
    Metrics.histogram("cart.items", 0.2)
    Metrics.distribution("cart.items", 7)
    summary = Metrics.value("cart.items")
    assert_eq(summary["count"], 3)
    assert_lt((summary["sum"] - 7.203).abs, 0.000001)
    assert_eq(summary["buckets"]["0.005"], 1)
    assert_eq(summary["buckets"]["0.25"], 2)
    assert_eq(summary["buckets"]["10"], 3)
    assert_eq(summary["buckets"]["+Inf"], 3)
  end

  test("time records the block's duration and answers its value") do
    answer = Metrics.time("checkout.duration") do
      sleep(0.01)
      42
    end
    assert_eq(answer, 42)
    summary = Metrics.value("checkout.duration")
    assert_eq(summary["count"], 1)
    assert_gt(summary["sum"], 0.009)
  end

  test("time takes its tags positionally") do
    Metrics.time("job.run", {"job": "mail"}) do
      1
    end
    Metrics.time("job.run", {"job": "mail"}) do
      2
    end
    assert_eq(Metrics.value("job.run", {"job": "mail"})["count"], 2)
  end

  test("time records even when the block raises") do
    outcome = Metrics.time("risky") do
      throw "boom"
    end rescue "rescued"
    assert_eq(outcome, "rescued")
    assert_eq(Metrics.value("risky")["count"], 1)
  end

  test("dotted and underscored names are one metric") do
    Metrics.increment("api.calls")
    Metrics.increment("api_calls")
    assert_eq(Metrics.value("api.calls"), 2)
  end

  test("refuses a bad name") do
    assert_raises("invalid metric name") do
      Metrics.increment("has space")
    end
    assert_raises("reserved") do
      Metrics.increment("soli.http.requests")
    end
  end

  test("refuses a bad tag") do
    assert_raises("invalid tag key") do
      Metrics.increment("ok", tags: {"bad key": "x"})
    end
    assert_raises("reserved") do
      Metrics.histogram("ok_h", 1, tags: {"le": "1"})
    end
    assert_raises("must be a String, number, Bool or nil") do
      Metrics.increment("ok", tags: {"list": [1]})
    end
  end

  test("refuses a bad value") do
    assert_raises("expects a number") do
      Metrics.gauge("g", "12")
    end
    assert_raises("a counter only goes up") do
      Metrics.increment("down", -1)
    end
  end

  test("refuses to reuse a name as another kind") do
    Metrics.increment("dual")
    assert_raises("already a counter") do
      Metrics.gauge("dual", 1)
    end
  end
end

describe("Metrics.time when the block raises") do
  test("the error reaches the caller unchanged") do
    Metrics.reset
    message = nil
    try
      Metrics.time("raising") do
        throw "kaboom"
      end
    catch error
      message = error
    end
    assert_eq(message, "kaboom")
    assert_eq(Metrics.value("raising")["count"], 1)
  end
end
