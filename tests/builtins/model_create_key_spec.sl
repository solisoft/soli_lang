# Model.create(attrs, {"key": "..."}) — choosing the document key on create.
# The attributes hash never picks `_key` (request params must not choose ids);
# the separate options hash does.

class KeyedToken < Model
end

describe("Model.create options are checked") do
  test("an unknown option raises") do
    assert_raises("Model.create() got unknown option id (expected \"key\")") do
      KeyedToken.create({"name": "x"}, {"id": "nope"})
    end
  end

  test("an empty key raises") do
    assert_raises("option \"key\" must be a non-empty String, got string") do
      KeyedToken.create({"name": "x"}, {"key": ""})
    end
  end

  test("a non-string key raises") do
    assert_raises("option \"key\" must be a non-empty String, got array") do
      KeyedToken.create({"name": "x"}, {"key": [1]})
    end
  end

  test("options that are not a hash raise") do
    assert_raises("Model.create() options must be a Hash, got string") do
      KeyedToken.create({"name": "x"}, "key")
    end
  end
end

describe("Model.create with a chosen key") do
  before_each() do
    requires_solidb()
  end

  after_each() do
    KeyedToken.delete_all()
  end

  test("the key option names the stored document") do
    token = KeyedToken.create({"user": "joe"}, {"key": "joe_inbound_token"})

    assert_null(token._errors)
    assert_eq(token._key, "joe_inbound_token")
    assert_eq(token.id, "joe_inbound_token")
    assert_eq(KeyedToken.find_by("_key", "joe_inbound_token").user, "joe")
    assert_eq(KeyedToken.find("joe_inbound_token").user, "joe")
  end

  test("a duplicate key reports an error and keeps the first document") do
    KeyedToken.create({"user": "joe"}, {"key": "joe_inbound_token"})

    duplicate = KeyedToken.create({"user": "eve"}, {"key": "joe_inbound_token"})

    assert_eq(duplicate._errors, [{"field": "_base", "message": "has already been taken"}])
    assert_null(duplicate._key)
    assert_eq(KeyedToken.find("joe_inbound_token").user, "joe")
    assert_eq(KeyedToken.count, 1)
  end

  test("a _key in the attributes is still dropped") do
    token = KeyedToken.create({"_key": "client_chosen_key", "user": "mallory"})

    assert_null(token._errors)
    assert_ne(token._key, "client_chosen_key")
    assert_eq(KeyedToken.find(token._key).user, "mallory")
    assert_null(KeyedToken.find_by("_key", "client_chosen_key"))
  end
end
