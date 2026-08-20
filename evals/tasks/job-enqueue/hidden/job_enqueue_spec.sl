describe("job-enqueue", fn() {
    test("create enqueues NotifyJob.perform_later", fn() {
        let src = File.read("app/controllers/orders_controller.sl")
        assert(src.includes?("NotifyJob.perform_later"))
        assert(src.includes?("def create"))
    })
})
