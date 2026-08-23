describe("attachment", fn() {
    test("has_one_attached receipt", fn() {
        let src = File.read("app/models/order.sl")
        assert(src.includes?("has_one_attached"))
        assert(src.includes?("receipt"))
    })
})
