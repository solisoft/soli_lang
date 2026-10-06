# clock(), sleep(), Int#sleep / Float#sleep and DateTime.microtime.

describe("clock") do
  test("is the wall clock in fractional Unix seconds") do
    now = clock()
    assert_eq(type(now), "float")
    assert_lt((now - DateTime.now.to_unix).abs, 2)
  end

  test("never goes backwards") do
    first = clock()
    second = clock()
    assert(second >= first)
  end

  test("ignores freeze_time") do
    freeze_time(1000)
    assert_gt(clock(), 1700000000)
  end
end

describe("sleep") do
  test("pauses for a fraction of a second") do
    start = clock()
    sleep(0.01)
    assert(clock() - start >= 0.01)
  end

  test("returns nil") do
    assert_null(sleep(0))
  end

  test("raises on a non-number") do
    assert_raises("sleep() expects number") do
      sleep(["x"][0])
    end
  end

  test("a negative Int is a no-op") do
    pending("bug: sleep(-1) casts -1 to u64 and blocks forever")
    start = clock()
    sleep(-1)
    assert_lt(clock() - start, 0.5)
  end

  test("a negative Float is a no-op") do
    pending("bug: sleep(-0.5) panics the interpreter (negative Duration)")
    start = clock()
    sleep(-0.5)
    assert_lt(clock() - start, 0.5)
  end
end

describe("Int#sleep and Float#sleep") do
  test("Int#sleep pauses and returns nil") do
    start = clock()
    assert_null(0.sleep)
    assert_lt(clock() - start, 0.5)
  end

  test("Float#sleep pauses for a fraction of a second") do
    start = clock()
    assert_null((0.01).sleep)
    assert(clock() - start >= 0.01)
  end

  test("a negative receiver is a no-op") do
    start = clock()
    (-1).sleep
    (-0.5).sleep
    assert_lt(clock() - start, 0.5)
  end
end

describe("DateTime.microtime") do
  test("is the wall clock in microseconds") do
    micros = DateTime.microtime
    assert_eq(type(micros), "float")
    assert_lt((micros / 1000000 - clock()).abs, 1)
  end
end
