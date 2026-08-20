describe("validation", fn() {
    test("Order validates email", fn() {
        let src = File.read("app/models/order.sl")
        assert(src.includes?("validates"))
        assert(src.includes?("email"))
    })
})
