// ============================================================================
// Test: Ruby-style `scope` DSL inside Model class bodies.
//   class User < Model
//     scope("published", fn(qb) { qb })
//   end
// The class is auto-prepended to the call by execute_class, so the user
// doesn't repeat the class name.
// ============================================================================

class Article extends Model
    scope("published", fn(qb) { qb })
    scope("draft", fn(qb) { qb })
end

describe("scope (class-body DSL)", fn() {
    test("registered scope is reachable on the model class", fn() {
        let qb = Article.published
        assert_eq(qb.nil?, false)
    });

    test("scope name accepts string form", fn() {
        let qb = Article.draft
        assert_eq(qb.nil?, false)
    });
});
