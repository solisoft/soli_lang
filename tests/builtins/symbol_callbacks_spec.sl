# Callback DSL: a callback names its method with a symbol (`:normalize`) or a
# string ("string_cb"), with or without parentheses. All four register, run in
# declaration order, and a name registered twice runs twice. The paren-less
# forms are the subject: do not run `soli fmt` on this file, it adds parens.

class SpecCallbacks < Model
  before_save(:normalize)
  before_save :normalize
  before_save("string_cb")
  after_create(:notify)
  after_create "notify_str"

  def normalize
    @calls = (@calls ?? []) + ["normalize"]
    @name = @name.downcase
    true
  end

  def string_cb
    @calls = (@calls ?? []) + ["string_cb"]
    true
  end

  def notify
    @calls = @calls + ["notify"]
    true
  end

  def notify_str
    @calls = @calls + ["notify_str"]
    true
  end
end

describe("Callback DSL with symbols and strings") do
  test("the callback methods run on a plain instance, without the database") do
    record = SpecCallbacks.new()
    record.name = "MiXeD"

    assert_eq(record.normalize, true)
    assert_eq(record.string_cb, true)
    assert_eq(record.name, "mixed")
    assert_eq(record.calls, ["normalize", "string_cb"])
  end

  describe("on persistence") do
    before_each() do
      requires_solidb()
    end

    after_each() do
      SpecCallbacks.delete_all()
    end

    test("create runs symbol and string callbacks in declaration order") do
      record = SpecCallbacks.create({"name": "MiXeD"})

      assert_null(record._errors)
      assert_eq(record.calls, ["normalize", "normalize", "string_cb", "notify", "notify_str"])
    end

    test("the before_save callbacks changed what was stored") do
      record = SpecCallbacks.create({"name": "MiXeD"})
      stored = SpecCallbacks.find(record._key)

      assert_eq(stored.name, "mixed")
      # after_create ran after the write, so its entries were never stored.
      assert_eq(stored.calls, ["normalize", "normalize", "string_cb"])
    end

    test("save() on a persisted record runs before_save but not after_create") do
      record = SpecCallbacks.create({"name": "first"})
      record.name = "SECOND"
      record.calls = []
      record.save()

      assert_eq(record.calls, ["normalize", "normalize", "string_cb"])
      assert_eq(SpecCallbacks.find(record._key).name, "second")
    end

    test("save without parentheses runs before_save like save()") do
      pending("bug: bare `record.save` persists but skips before_save; `record.save()` runs it")
      record = SpecCallbacks.new()
      record.name = "BARE"
      record.save

      assert_eq(SpecCallbacks.find(record._key).name, "bare")
    end
  end
end
