# pending() and skip() stop a test without failing it. They used to raise an
# error nothing recognised, so the test counted as a failure and failed the
# whole run. If this file fails, they regressed.
describe("pending and skip") do
  test("pending() stops the test before a failing assertion") do
    pending()
    assert(false)
  end

  test("pending(reason) takes a reason") do
    pending("not written yet")
    assert(false)
  end

  test("skip(reason) stops the test too") do
    skip("waiting on the API")
    assert_eq(1, 2)
  end

  test("a test after them still runs") do
    assert_eq(1 + 1, 2)
  end
end
