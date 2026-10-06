# Session builtins against the default in_memory driver, which works without a
# server: get/set/has/delete, ids, regenerate, destroy, configuration, and the
# create_session / destroy_session / with_session test helpers.

describe("session store") do
  # Every test starts from an empty session with a live id
  before_each() do
    session_destroy()
    session_regenerate()
  end

  context("session_set / session_get") do
    test("round-trips an int, a string, a hash and an array") do
      session_set("count", 42)
      session_set("greeting", "hello session")
      session_set("user", {"name": "Alice", "admin": true})
      session_set("ids", [1, 2, 3])
      assert_eq(session_get("count"), 42)
      assert_eq(session_get("greeting"), "hello session")
      assert_eq(session_get("user"), {"name": "Alice", "admin": true})
      assert_eq(session_get("ids"), [1, 2, 3])
    end

    test("overwrites an existing key") do
      session_set("step", "first")
      session_set("step", "second")
      assert_eq(session_get("step"), "second")
    end

    test("returns nil for a missing key") do
      assert_null(session_get("never_set"))
    end

    test("stores nil as a present key") do
      session_set("cleared", nil)
      assert(session_has("cleared"))
      assert_null(session_get("cleared"))
    end

    test("refuses a key that is not a string") do
      assert_raises("session_set() expects string key, got int") do
        session_set(1, 2)
      end
    end
  end

  context("session_has") do
    test("is true for a stored key and false for an unknown one") do
      session_set("present", "yes")
      assert(session_has("present"))
      assert_not(session_has("missing"))
    end

    test("is false after the key is deleted") do
      session_set("doomed", 1)
      session_delete("doomed")
      assert_not(session_has("doomed"))
    end
  end

  context("session_delete") do
    test("returns the deleted value and removes the key") do
      session_set("payload", "data")
      assert_eq(session_delete("payload"), "data")
      assert_null(session_get("payload"))
    end

    test("returns nil for an unknown key and on a second delete") do
      assert_null(session_delete("unknown"))
      session_set("once", 7)
      assert_eq(session_delete("once"), 7)
      assert_null(session_delete("once"))
    end
  end

  context("session_id") do
    test("is a UUID") do
      session_set("lazy", true)
      assert_match(session_id(), "^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")
    end

    test("stays stable across writes") do
      session_set("a", 1)
      first = session_id()
      session_set("b", 2)
      assert_eq(session_id(), first)
    end
  end

  context("session_regenerate") do
    test("mints and returns a new id") do
      session_set("marker", "before")
      old_id = session_id()
      new_id = session_regenerate()
      assert_ne(new_id, old_id)
      assert_eq(session_id(), new_id)
    end

    test("carries the data over to the new id") do
      session_set("data", "carried over")
      session_regenerate()
      assert_eq(session_get("data"), "carried over")
    end
  end

  context("session_destroy") do
    test("clears all session data") do
      session_set("doomed", "value")
      session_destroy()
      assert_null(session_get("doomed"))
      assert_not(session_has("doomed"))
    end

    test("after a regenerate, writes land again") do
      session_destroy()
      session_regenerate()
      session_set("reborn", "yes")
      assert_eq(session_get("reborn"), "yes")
    end

    test("a write right after destroy lands in a fresh session") do
      pending("bug: after session_destroy(), session_set is silently dropped until session_regenerate()")
      session_destroy()
      session_set("after_destroy", 1)
      assert_eq(session_get("after_destroy"), 1)
    end
  end
end

describe("session configuration") do
  after_each() do
    session_configure({"driver": "in_memory"})
  end

  test("session_driver defaults to in_memory") do
    assert_eq(session_driver(), "in_memory")
  end

  test("session_config reports the driver and the default one-day TTL") do
    assert_eq(session_config(), {"driver": "in_memory", "ttl": 86400})
  end

  test("session_configure accepts options and reports success") do
    assert(session_configure({"driver": "in_memory"}))
    assert_eq(session_config()["driver"], "in_memory")
  end

  test("session_configure refuses an unknown driver") do
    assert_raises("Unknown session driver: bogus") do
      session_configure({"driver": "bogus"})
    end
  end
end

describe("session test helpers") do
  after_each() do
    destroy_session()
  end

  test("destroy_session signs out the test user and returns nil") do
    create_session(42)
    assert_null(destroy_session())
    assert(signed_out())
  end

  test("create_session returns a named test session id for a user id") do
    assert_eq(create_session(42), "session_test_42")
  end

  test("create_session marks the caller signed in until destroyed") do
    create_session(42)
    assert(signed_in())
    assert_not(signed_out())
    destroy_session()
    assert(signed_out())
  end

  test("with_session signs the test client in") do
    destroy_session()
    with_session({"user_id": 42, "role": "editor"})
    assert(signed_in())
  end

  test("with_session refuses a non-hash argument") do
    assert_raises("with_session expects a hash, got int") do
      with_session(7)
    end
  end
end
