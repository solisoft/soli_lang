# freeze_time / travel_to / unfreeze_time pin datetime_now() for time-dependent
# specs; the runner unfreezes before every test.

describe("freeze_time") do
  test("pins datetime_now to a timestamp") do
    freeze_time(1700000000)
    assert_eq(datetime_now(), 1700000000)
    assert_eq(datetime_now(), 1700000000)
  end

  test("accepts a date string") do
    freeze_time("2024-01-15T10:00:00Z")
    assert_eq(datetime_now(), 1705312800)
  end

  test("a second freeze moves the clock") do
    freeze_time(1700000000)
    freeze_time(1700000001)
    assert_eq(datetime_now(), 1700000001)
  end

  test("unfreeze_time returns to the wall clock") do
    freeze_time(1000)
    unfreeze_time()
    assert_lt((datetime_now() - DateTime.now.to_unix).abs, 2)
  end

  test("the runner unfreezes between tests") do
    freeze_time(1000)
    assert_eq(datetime_now(), 1000)
  end

  test("so the previous test's freeze is gone") do
    assert_gt(datetime_now(), 1700000000)
  end

  test("travel_to is an alias") do
    travel_to(1715212800)
    assert_eq(datetime_now(), 1715212800)
  end

  test("raises on a string that is not a date") do
    assert_raises("freeze_time(): invalid date string \"nope\"") do
      freeze_time("nope")
    end
  end

  test("raises on a DateTime") do
    assert_raises("freeze_time() expects timestamp (int) or date string, got DateTime") do
      freeze_time(DateTime.epoch)
    end
  end

  test("pins DateTime.now too") do
    pending("bug: DateTime.now reads the wall clock and ignores freeze_time")
    freeze_time(1700000000)
    assert_eq(DateTime.now.to_unix, 1700000000)
  end
end
