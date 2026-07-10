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

        Factory.define("user", {"email": "tx@test.com"})
        Factory.bind("user", TxTestUser)

        with_transaction(fn() {
            Factory.insert("user")
            assert_eq(TxTestUser.count(), 1)
        })

        assert_eq(TxTestUser.count(), 0)
    })
})