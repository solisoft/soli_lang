# A read that fails inside `grouped` answers for itself: the reads coalesced
# with it keep their own value, and a failure nobody could be told about is
# raised by the block's end. Runs against SoliDB.

class GrpNote < Model
end

describe("grouped: one read failing") do
  before_each() do
    requires_solidb()
    GrpNote.delete_all
    GrpNote.create({"title": "kept", "kind": "ban"})
  end

  test("a rescued miss leaves the reads coalesced with it their own value") do
    let ban = "unset"
    let total = "unset"
    let title = "unset"
    grouped(fn() {
      missing = GrpNote.find("no-such-key")
      ban = GrpNote.where({"kind": "ban"}).first
      total = GrpNote.where({"kind": "ban"}).count
      title = (missing.title rescue "rescued")
    })
    assert_eq(title, "rescued")
    assert(!ban.nil?)
    assert_eq(ban.title, "kept")
    assert_eq(total, 1)
  end

  # `grouped` takes its block as a literal, so each case spells its own
  # try / catch.
  test("a miss read where nothing can raise is raised by the block's end") do
    let outcome = "finished"
    try
      grouped(fn() {
        gone = GrpNote.find("no-such-key")
        seen = gone ? "truthy" : "falsy"
      })
    catch error
      outcome = "raised: #{error}"
    end
    assert(outcome.starts_with?("raised: "))
    assert_contains(outcome, "not found")
  end

  test("a block whose reads all answer finishes") do
    let outcome = "finished"
    try
      grouped(fn() {
        found = GrpNote.where({"kind": "ban"}).first
        label = found ? "truthy" : "falsy"
      })
    catch error
      outcome = "raised: #{error}"
    end
    assert_eq(outcome, "finished")
  end

  test("a miss nobody read is raised by the block's end") do
    let outcome = "finished"
    try
      grouped(fn() {
        post = GrpNote.find("no-such-key")
        total = GrpNote.where({"kind": "ban"}).count
        pages = total / 1
      })
    catch error
      outcome = "raised: #{error}"
    end
    assert_contains(outcome, "not found")
  end
end
