# In a real app `Post` is auto-loaded from app/models/. Here we define a stand-in
# so the controller action can be exercised without a database.
class Post {
    static def all {
        return [{"title": "First Post"}, {"title": "Second Post"}];
    }
}

describe("no_redundant_model_import", fn() {
    test("returns post titles", fn() {
        assert_eq(post_titles, ["First Post", "Second Post"]);
    });
});
