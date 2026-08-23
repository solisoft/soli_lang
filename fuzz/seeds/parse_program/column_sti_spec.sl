describe("column-sti", fn() {
    test("Payment table + CardPayment subclass", fn() {
        assert(File.exists("app/models/payment.sl"))
        let src = File.read("app/models/payment.sl")
        assert(src.includes?("table"))
        assert(src.includes?("payments"))
        let has_child = File.exists("app/models/card_payment.sl") || src.includes?("CardPayment")
        assert(has_child)
    })
})
