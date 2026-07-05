# ============================================================================
# Conditional & per-operation model validations
# validates(field, { ..., "on": "create"|"update", "if": fn, "unless": fn })
# Failing-path assertions run without a database (validation fails before
# any insert/update). Passing-path assertions that persist are gated behind
# the DB availability probe, matching model_instances_spec.sl.
# ============================================================================
class OnCreateDoc < Model
  validates(
    "title",
    {"presence": true, "on": "create"}
  )
end

class OnUpdateDoc < Model
  validates(
    "reviewer",
    {"presence": true, "on": "update"}
  )
end

class BareHashOnDoc < Model
  validates(:label, presence: true, on: "create")
end

class StrictNick < Model
  validates(
    "nickname",
    {"min_length": 5, "if": fn(record) { record["strict"] == true }}
  )
end

class BioUser < Model
  validates(
    "bio",
    {"presence": true, "unless": fn(record) { record["role"] == "admin" }}
  )
end

class NeverChecked < Model
  validates(
    "code",
    {"presence": true, "if": fn() { false }}
  )
end

# Detect DB availability
let __db_available = false
try
  let __probe = OnCreateDoc.create({"title": "__probe__"})
  if !__probe.nil? && !__probe._errors
    __db_available = true
    __probe.delete()
  end
catch e
  __db_available = false
end

describe("validates on: create", fn() {
  test("rule applies to create", fn() {
    let doc = OnCreateDoc.create({})
    assert_not_null(doc._errors)
    assert_eq(doc._errors[0]["field"], "title")
  })

  test("bare-hash option style parses on:", fn() {
    let doc = BareHashOnDoc.create({})
    assert_not_null(doc._errors)
    assert_eq(doc._errors[0]["field"], "label")
  })

  test("rule is skipped on update", fn() {
    if __db_available
      let doc = OnCreateDoc.create({"title": "hello"})
      assert_null(doc._errors)
      doc.title = null
      # presence(on: create) must not block the update
      assert(doc.update())
      doc.delete()
    end
  })
})

describe("validates on: update", fn() {
  test("rule is skipped on create", fn() {
    if __db_available
      let doc = OnUpdateDoc.create({})
      assert_null(doc._errors)
      doc.delete()
    end
  })

  test("rule applies to update", fn() {
    if __db_available
      let doc = OnUpdateDoc.create({"note": "x"})
      assert_null(doc._errors)
      assert_not(doc.update())
      assert_eq(doc._errors[0]["field"], "reviewer")
      doc.delete()
    end
  })
})

describe("validates if:", fn() {
  test("rule runs when the condition is true", fn() {
    let user = StrictNick.create({
      "strict": true,
      "nickname": "abc"
    })
    assert_not_null(user._errors)
    assert_eq(user._errors[0]["field"], "nickname")
  })

  test("rule is skipped when the condition is false", fn() {
    if __db_available
      let user = StrictNick.create({
        "strict": false,
        "nickname": "abc"
      })
      assert_null(user._errors)
      user.delete()
    end
  })

  test("zero-param condition works", fn() {
    if __db_available
      let rec = NeverChecked.create({})
      assert_null(rec._errors)
      rec.delete()
    end
  })
})

describe("validates unless:", fn() {
  test("rule runs when the condition is false", fn() {
    let user = BioUser.create({"role": "member"})
    assert_not_null(user._errors)
    assert_eq(user._errors[0]["field"], "bio")
  })

  test("rule is skipped when the condition is true", fn() {
    if __db_available
      let user = BioUser.create({"role": "admin"})
      assert_null(user._errors)
      user.delete()
    end
  })
})
