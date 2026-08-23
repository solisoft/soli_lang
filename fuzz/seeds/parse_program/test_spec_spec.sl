describe("test-spec", fn() {
    test("orders spec uses assert_eq", fn() {
        assert(File.exists("tests/orders_spec.sl"))
        let src = File.read("tests/orders_spec.sl")
        assert(src.includes?("describe"))
        assert(src.includes?("assert_eq"))
        assert_not(src.includes?("assert_equal"))
    })
})
