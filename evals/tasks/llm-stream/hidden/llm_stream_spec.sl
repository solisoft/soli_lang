describe("llm-stream", fn() {
    test("sse + llm_stream", fn() {
        let src = File.read("app/controllers/orders_controller.sl")
        assert(src.includes?("sse") || src.includes?("llm_stream"))
        assert(src.includes?("llm_stream"))
    })
})
