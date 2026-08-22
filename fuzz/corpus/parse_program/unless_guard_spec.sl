describe("unless-guard", fn() {
    test("block unless in orders controller", fn() {
        let src = File.read("app/controllers/orders_controller.sl")
        assert(src.includes?("unless"))
    })
})
