# ============================================================================
# Enum Test Suite
# ============================================================================

enum Status {
  Active,
  Archived,
  Pending(reason: String)

  fn label() -> String {
    return match self {
      Status.Active => "Live",
      Status.Pending(r) => "Waiting: " + r,
      _ => "Archived",
    }
  }
}

enum Shape {
  Circle(radius: Float),
  Rect(w: Float, h: Float),
  Point
}

describe("Enum construction", fn() {
  test("unit variant is a value", fn() {
    let s = Status.Active
    assert_eq(s.variant(), "Active")
  })

  test("named payload construction", fn() {
    let s = Status.Pending(reason: "kyc")
    assert_eq(s.variant(), "Pending")
  })

  test("positional payload construction", fn() {
    let s = Status.Pending("aml")
    assert_eq(s.variant(), "Pending")
  })

  test("multi-field payload", fn() {
    let r = Shape.Rect(w: 3.0, h: 4.0)
    assert_eq(r.variant(), "Rect")
  })
})

describe("Enum pattern matching", fn() {
  test("matches a unit variant", fn() {
    let result = match Status.Active {
      Status.Active => "live",
      Status.Pending(r) => "waiting",
      _ => "other",
    }
    assert_eq(result, "live")
  })

  test("binds a payload field positionally", fn() {
    let result = match Status.Pending(reason: "kyc") {
      Status.Active => "live",
      Status.Pending(r) => "waiting: " + r,
      _ => "other",
    }
    assert_eq(result, "waiting: kyc")
  })

  test("binds multiple payload fields by declared order", fn() {
    let area = match Shape.Rect(w: 3.0, h: 4.0) {
      Shape.Circle(radius) => 3.14 * radius * radius,
      Shape.Rect(w, h) => w * h,
      _ => 0.0,
    }
    assert_eq(area, 12.0)
  })

  test("wildcard catches unhandled variants", fn() {
    let result = match Status.Archived {
      Status.Active => "live",
      _ => "fallback",
    }
    assert_eq(result, "fallback")
  })
})

describe("Enum methods", fn() {
  test("method dispatches via match self", fn() {
    assert_eq(Status.Active.label(), "Live")
    assert_eq(Status.Pending(reason: "kyc").label(), "Waiting: kyc")
    assert_eq(Status.Archived.label(), "Archived")
  })
})

describe("Enum equality", fn() {
  test("unit variants are equal to themselves", fn() {
    assert(Status.Active == Status.Active)
  })

  test("different unit variants are not equal", fn() {
    assert(Status.Active != Status.Archived)
  })

  test("payload variants compare structurally", fn() {
    assert(Status.Pending(reason: "x") == Status.Pending(reason: "x"))
    assert(Status.Pending(reason: "x") != Status.Pending(reason: "y"))
  })

  test("different variants of the same enum are not equal", fn() {
    assert(Shape.Point != Shape.Circle(radius: 1.0))
  })
})

describe("Enum serialization & reconstruction", fn() {
  test("unit variant serializes to its tag string", fn() {
    assert_eq(json_stringify({"s": Status.Active}), "{\"s\":\"Active\"}")
  })

  test("payload variant serializes to a tagged object", fn() {
    assert_eq(
      json_stringify({"s": Status.Pending(reason: "kyc")}),
      "{\"s\":{\"variant\":\"Pending\",\"reason\":\"kyc\"}}"
    )
  })

  test("Status.parse rebuilds a unit variant from its tag string", fn() {
    assert(Status.parse("Active") == Status.Active)
  })

  test("Status.parse rebuilds a payload variant from a tagged object", fn() {
    let back = Status.parse({"variant": "Pending", "reason": "kyc"})
    assert(back == Status.Pending(reason: "kyc"))
  })

  test("serialize then parse round-trips", fn() {
    let original = Status.Pending(reason: "review")
    let stored = json_parse(json_stringify({"v": original}))
    assert(Status.parse(stored["v"]) == original)
  })
})
