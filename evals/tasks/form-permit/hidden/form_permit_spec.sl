describe("form-permit", fn() {
    test("permit + Order.create", fn() {
        let src = File.read("app/controllers/orders_controller.sl")
        assert(src.includes?("permit"))
        assert(src.includes?("Order.create"))
    })
})
