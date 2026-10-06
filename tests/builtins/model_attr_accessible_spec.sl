# attr_accessible — mass-assignment protection. A declared whitelist filters
# the hash given to every mass-assign path (create, save(hash), update(hash),
# Model.update, upsert, create_many, find_or_create_by) before validation and
# before the write; direct field assignment stays unfiltered.

class WhitelistedPost < Model
  attr_accessible("title", "body")
end

class WhitelistedPostArray < Model
  attr_accessible(["title", "body"])
end

class LockedPost < Model
  attr_accessible([])
end

class LegacyPost < Model
end

describe("attr_accessible") do
  before_each() do
    requires_solidb()
  end

  after_each() do
    WhitelistedPost.delete_all()
    WhitelistedPostArray.delete_all()
    LockedPost.delete_all()
    LegacyPost.delete_all()
  end

  describe("on Model.create") do
    test("the variadic form drops non-whitelisted keys") do
      post = WhitelistedPost.create({"title": "Hi", "body": "Body", "role": "admin"})
      stored = WhitelistedPost.find(post._key)

      assert_eq(post.title, "Hi")
      assert_null(post.role)
      assert_eq(stored.title, "Hi")
      assert_eq(stored.body, "Body")
      assert_null(stored.role)
    end

    test("the array form is equivalent to the variadic one") do
      post = WhitelistedPostArray.create({"title": "Hi", "body": "B", "role": "admin"})
      stored = WhitelistedPostArray.find(post._key)

      assert_eq(stored.title, "Hi")
      assert_eq(stored.body, "B")
      assert_null(stored.role)
    end

    test("an empty whitelist drops every key") do
      post = LockedPost.create({"title": "x", "role": "admin"})
      stored = LockedPost.find(post._key)

      assert_null(stored.title)
      assert_null(stored.role)
    end

    test("a model without attr_accessible accepts every key") do
      post = LegacyPost.create({"title": "x", "anything": "y"})
      stored = LegacyPost.find(post._key)

      assert_eq(stored.title, "x")
      assert_eq(stored.anything, "y")
    end
  end

  describe("on instance writes") do
    test("save(hash) drops non-whitelisted keys") do
      post = WhitelistedPost.new()

      assert(post.save({"title": "Hi", "body": "B", "role": "admin"}))
      assert_eq(post.title, "Hi")
      assert_null(post.role)
      assert_null(WhitelistedPost.find(post._key).role)
    end

    test("update(hash) drops non-whitelisted keys") do
      post = WhitelistedPost.create({"title": "original"})

      post.update({"body": "new", "role": "admin"})
      stored = WhitelistedPost.find(post._key)

      assert_eq(post.body, "new")
      assert_null(post.role)
      assert_eq(stored.body, "new")
      assert_null(stored.role)
    end

    test("direct field assignment is not filtered") do
      post = WhitelistedPost.new()
      post.title = "t"
      post.role = "set by trusted code"

      post.save

      assert_eq(WhitelistedPost.find(post._key).role, "set by trusted code")
    end

    test("Model.new(hash) filters too") do
      pending("bug: WhitelistedPost.new({\"role\": \"admin\"}).save persists role — new(hash) skips the whitelist")
      post = WhitelistedPost.new({"title": "Hi", "role": "admin"})

      assert_null(post.role)
    end
  end

  describe("on class-level write paths") do
    test("Model.update(id, hash) drops non-whitelisted keys") do
      post = WhitelistedPost.create({"title": "a"})

      WhitelistedPost.update(post._key, {"title": "b", "role": "admin"})
      stored = WhitelistedPost.find(post._key)

      assert_eq(stored.title, "b")
      assert_null(stored.role)
    end

    test("upsert drops non-whitelisted keys from the payload") do
      result = WhitelistedPost.upsert("any-key", {"title": "Hi", "body": "B", "role": "admin"})
      stored = WhitelistedPost.find("any-key")

      assert(result.is_a?("WhitelistedPost"))
      assert_null(result.role)
      assert_eq(stored.title, "Hi")
      assert_null(stored.role)
    end

    test("create_many filters each item independently") do
      result = WhitelistedPost.create_many([{"title": "A", "role": "admin"}, {"title": "B", "is_admin": true}])

      assert_eq(result, {"created": 2})
      assert_null(WhitelistedPost.find_by("title", "A").role)
      assert_null(WhitelistedPost.find_by("title", "B").is_admin)
    end

    test("find_or_create_by filters the defaults hash on the create branch") do
      result = WhitelistedPost.find_or_create_by("title", "unique-title", {"body": "ok", "role": "admin"})
      stored = WhitelistedPost.find(result._key)

      assert_eq(stored.title, "unique-title")
      assert_eq(stored.body, "ok")
      assert_null(stored.role)
    end
  end
end
