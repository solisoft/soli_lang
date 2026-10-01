# ============================================================================
# Model.create(attrs, {"key": "..."}) — choosing the document key on create.
# The attributes hash never picks `_key` (request params must not choose ids);
# the separate options hash does. Persistence paths are gated behind the DB
# availability probe, matching model_dirty_tracking_spec.sl.
# ============================================================================
class KeyedToken < Model
end

def raises?(body)
  try
    body()
  catch e
    return true
  end
  false
end

__db_available = false
try
  __probe = KeyedToken.create({"name": "__probe__"})
  if !__probe.nil? && !__probe._errors
    __db_available = true
    __probe.delete
  end
catch e
  __db_available = false
end

describe("Model.create with a chosen key", fn() {
  test("an unknown option raises", fn() {
    assert(raises?(fn() { KeyedToken.create({"name": "x"}, {"id": "nope"}) }))
  })

  test("a non-string key raises", fn() {
    assert(raises?(fn() { KeyedToken.create({"name": "x"}, {"key": ""}) }))
    assert(raises?(fn() { KeyedToken.create({"name": "x"}, {"key": [1]}) }))
  })

  test("the key option names the stored document", fn() {
    return nil unless __db_available

    key = "joe_inbound_token_#{DateTime.utc.to_unix}"
    token = KeyedToken.create({"user": "joe"}, {"key": key})
    assert_null(token._errors)
    assert_eq(token._key, key)
    assert_eq(token.id, key)
    assert_eq(KeyedToken.find_by("_key", key).user, "joe")
    assert_eq(KeyedToken.find(key).user, "joe")

    duplicate = KeyedToken.create({"user": "eve"}, {"key": key})
    assert(duplicate._errors.length > 0)
    assert_eq(KeyedToken.find(key).user, "joe")
    token.delete
  })

  test("a _key in the attributes is still dropped", fn() {
    return nil unless __db_available

    token = KeyedToken.create({"_key": "client_chosen_key", "user": "mallory"})
    assert_null(token._errors)
    assert(token._key != "client_chosen_key")
    assert_null(KeyedToken.find_by("_key", "client_chosen_key"))
    token.delete
  })
})
