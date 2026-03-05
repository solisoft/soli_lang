// ============================================================================
// Model Relations Test Suite
// Tests has_many, has_one, belongs_to DSL and query generation
// ============================================================================

// Define test models with relations at top level
class User extends Model
    has_many("posts")
    has_one("profile")
end

class Post extends Model
    belongs_to("user")
    has_many("comments")
end

class Comment extends Model
    belongs_to("post")
end

describe("Model Relations - includes", fn() {
    test("has_many generates subquery", fn() {
        let q = User.includes("posts").to_query;
        assert(q.contains("LET _rel_posts"));
        assert(q.contains("FOR rel IN posts FILTER rel.user_id == doc._key RETURN rel"));
        assert(q.contains("RETURN MERGE(doc, {posts: _rel_posts})"));
    });

    test("has_one generates LIMIT 1 subquery", fn() {
        let q = User.includes("profile").to_query;
        assert(q.contains("LET _rel_profile"));
        assert(q.contains("FILTER rel.user_id == doc._key LIMIT 1"));
        assert(q.contains("profile: FIRST(_rel_profile)"));
    });

    test("belongs_to uses doc FK to match rel._key", fn() {
        let q = Post.includes("user").to_query;
        assert(q.contains("FILTER rel._key == doc.user_id"));
        assert(q.contains("user: FIRST(_rel_user)"));
    });

    test("multiple relations", fn() {
        let q = User.includes("posts", "profile").to_query;
        assert(q.contains("_rel_posts"));
        assert(q.contains("_rel_profile"));
        assert(q.contains("RETURN MERGE"));
    });

    test("chained with where", fn() {
        let q = User.includes("posts").where("active = @a", { "a": true }).to_query;
        assert(q.contains("FILTER doc.active == @a"));
        assert(q.contains("LET _rel_posts"));
        assert(q.contains("RETURN MERGE"));
    });
});

describe("Model Relations - join", fn() {
    test("generates existence check", fn() {
        let q = User.join("posts").to_query;
        assert(q.contains("FILTER LENGTH(FOR rel IN posts FILTER rel.user_id == doc._key LIMIT 1 RETURN 1) > 0"));
        assert(q.contains("RETURN doc"));
    });

    test("with filter condition", fn() {
        let q = User.join("posts", "published = @p", { "p": true }).to_query;
        assert(q.contains("rel.published == @p"));
        assert(q.contains("rel.user_id == doc._key"));
    });
});

describe("Model Relations - chaining", fn() {
    test("includes then where", fn() {
        let q = User.includes("posts").where("name = @n", { "n": "Alice" }).to_query;
        assert(q.contains("FILTER doc.name == @n"));
        assert(q.contains("LET _rel_posts"));
    });

    test("where then includes", fn() {
        let q = User.where("active = @a", { "a": true }).includes("posts").to_query;
        assert(q.contains("FILTER doc.active == @a"));
        assert(q.contains("LET _rel_posts"));
    });

    test("join then where", fn() {
        let q = User.join("posts").where("active = @a", { "a": true }).to_query;
        assert(q.contains("FILTER LENGTH"));
        assert(q.contains("FILTER doc.active == @a"));
    });
});

describe("Model Relations - nested models", fn() {
    test("Comment belongs_to post", fn() {
        let q = Comment.includes("post").to_query;
        assert(q.contains("FILTER rel._key == doc.post_id"));
        assert(q.contains("FOR rel IN posts"));
    });

    test("Post has_many comments", fn() {
        let q = Post.includes("comments").to_query;
        assert(q.contains("FOR rel IN comments FILTER rel.post_id == doc._key"));
    });
});
