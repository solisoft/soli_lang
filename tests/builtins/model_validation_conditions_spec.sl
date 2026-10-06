# Conditional and per-operation validations:
#   validates(field, { ..., "on": "create"|"update", "if": fn, "unless": fn })
# A failing rule stops the write before the database is reached, so those
# specs need no database; the passing paths persist and require SoliDB.

class OnCreateDoc < Model
  validates("title", {"presence": true, "on": "create"})
end

class OnUpdateDoc < Model
  validates("reviewer", {"presence": true, "on": "update"})
end

class BareHashOnDoc < Model
  validates(:label, presence: true, on: "create")
end

class StrictNick < Model
  validates("nickname", {"min_length": 5, "if": fn(record) { record["strict"] == true }})
end

class BioUser < Model
  validates("bio", {"presence": true, "unless": fn(record) { record["role"] == "admin" }})
end

class NeverChecked < Model
  validates("code", {"presence": true, "if": fn() { false }})
end

describe("conditional validations") do
  describe("when the rule applies (no database reached)") do
    test("on: create applies to create") do
      assert_eq(OnCreateDoc.create({})._errors, [{"field": "title", "message": "can't be blank"}])
    end

    test("the bare-hash option style parses on:") do
      assert_eq(BareHashOnDoc.create({})._errors, [{"field": "label", "message": "can't be blank"}])
    end

    test("if: runs the rule when the condition is true") do
      user = StrictNick.create({"strict": true, "nickname": "abc"})

      assert_eq(user._errors, [{"field": "nickname", "message": "is too short (minimum is 5 characters)"}])
    end

    test("unless: runs the rule when the condition is false") do
      user = BioUser.create({"role": "member"})

      assert_eq(user._errors, [{"field": "bio", "message": "can't be blank"}])
    end
  end

  describe("when the rule is skipped (DB)") do
    before_each() do
      requires_solidb()
    end

    after_each() do
      OnCreateDoc.delete_all()
      OnUpdateDoc.delete_all()
      StrictNick.delete_all()
      BioUser.delete_all()
      NeverChecked.delete_all()
    end

    test("on: create does not block an update") do
      doc = OnCreateDoc.create({"title": "hello"})
      assert_null(doc._errors)
      doc.title = nil

      assert_eq(doc.update, true)
      assert_null(OnCreateDoc.find(doc._key).title)
    end

    test("on: update does not block a create") do
      doc = OnUpdateDoc.create({"note": "x"})

      assert_null(doc._errors)
      assert_eq(OnUpdateDoc.find(doc._key).note, "x")
    end

    test("on: update applies to update") do
      doc = OnUpdateDoc.create({"note": "x"})

      assert_eq(doc.update, false)
      assert_eq(doc._errors, [{"field": "reviewer", "message": "can't be blank"}])
    end

    test("on: update passes once the field is given") do
      doc = OnUpdateDoc.create({"note": "x"})

      assert_eq(doc.update({"reviewer": "bob"}), true)
      assert_eq(OnUpdateDoc.find(doc._key).reviewer, "bob")
    end

    test("if: skips the rule when the condition is false") do
      user = StrictNick.create({"strict": false, "nickname": "abc"})

      assert_null(user._errors)
      assert_eq(StrictNick.find(user._key).nickname, "abc")
    end

    test("if: is re-evaluated on update") do
      user = StrictNick.create({"strict": false, "nickname": "abc"})
      user.strict = true

      assert_eq(user.update, false)
      assert_eq(user._errors[0]["field"], "nickname")
    end

    test("a zero-parameter condition works") do
      record = NeverChecked.create({})

      assert_null(record._errors)
      assert_eq(NeverChecked.count, 1)
    end

    test("unless: skips the rule when the condition is true") do
      user = BioUser.create({"role": "admin"})

      assert_null(user._errors)
      assert_eq(BioUser.find(user._key).role, "admin")
    end
  end
end
