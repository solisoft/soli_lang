# with_transaction (test helper): runs a block in a SoliDB transaction and
# always rolls it back, whether the block returns or throws.
#
# Reads are deliberately not transactional: a write goes to
# /transaction/{tx}/document/... while a query-builder read goes to the
# ordinary cursor, so an uncommitted row is invisible from inside the block.

class TxTestUser < Model
end

def user_emails
  TxTestUser.all.map { |user| user.email }.sort()
end

describe("with_transaction") do
  before_each() do
    requires_solidb()
    Factory.clear()
    Factory.define("user", {"email": "tx@test.com"})
    Factory.bind("user", TxTestUser)
    # A committed row, which also creates the collection: a write inside a
    # transaction does not create a missing one.
    TxTestUser.create({"email": "existing@test.com"})
  end

  after_each() do
    TxTestUser.delete_all()
  end

  test("the write inside the block succeeds, and is rolled back after it") do
    inserted = nil
    inside_emails = nil
    with_transaction(fn() {
      inserted = Factory.insert("user")
      inside_emails = user_emails()
    })

    # The insert really went through the transaction — without this the
    # rollback check below would hold just as well if the write had failed.
    assert_null(inserted._errors)
    assert_eq(inserted.email, "tx@test.com")
    assert_eq(inserted._key.length, 36)
    # Query-builder reads inside the block see only committed state.
    assert_eq(inside_emails, ["existing@test.com"])
    # The block finished normally, and still nothing was committed.
    assert_eq(user_emails(), ["existing@test.com"])
  end

  test("the same insert outside the block persists") do
    Factory.insert("user")

    assert_eq(user_emails(), ["existing@test.com", "tx@test.com"])
  end

  test("a block that throws is rolled back and the error re-raised") do
    assert_raises("boom") do
      with_transaction(fn() {
        Factory.insert("user")
        throw "boom"
      })
    end

    assert_eq(user_emails(), ["existing@test.com"])
  end

  test("rolls back updates and deletes of committed rows") do
    existing = TxTestUser.find_by("email", "existing@test.com")
    with_transaction(fn() {
      existing.email = "changed@test.com"
      existing.save
      TxTestUser.create({"email": "doomed@test.com"}).delete
    })

    assert_eq(TxTestUser.find(existing._key).email, "existing@test.com")
  end

  test("takes a do block too") do
    inserted = nil
    with_transaction() do
      inserted = Factory.insert("user")
    end

    assert_null(inserted._errors)
    assert_eq(inserted.email, "tx@test.com")
    assert_eq(user_emails(), ["existing@test.com"])
  end
end
