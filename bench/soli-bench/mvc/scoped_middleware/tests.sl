describe("scoped_middleware", fn() {
    before_each(fn() {
        User.all.each(fn(user) user.delete())
        User.create({"name": "Ada", "role": "admin"})
        User.create({"name": "Bo", "role": "member"})
        User.create({"name": "Cy", "role": "admin"})
    })

    test("admins scope filters by role", fn() {
        assert_eq(User.admins.all().length, 2)
    })

    test("middleware passes an admin through", fn() {
        assert_null(require_admin({"user": {"role": "admin"}}))
    })

    test("middleware blocks a non-admin", fn() {
        res = require_admin({"user": {"role": "member"}})
        assert_eq(res["status"], 403)
    })

    test("middleware blocks an anonymous request", fn() {
        res = require_admin({})
        assert_eq(res["status"], 403)
    })
})
