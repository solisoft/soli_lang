describe("scaffold-resource", fn() {
    test("resources notes and NotesController", fn() {
        let routes = File.read("config/routes.sl")
        assert(routes.includes?("resources"))
        assert(routes.includes?("notes"))
        assert(File.exists("app/controllers/notes_controller.sl"))
        let src = File.read("app/controllers/notes_controller.sl")
        assert(src.includes?("NotesController"))
    })
})
