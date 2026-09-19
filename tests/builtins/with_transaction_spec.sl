class TxTestUser < Model
end

let __db_available = false
try
    probe = TxTestUser.create({"email": "__probe__@test.com"})
    if !probe.nil? && !probe._errors
        __db_available = true
        probe.delete()
    end
catch error
end

describe("with_transaction", fn() {
    before_each(fn() {
        Factory.clear()
    })

    test("rolls back writes after the block", fn() {
        if !__db_available
            return
        end

        # Start from a known state: nothing here resets the database, so a
        # previous run's rows would make the counts below meaningless.
        for leftover in TxTestUser.all()
            leftover.delete()
        end

        Factory.define("user", {"email": "tx@test.com"})
        Factory.bind("user", TxTestUser)

        with_transaction(fn() {
            Factory.insert("user")
            # Reads are deliberately NOT transactional: the write goes to
            # /transaction/{tx}/document/... while a query-builder read goes
            # to the ordinary cursor, so the uncommitted row is not visible
            # from in here. Asserting that it *was* visible is what this test
            # used to do, and it contradicted the documented behaviour.
            assert_eq(len(TxTestUser.all()), 0)
        })

        # `with_transaction` always rolls back, so the row never lands.
        assert_eq(len(TxTestUser.all()), 0)

        # And the same insert outside the block does persist — without this
        # the assertion above would hold just as well if the write had
        # silently failed, which is the failure mode worth guarding.
        Factory.insert("user")
        assert_eq(len(TxTestUser.all()), 1)
        TxTestUser.all()[0].delete()
    })
})