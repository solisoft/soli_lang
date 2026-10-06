# Factory: hash and callable templates, overrides, the #{n} sequence marker,
# lists, and insert through a bound model (the only part that needs SoliDB).

class FactoryTestUser < Model
end

class FactoryPlainClass
end

describe("Factory") do
  before_each() do
    Factory.clear()
  end

  describe("create") do
    test("static hash factories merge overrides") do
      Factory.define("user", {"email": "base@test.com", "name": "Base"})
      user = Factory.create_with("user", {"name": "Override"})
      assert_eq(user["email"], "base@test.com")
      assert_eq(user["name"], "Override")
    end

    test("an override can add a key the template lacks") do
      Factory.define("user", {"name": "Base"})
      user = Factory.create_with("user", {"role": "admin"})
      assert_eq(user, {"name": "Base", "role": "admin"})
    end

    test("callable factories run on each create") do
      counter = 0
      Factory.define("user", fn() {
        counter = counter + 1
        {"n": counter}
      })
      first = Factory.create("user")
      second = Factory.create("user")
      assert_eq(first["n"], 1)
      assert_eq(second["n"], 2)
    end

    test("an undefined factory raises with its name") do
      assert_raises("Factory 'ghost' not defined") do
        Factory.create("ghost")
      end
    end
  end

  describe("sequences") do
    test("interpolates the sequence number into string attributes") do
      Factory.define("user", {"email": r"user#{n}@test.com"})
      first = Factory.create("user")
      second = Factory.create("user")
      assert_eq(first["email"], "user0@test.com")
      assert_eq(second["email"], "user1@test.com")
    end

    test("interpolates inside nested hashes and arrays") do
      Factory.define("user", {"profile": {"handle": r"h#{n}"}, "tags": [r"t#{n}"]})
      user = Factory.create("user")
      assert_eq(user["profile"]["handle"], "h0")
      assert_eq(user["tags"], ["t0"])
    end

    test("clear restarts the sequence and forgets definitions") do
      Factory.define("user", {"email": r"u#{n}"})
      Factory.create("user")
      Factory.clear()
      assert_raises("not defined") do
        Factory.create("user")
      end
      Factory.define("user", {"email": r"u#{n}"})
      assert_eq(Factory.create("user")["email"], "u0")
    end
  end

  describe("create_list") do
    test("builds count records, each with its own sequence number") do
      Factory.define("user", {"email": r"u#{n}@test.com"})
      users = Factory.create_list("user", 3)
      assert_eq(users.map { |user| user["email"] }, ["u0@test.com", "u1@test.com", "u2@test.com"])
    end

    test("a count of zero builds an empty list") do
      Factory.define("user", {"email": "x"})
      assert_eq(Factory.create_list("user", 0), [])
    end
  end

  describe("bind and insert errors") do
    test("insert on an unbound factory says to bind it first") do
      Factory.define("user", {"email": "x"})
      assert_raises("call Factory.bind(name, ModelClass) first") do
        Factory.insert("user")
      end
    end

    test("bind refuses a class that is not a model") do
      message = assert_raises("expects a Model subclass") do
        Factory.bind("user", FactoryPlainClass)
      end
      assert_contains(message, "FactoryPlainClass")
    end

    test("bind refuses a value that is not a class") do
      assert_raises("expects a Model class, got") do
        Factory.bind("user", "FactoryTestUser")
      end
    end
  end
end

describe("Factory.insert with SoliDB") do
  before_each() do
    requires_solidb()
    Factory.clear()
    # Nothing resets the database between runs, so start from a known state.
    FactoryTestUser.all.each do |leftover|
      leftover.delete
    end
  end

  after_each() do
    FactoryTestUser.all.each do |leftover|
      leftover.delete
    end
  end

  test("insert persists through the bound model") do
    Factory.define("user", {"email": "persist@test.com"})
    Factory.bind("user", FactoryTestUser)
    record = Factory.insert("user")
    # `all` rather than `count`: SoliDB's COLLECTION_COUNT is not decremented
    # on delete, so it reports rows a scan of the collection does not return.
    assert_eq(FactoryTestUser.all.length, 1)
    assert_eq(record.email, "persist@test.com")
  end

  test("insert applies overrides before saving") do
    Factory.define("user", {"email": "base@test.com"})
    Factory.bind("user", FactoryTestUser)
    record = Factory.insert("user", {"email": "override@test.com"})
    assert_eq(FactoryTestUser.find(record.id).email, "override@test.com")
  end
end
