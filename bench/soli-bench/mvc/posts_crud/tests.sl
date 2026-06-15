describe("posts_crud", fn() {
    test("creates a valid post", fn() {
        res = create_post({"title": "Hello", "body": "World"})
        assert_eq(res["status"], 201)
        assert_eq(res["title"], "Hello")
        assert_not_null(res["key"])
    })

    test("rejects an invalid post with 422", fn() {
        res = create_post({"body": "no title"})
        assert_eq(res["status"], 422)
        assert(len(res["errors"]) > 0)
    })

    test("show / update / delete round trip", fn() {
        created = create_post({"title": "First"})
        key = created["key"]

        assert_eq(show_post(key)["title"], "First")

        updated = update_post(key, {"title": "Second"})
        assert_eq(updated["status"], 200)
        assert_eq(updated["title"], "Second")

        assert_eq(delete_post(key)["status"], 204)
    })

    test("index counts posts", fn() {
        before = index_posts()["count"]
        create_post({"title": "Counted"})
        assert_eq(index_posts()["count"], before + 1)
    })
})
