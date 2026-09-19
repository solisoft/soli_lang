class FactoryTestUser < Model
end

let __db_available = false
try
    probe = FactoryTestUser.create({"email": "__probe__@test.com"})
    if !probe.nil? && !probe._errors
        __db_available = true
        probe.delete()
    end
catch error
end

describe("Factory", fn() {
    before_each(fn() {
        Factory.clear()
    })

    test("static hash factories merge overrides", fn() {
        Factory.define("user", {"email": "base@test.com", "name": "Base"})
        user = Factory.create_with("user", {"name": "Override"})
        assert_eq(user["email"], "base@test.com")
        assert_eq(user["name"], "Override")
    })

    test("callable factories run on each create", fn() {
        counter = 0
        Factory.define("user", fn() {
            counter = counter + 1
            return {"n": counter}
        })
        first = Factory.create("user")
        second = Factory.create("user")
        assert_eq(first["n"], 1)
        assert_eq(second["n"], 2)
    })

    test("interpolates #{n} in string attributes", fn() {
        Factory.define("user", {"email": "user#{n}@test.com"})
        first = Factory.create("user")
        second = Factory.create("user")
        assert_eq(first["email"], "user0@test.com")
        assert_eq(second["email"], "user1@test.com")
    })

    test("insert persists through bound model", fn() {
        if !__db_available
            return
        end

        # Nothing resets the database between runs, so start from a known
        # state — this asserted an absolute count and so passed exactly once
        # per clean database and failed on every run after.
        for leftover in FactoryTestUser.all()
            leftover.delete()
        end

        Factory.define("user", {"email": "persist@test.com"})
        Factory.bind("user", FactoryTestUser)
        record = Factory.insert("user")
        # `all()` rather than `count()`: the latter asks SoliDB for
        # COLLECTION_COUNT, whose counter is not decremented on delete, so it
        # reports rows that a scan of the same collection does not return.
        assert_eq(len(FactoryTestUser.all()), 1)
        assert_eq(record.email, "persist@test.com")
        record.delete()
    })
})