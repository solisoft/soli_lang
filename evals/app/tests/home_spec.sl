describe("fixture smoke", fn() {
    test("routes file exists", fn() {
        assert(File.exists("config/routes.sl"))
    })
})
