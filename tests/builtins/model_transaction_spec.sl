# Model.transaction: the block form commits when the block returns and rolls
# back (re-raising) when it throws; a string runs one SDBQL statement in a
# transaction; Model.transaction() hands back a handle for manual control.
#
# Query-builder reads (`where`, `all`, `count`) are not transactional: they
# observe committed state, which is what these specs use to check the outcome.

class TxAccount < Model
end

class TxFreshAccount < Model
end

def account_names
  TxAccount.all.map { |account| account.name }.sort()
end

def create_then_throw(name)
  TxAccount.transaction do
    TxAccount.create({"name": name, "balance": 50})
    throw "boom"
  end
end

def save_then_throw(account, balance)
  TxAccount.transaction do
    account.balance = balance
    account.save
    throw "boom"
  end
end

def delete_then_throw(account)
  TxAccount.transaction do
    account.delete
    throw "boom"
  end
end

def create_then_divide_by(name, divisor)
  TxAccount.transaction do
    TxAccount.create({"name": name, "balance": 1})
    1 / divisor
  end
end

# Two levels of Model.transaction; the inner one throws when asked to.
def nested_create(inner_throws)
  TxAccount.transaction do
    TxAccount.create({"name": "outer", "balance": 1})
    TxAccount.transaction do
      TxAccount.create({"name": "inner", "balance": 2})
      throw "boom" if inner_throws
    end
  end
end

describe("Model.transaction") do
  before_each() do
    requires_solidb()
    # A committed row, which also creates the collection: a write inside a
    # transaction does not create a missing one (see the pending spec below).
    TxAccount.create({"name": "existing", "balance": 10})
  end

  after_each() do
    TxAccount.delete_all()
    TxFreshAccount.delete_all()
  end

  describe("block form") do
    test("commits every write when the block completes normally") do
      TxAccount.transaction do
        TxAccount.create({"name": "alice", "balance": 100})
        TxAccount.create({"name": "bob", "balance": 50})
      end

      assert_eq(account_names(), ["alice", "bob", "existing"])
    end

    test("accepts an inline fn() literal as the block") do
      TxAccount.transaction(fn() { TxAccount.create({"name": "carol", "balance": 1}) })

      assert_eq(account_names(), ["carol", "existing"])
    end

    test("returns the block's value") do
      result = TxAccount.transaction do
        "committed"
      end

      assert_eq(result, "committed")
    end

    test("returns the record created by the block, persisted") do
      record = TxAccount.transaction do
        TxAccount.create({"name": "dave", "balance": 7})
      end

      assert_null(record._errors)
      assert_eq(TxAccount.find(record._key).name, "dave")
    end

    test("a query-builder read inside the block sees the pre-transaction state") do
      inside_count = TxAccount.transaction do
        TxAccount.create({"name": "erin", "balance": 3})
        TxAccount.count
      end

      assert_eq(inside_count, 1)
      assert_eq(TxAccount.count, 2)
    end

    test("find inside the block sees the transaction's own writes") do
      pending("bug: find inside a transaction raises RecordNotFound (no tx document GET route)")
      found_balance = TxAccount.transaction do
        record = TxAccount.create({"name": "frank", "balance": 4})
        TxAccount.find(record._key).balance
      end

      assert_eq(found_balance, 4)
    end

    test("a block held in a variable is refused with a hint to inline it") do
      pending("bug: Model.transaction(variable) fails with 'got int' instead of the inline-block hint")
      block = fn() { 1 }

      assert_raises("inline function literal") do
        TxAccount.transaction(block)
      end
    end
  end

  describe("rollback") do
    test("discards a create when the block throws, and re-raises the error") do
      message = assert_raises("boom") do
        create_then_throw("rolled_back")
      end

      assert_eq(message, "boom")
      assert_eq(account_names(), ["existing"])
    end

    test("discards a save to an existing record") do
      account = TxAccount.find_by("name", "existing")
      assert_raises("boom") do
        save_then_throw(account, 99)
      end

      assert_eq(TxAccount.find(account._key).balance, 10)
    end

    test("discards a delete") do
      account = TxAccount.find_by("name", "existing")
      assert_raises("boom") do
        delete_then_throw(account)
      end

      assert_eq(TxAccount.find(account._key).name, "existing")
    end

    test("a runtime error rolls back like a throw") do
      assert_raises("Division by zero") do
        create_then_divide_by("divided", 0)
      end

      assert_eq(account_names(), ["existing"])
    end

    test("nested transactions join the outer one: an inner throw rolls back both") do
      assert_raises("boom") do
        nested_create(true)
      end

      assert_eq(account_names(), ["existing"])
    end

    test("nested transactions commit together when nothing throws") do
      nested_create(false)

      assert_eq(account_names(), ["existing", "inner", "outer"])
    end
  end

  describe("on a collection that does not exist yet") do
    test("a create inside the block creates the collection like a plain create") do
      pending("bug: a create in a transaction on a missing collection fails: CollectionNotFound")
      record = TxFreshAccount.transaction do
        TxFreshAccount.create({"name": "first"})
      end

      assert_null(record._errors)
      assert_eq(TxFreshAccount.count, 1)
    end
  end

  describe("SDBQL string form") do
    test("runs a read and returns its rows") do
      assert_eq(TxAccount.transaction("FOR a IN tx_accounts RETURN a.name"), ["existing"])
    end

    test("commits a write statement") do
      TxAccount.transaction("FOR a IN tx_accounts UPDATE a WITH { balance: 50 } IN tx_accounts")

      assert_eq(TxAccount.find_by("name", "existing").balance, 50)
    end

    test("a malformed statement comes back as an error string") do
      assert_match(TxAccount.transaction("FOR a IN"), "^Error: Query failed")
    end
  end

  describe("arguments") do
    test("anything but a block, a string or nothing is refused") do
      assert_raises("expects SDBQL string or no arguments, got int") do
        TxAccount.transaction(5)
      end
    end
  end

  describe("transaction handle") do
    test("commit persists the handle's writes") do
      pending("bug: Model.transaction() returns a Class; tx.create/get/commit are unreachable")
      handle = TxAccount.transaction()
      handle.create({"name": "manual", "balance": 1})
      assert_eq(handle.commit(), true)

      assert_eq(account_names(), ["existing", "manual"])
    end

    test("rollback discards the handle's writes") do
      pending("bug: Model.transaction() returns a Class; tx.create/get/commit are unreachable")
      handle = TxAccount.transaction()
      handle.create({"name": "manual", "balance": 1})
      assert_eq(handle.rollback(), true)

      assert_eq(account_names(), ["existing"])
    end

    test("get, update and delete go through the handle") do
      pending("bug: Model.transaction() returns a Class; tx.create/get/commit are unreachable")
      key = TxAccount.find_by("name", "existing")._key
      handle = TxAccount.transaction()
      assert_eq(handle.get(key)["balance"], 10)
      handle.update(key, {"balance": 20})
      handle.commit()
      assert_eq(TxAccount.find(key).balance, 20)

      second = TxAccount.transaction()
      second.delete(key)
      second.commit()
      assert_null(TxAccount.find_by("_key", key))
    end
  end
end
