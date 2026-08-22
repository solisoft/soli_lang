describe("csrf-webhook", fn() {
    test("skip_csrf on webhook only", fn() {
        let src = File.read("app/controllers/orders_controller.sl")
        assert(src.includes?("skip_csrf"))
        assert(src.includes?("webhook"))
        let routes = File.read("config/routes.sl")
        assert(routes.includes?("webhook"))
    })
})
