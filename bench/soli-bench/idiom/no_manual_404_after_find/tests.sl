# In a real app `Post` is auto-loaded and `.find` raises RecordNotFound on a
# miss (handled as a 404 by the request handler). Here we stand it in so the
# happy path can be exercised without a database.
class Post {
    static def find(id) {
        return {"id": id, "title": "Hello World"}
    }
}

describe("no_manual_404_after_find", fn() {
    test("returns the found post", fn() {
        res = show(1)
        assert_eq(res["status"], 200)
        assert_eq(res["title"], "Hello World")
    })
})
