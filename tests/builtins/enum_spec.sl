# Enums: construction, payload access, pattern matching, methods, equality,
# and the JSON round-trip through `parse`.

enum Status
  Active,
  Archived,
  Pending(reason: String)

  def label -> String
    match self
      Status.Active => "Live",
      Status.Pending(r) => "Waiting: " + r,
      _ => "Archived",
    end
  end
end

enum Shape
  Circle(radius: Float),
  Rect(w: Float, h: Float),
  Point
end

describe("Enum construction") do
  test("a unit variant is a value of the enum's type") do
    status = Status.Active
    assert_eq(status.variant, "Active")
    assert_eq(type(status), "Status")
  end

  test("a payload variant takes named arguments") do
    status = Status.Pending(reason: "kyc")
    assert_eq(status.variant, "Pending")
    assert_eq(status.reason, "kyc")
  end

  test("a payload variant takes positional arguments") do
    status = Status.Pending("aml")
    assert_eq(status.variant, "Pending")
    assert_eq(status.reason, "aml")
  end

  test("a multi-field payload keeps every field") do
    rect = Shape.Rect(w: 3.0, h: 4.0)
    assert_eq(rect.variant, "Rect")
    assert_eq(rect.w, 3.0)
    assert_eq(rect.h, 4.0)
  end

  test("a payload variant called without its fields raises") do
    assert_raises("expected 1, got 0") do
      Status.Pending()
    end
  end
end

describe("Enum pattern matching") do
  test("matches a unit variant") do
    result = match Status.Active
      Status.Active => "live",
      Status.Pending(r) => "waiting",
      _ => "other",
    end
    assert_eq(result, "live")
  end

  test("binds a payload field positionally") do
    result = match Status.Pending(reason: "kyc")
      Status.Active => "live",
      Status.Pending(r) => "waiting: " + r,
      _ => "other",
    end
    assert_eq(result, "waiting: kyc")
  end

  test("binds multiple payload fields in declared order") do
    area = match Shape.Rect(w: 3.0, h: 4.0)
      Shape.Circle(radius) => 3.14 * radius * radius,
      Shape.Rect(w, h) => w * h,
      _ => 0.0,
    end
    assert_eq(area, 12.0)
  end

  test("matches a unit variant declared after payload variants") do
    result = match Shape.Point
      Shape.Circle(radius) => "circle",
      Shape.Point => "point",
      _ => "other",
    end
    assert_eq(result, "point")
  end

  test("the wildcard catches unhandled variants") do
    result = match Status.Archived
      Status.Active => "live",
      _ => "fallback",
    end
    assert_eq(result, "fallback")
  end
end

describe("Enum methods") do
  test("a method dispatches on self with match") do
    assert_eq(Status.Active.label, "Live")
    assert_eq(Status.Pending(reason: "kyc").label, "Waiting: kyc")
    assert_eq(Status.Archived.label, "Archived")
  end
end

describe("Enum equality") do
  # `==` and not assert_eq: assert_eq compares enum values by identity
  # (see the pending test at the end of this describe).
  test("a unit variant equals itself") do
    assert(Status.Active == Status.Active)
  end

  test("different unit variants are not equal") do
    assert(Status.Active != Status.Archived)
  end

  test("payload variants compare structurally") do
    assert(Status.Pending(reason: "x") == Status.Pending(reason: "x"))
    assert(Status.Pending(reason: "x") != Status.Pending(reason: "y"))
  end

  test("named and positional construction give equal values") do
    assert(Shape.Rect(w: 3.0, h: 4.0) == Shape.Rect(3.0, 4.0))
  end

  test("different variants of the same enum are not equal") do
    assert(Shape.Point != Shape.Circle(radius: 1.0))
  end

  test("a variant is not equal to its tag string or another enum's variant") do
    assert(Status.Active != "Active")
    assert(Status.Active != Shape.Point)
  end

  test("assert_eq agrees with == on structurally equal values") do
    pending("bug: assert_eq(Status.Pending(\"x\"), Status.Pending(\"x\")) fails although == is true")
    assert_eq(Status.Pending(reason: "x"), Status.Pending(reason: "x"))
    assert_eq(Status.parse("Active"), Status.Active)
  end
end

describe("Enum serialization and reconstruction") do
  test("a unit variant serializes to its tag string") do
    assert_eq(json_stringify({"s": Status.Active}), "{\"s\":\"Active\"}")
  end

  test("a payload variant serializes to a tagged object") do
    expected = "{\"s\":{\"variant\":\"Pending\",\"reason\":\"kyc\"}}"
    assert_eq(json_stringify({"s": Status.Pending(reason: "kyc")}), expected)
  end

  test("a multi-field payload serializes every field") do
    # Field order in the JSON is not fixed, so compare the parsed hash.
    assert_eq(json_parse(json_stringify(Shape.Rect(w: 3.0, h: 4.0))), {"variant": "Rect", "w": 3.0, "h": 4.0})
  end

  test("parse rebuilds a unit variant from its tag string") do
    assert(Status.parse("Active") == Status.Active)
  end

  test("parse rebuilds a payload variant from a tagged object") do
    assert(Status.parse({"variant": "Pending", "reason": "kyc"}) == Status.Pending(reason: "kyc"))
  end

  test("serialize then parse round-trips") do
    original = Status.Pending(reason: "review")
    stored = json_parse(json_stringify({"v": original}))
    assert(Status.parse(stored["v"]) == original)
  end

  test("parse refuses a tag that is not a variant") do
    pending("bug: Status.parse(\"Bogus\") returns <Status __variant: \"Bogus\"> instead of raising")
    assert_raises() do
      Status.parse("Bogus")
    end
  end
end
