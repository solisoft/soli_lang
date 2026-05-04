// ============================================================================
// Test: Ruby-style `scope` DSL inside Model class bodies. Inside the closure
// `this` is bound to a fresh QueryBuilder for the model, so user code reads:
//
//   class Post < Model
//     scope("published", fn() { this.where("status = @s", { "s": "published" }) })
//   end
//
// The class is auto-prepended to the call by execute_class, so the user
// doesn't repeat the class name.
// ============================================================================

class Article extends Model
    scope("published", fn() { this.where("status = @s", { "s": "published" }) })
    scope("recent",    fn() { this.order("created_at", "desc").limit(10) })
    scope("identity",  fn() { this })
end

describe("scope (class-body DSL)", fn() {
    test("scope with this.where(filter, binds) returns a QueryBuilder", fn() {
        let qb = Article.published
        assert_eq(qb.class, "query_builder")
    });

    test("scope with this.order(...).limit(...) chains correctly", fn() {
        let qb = Article.recent
        assert_eq(qb.class, "query_builder")
    });

    test("identity scope (returns this) is a QueryBuilder", fn() {
        let qb = Article.identity
        assert_eq(qb.class, "query_builder")
    });

    test("scopes compose: Article.published.recent stays a QueryBuilder", fn() {
        let qb = Article.published.recent
        assert_eq(qb.class, "query_builder")
    });
});
