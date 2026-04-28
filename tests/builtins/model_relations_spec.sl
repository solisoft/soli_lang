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

class Tag extends Model
    has_and_belongs_to_many("posts")
end

class Article extends Model
    has_and_belongs_to_many("labels", { "class_name": "Tag" })
end

class PostHabtm extends Model
    has_and_belongs_to_many("tags")
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

describe("Model Relations - filtered includes", fn() {
    test("has_many with filter", fn() {
        let q = User.includes("posts", "published = @p", { "p": true }).to_query;
        assert(q.contains("rel.user_id == doc._key AND rel.published == @p"));
        assert(q.contains("RETURN rel)"));
        assert(q.contains("bind_vars"));
    });

    test("has_many with filter and fields", fn() {
        let q = User.includes("posts", "published = @p", { "p": true, "fields": ["title", "body"] }).to_query;
        assert(q.contains("rel.user_id == doc._key AND rel.published == @p"));
        assert(q.contains("RETURN {title: rel.title, body: rel.body}"));
    });

    test("hash arg includes with fields", fn() {
        let q = User.includes({ "posts": ["title", "body"] }).to_query;
        assert(q.contains("RETURN {title: rel.title, body: rel.body}"));
        assert(q.contains("RETURN MERGE(doc"));
    });

    test("hash arg includes without fields", fn() {
        let q = User.includes({ "posts": nil }).to_query;
        assert(q.contains("RETURN rel)"));
    });

    test("multiple chained includes", fn() {
        let q = User.includes("posts", "published = @p", { "p": true }).includes("profile").to_query;
        assert(q.contains("_rel_posts"));
        assert(q.contains("_rel_profile"));
        assert(q.contains("rel.published == @p"));
        assert(q.contains("RETURN MERGE"));
    });
});

describe("Model Relations - select/fields", fn() {
    test("select on main collection", fn() {
        let q = User.select("name", "email").to_query;
        assert(q.contains("RETURN {name: doc.name, email: doc.email, _key: doc._key}"));
    });

    test("fields alias works same as select", fn() {
        let q = User.fields("name", "email").to_query;
        assert(q.contains("RETURN {name: doc.name, email: doc.email, _key: doc._key}"));
    });

    test("select with includes", fn() {
        let q = User.select("name", "email").includes("posts").to_query;
        assert(q.contains("RETURN MERGE({name: doc.name, email: doc.email, _key: doc._key}, {posts: _rel_posts})"));
    });

    test("select chained after where", fn() {
        let q = User.where("active = @a", { "a": true }).select("name").to_query;
        assert(q.contains("FILTER doc.active == @a"));
        assert(q.contains("RETURN {name: doc.name, _key: doc._key}"));
    });

    test("select with filtered includes and fields", fn() {
        let q = User.select("name").includes("posts", "published = @p", { "p": true, "fields": ["title"] }).to_query;
        assert(q.contains("RETURN MERGE({name: doc.name, _key: doc._key}"));
        assert(q.contains("RETURN {title: rel.title}"));
        assert(q.contains("rel.published == @p"));
    });
});

describe("Model Relations - has_and_belongs_to_many", fn() {
    test("habtm includes generates two-stage subquery", fn() {
        let q = PostHabtm.includes("tags").to_query;
        assert(q.contains("LET _rel_tags"));
        assert(q.contains("FOR jt IN post_habtms_tags FILTER jt.post_habtm_id == doc._key"));
        assert(q.contains("FOR rel IN tags FILTER rel._key == jt.tag_id"));
        assert(q.contains("RETURN MERGE(doc, {tags: _rel_tags})"));
    });

    test("habtm join generates existence check via join table", fn() {
        let q = PostHabtm.join("tags").to_query;
        assert(q.contains("FILTER LENGTH(FOR jt IN post_habtms_tags"));
        assert(q.contains("FOR rel IN tags FILTER rel._key == jt.tag_id"));
    });

    test("habtm with filter on related collection", fn() {
        let q = PostHabtm.includes("tags", "active = @a", { "a": true }).to_query;
        assert(q.contains("FOR jt IN post_habtms_tags FILTER jt.post_habtm_id == doc._key"));
        assert(q.contains("rel._key == jt.tag_id AND rel.active == @a"));
    });

    test("habtm join table uses alphabetical order on Tag side", fn() {
        let q = Tag.includes("posts").to_query;
        assert(q.contains("FOR jt IN posts_tags FILTER jt.tag_id == doc._key"));
        assert(q.contains("FOR rel IN posts FILTER rel._key == jt.post_id"));
    });

    test("habtm with class_name override uses override class", fn() {
        let q = Article.includes("labels").to_query;
        // Article ↔ Tag through the alphabetical join "articles_tags"
        assert(q.contains("FOR jt IN articles_tags FILTER jt.article_id == doc._key"));
        assert(q.contains("FOR rel IN tags FILTER rel._key == jt.tag_id"));
        assert(q.contains("RETURN MERGE(doc, {labels: _rel_labels})"));
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
