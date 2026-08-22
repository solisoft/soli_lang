describe("has_*_attached DSL", fn() {
    test("has_one_attached registers a disk uploader", fn() {
        class AttachedUser < Model
            has_one_attached("avatar")
        end
        cfg = model_uploader_config(AttachedUser, "avatar")
        assert(cfg != null)
        assert_eq(cfg["name"], "avatar")
        assert_eq(cfg["multiple"], false)
        assert_eq(cfg["service"], "disk")
        assert(cfg["max_size"] > 0)
    })

    test("has_many_attached can target s3", fn() {
        class AttachedAlbum < Model
            has_many_attached("photos", { "service": "s3", "max_size": 1_000_000 })
        end
        cfg = model_uploader_config(AttachedAlbum, "photos")
        assert_eq(cfg["multiple"], true)
        assert_eq(cfg["service"], "s3")
        assert_eq(cfg["max_size"], 1_000_000)
    })
})
