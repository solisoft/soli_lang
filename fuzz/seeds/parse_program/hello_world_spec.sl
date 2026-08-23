describe("hello-world", fn() {
    test("app/hello.sl exists", fn() {
        assert(File.exists("app/hello.sl"))
    })

    test("defines greet", fn() {
        let src = File.read("app/hello.sl")
        assert(src.includes?("greet"))
    })
})
