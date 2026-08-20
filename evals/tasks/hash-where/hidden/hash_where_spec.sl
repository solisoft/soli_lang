describe("hash-where", fn() {
    test("index uses Order.where with a hash comparison", fn() {
        let src = File.read("app/controllers/orders_controller.sl")
        assert(src.includes?("Order.where"))
        let has_gt = src.includes?("gt") || src.includes?(">")
        assert(has_gt)
        assert_not(src.includes?("doc.total"))
    })
})
