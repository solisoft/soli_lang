# Tests for the counter component in app/controllers/eui_controller.sl —
# written by `soli new --eui`. A handler and a view are plain functions: a
# spec calls them directly, the catalogue's builders included.

describe("counter component") do
  test("the handler steps the count") do
    assert_eq(counter({"event": "increment", "state": {"count": 1}}), {"count": 2})
    assert_eq(counter({"event": "decrement", "state": {"count": 1}}), {"count": 0})
  end

  test("an unnamed event keeps the state; a fresh one starts at zero") do
    assert_eq(counter({"event": "connect", "state": {"count": 3}}), {"count": 3})
    assert_eq(counter({"event": "connect", "state": {}}), {"count": 0})
  end

  test("the view shows the count") do
    tree = counter_view({"count": 42})
    assert(json_stringify(tree).contains("\"42\""))
  end
end
