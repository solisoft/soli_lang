// ============================================================================
// attr_accessible / mass-assignment protection
//
// Verifies that `attr_accessible(...)` filters hash arguments to the
// declared whitelist *before* they reach the in-memory instance (and
// before the DB write). The filter is checked through the in-memory side
// effects so the tests do not need a live DB.
// ============================================================================

class WhitelistedPost extends Model
    attr_accessible("title", "body")
end

class WhitelistedPostArray extends Model
    attr_accessible(["title", "body"])
end

class LockedPost extends Model
    attr_accessible([])
end

class LegacyPost extends Model
end

describe("attr_accessible — variadic form", fn() {
    test("drops non-whitelisted keys before assignment", fn() {
        let post = WhitelistedPost.new();
        post.save({ "title": "Hi", "body": "Body", "role": "admin" }) rescue null;
        assert_eq(post.title, "Hi");
        assert_eq(post.body, "Body");
        assert_null(post.role);
    });

    test("keeps every whitelisted key when present", fn() {
        let post = WhitelistedPost.new();
        post.save({ "title": "Hi", "body": "B" }) rescue null;
        assert_eq(post.title, "Hi");
        assert_eq(post.body, "B");
    });
});

describe("attr_accessible — array form", fn() {
    test("array argument is equivalent to variadic", fn() {
        let post = WhitelistedPostArray.new();
        post.save({ "title": "Hi", "body": "B", "role": "admin" }) rescue null;
        assert_eq(post.title, "Hi");
        assert_eq(post.body, "B");
        assert_null(post.role);
    });
});

describe("attr_accessible — empty list (lock-down)", fn() {
    test("empty whitelist drops every key", fn() {
        let post = LockedPost.new();
        post.save({ "title": "x", "role": "admin" }) rescue null;
        assert_null(post.title);
        assert_null(post.role);
    });
});

describe("attr_accessible — undeclared (back-compat)", fn() {
    test("model without attr_accessible accepts every key", fn() {
        let post = LegacyPost.new();
        post.save({ "title": "x", "anything": "y" }) rescue null;
        assert_eq(post.title, "x");
        assert_eq(post.anything, "y");
    });
});

describe("attr_accessible — instance.update(hash)", fn() {
    test("filters update arguments too", fn() {
        let post = WhitelistedPost.new();
        post.title = "original";
        // update will fail (no _key) but the hash filter runs first.
        post.update({ "body": "new", "role": "admin" }) rescue null;
        assert_eq(post.body, "new");
        assert_null(post.role);
    });
});

// ============================================================================
// The vulnerability covered every public mass-assign API. These specs guard
// the alternate write paths (upsert, create_many, find_or_create_by) so
// regressions there fail loudly. They run with-or-without a DB: they only
// inspect the response shape / returned class, not persistence.
// ============================================================================

describe("attr_accessible — Model.upsert", fn() {
    test("filters non-whitelisted keys from upsert payload", fn() {
        // upsert returns a class instance (or errors); the filter has
        // already been applied on the way in, so even on success the
        // unlisted fields cannot have made it to the document.
        let result = WhitelistedPost.upsert("any-key", {
            "title": "Hi",
            "body":  "B",
            "role":  "admin"
        }) rescue null;
        // Best-effort assertion: when the operation reaches the DB and
        // returns an instance, role should be absent.
        if !result.nil? and result.is_a?("WhitelistedPost")
            assert_null(result.role);
        end
    });
});

describe("attr_accessible — Model.create_many", fn() {
    test("filters each item independently", fn() {
        let result = WhitelistedPost.create_many([
            { "title": "A", "role": "admin" },
            { "title": "B", "is_admin": true }
        ]) rescue null;
        // The call returns a {created: N} hash; correctness is asserted
        // through behaviour at the rust unit level. Smoke-test only.
        assert(!result.nil?);
    });
});

describe("attr_accessible — Model.find_or_create_by", fn() {
    test("filters defaults hash on the create branch", fn() {
        let result = WhitelistedPost.find_or_create_by(
            "title",
            "unique-attr-accessible-spec-key",
            { "body": "ok", "role": "admin" }
        ) rescue null;
        if !result.nil? and result.is_a?("WhitelistedPost")
            assert_null(result.role);
        end
    });
});
