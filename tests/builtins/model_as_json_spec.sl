# A Model subclass can define its own `as_json` (SEC-013a): it dispatches like
# any method and returns a custom-shape Hash. `render_json(instance)` routes
# through that override (SEC-013b) instead of serialising the raw fields.

# Every as_json call records the instance's name, so a spec can see that
# render_json reached the override.
as_json_calls = []

class AsJsonOverrideItem < Model
  # An explicit allowlist that ignores the default sensitive-field filter.
  def as_json
    as_json_calls.push(@name)
    {"id": @_key, "label": @name}
  end
end

describe("Model subclass-defined as_json (SEC-013a)") do
  before_each() do
    as_json_calls = []
  end

  test("the override produces exactly the custom Hash shape") do
    item = AsJsonOverrideItem.new()
    item.name = "Custom"
    item.password_hash = "would-leak-without-explicit-shape"

    # `_key` is nil on an unsaved instance.
    assert_eq(item.as_json, {"id": nil, "label": "Custom"})
  end

  test("returns a Hash, not the instance itself") do
    item = AsJsonOverrideItem.new()
    item.name = "x"

    assert_eq(type(item.as_json), "hash")
    assert_eq(as_json_calls, ["x"])
  end
end

describe("render_json auto-dispatch through as_json (SEC-013b)") do
  before_each() do
    as_json_calls = []
  end

  test("render_json(instance) calls the override once") do
    item = AsJsonOverrideItem.new()
    item.name = "AutoCustom"

    render_json(item)

    assert_eq(as_json_calls, ["AutoCustom"])
  end

  test("render_json returns nil — the response is set on the request") do
    item = AsJsonOverrideItem.new()
    item.name = "Quiet"

    assert_null(render_json(item))
  end

  test("render_json on a plain hash does not touch the override") do
    render_json({"label": "plain"})

    assert_eq(as_json_calls, [])
  end
end
