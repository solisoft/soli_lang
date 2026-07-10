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

        Factory.define("user", {"email": "persist@test.com"})
        Factory.bind("user", FactoryTestUser)
        record = Factory.insert("user")
        assert_eq(FactoryTestUser.count(), 1)
        assert_eq(record.email, "persist@test.com")
    })
})