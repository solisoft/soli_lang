# Model query surface beyond plain CRUD: pluck, exists, aggregates, offset,
# pagination, finders, upsert, create_many, transactions, increment/decrement,
# touch and soft delete. Query-shape tests read `.to_query` and need no
# database; the rest run against SoliDB.

class AdvUser < Model
  scope("big_spenders", fn() { this.where("value > @v", {"v": 100}) })
end

class AdvSoft < Model
  soft_delete
end

# The SDBQL text of a builder, without the bind-variable dump that follows it.
def query_of(builder)
  builder.to_query.split(" | bind_vars:")[0]
end

const AGGREGATE_BUG = "bug: Model.sum/avg/min/max emit `FOR doc IN c RETURN SUM(doc.f)`, which SoliDB rejects " +
  "(Unknown function: SUM); the error is swallowed and .first is nil"

def create_then_throw(name)
  AdvUser.transaction do
    AdvUser.create({"name": name})
    throw "boom"
  end
end

def names_of(records)
  records.map { |record| record.name }
end

describe("query generation") do
  describe("exists") do
    test("stops at the first match and returns true") do
      query = query_of(AdvUser.where("active = @a", {"a": true}).exists)
      assert_eq(query, "FOR doc IN adv_users FILTER doc.active == @a LIMIT 1 RETURN true")
    end
  end

  describe("pluck") do
    test("one field returns the bare value") do
      query = query_of(AdvUser.where("active = @a", {"a": true}).pluck("name"))
      assert_eq(query, "FOR doc IN adv_users FILTER doc.active == @a RETURN doc.name")
    end

    test("several fields return a hash per row") do
      query = query_of(AdvUser.pluck("name", "email"))
      assert_eq(query, "FOR doc IN adv_users RETURN {name: doc.name, email: doc.email}")
    end
  end

  describe("symbols as field names") do
    test("pluck accepts symbols") do
      assert_eq(query_of(AdvUser.pluck(:name, :email)), query_of(AdvUser.pluck("name", "email")))
    end

    test("chained pluck accepts symbols") do
      query = query_of(AdvUser.where("active = @a", {"a": true}).pluck(:name))
      assert_eq(query, "FOR doc IN adv_users FILTER doc.active == @a RETURN doc.name")
    end

    test("order accepts a symbol field and direction") do
      assert_contains(query_of(AdvUser.order(:created_at, :desc)), "SORT doc.created_at DESC")
    end

    test("select accepts symbols and always keeps _key") do
      query = query_of(AdvUser.select(:name, :email))
      assert_contains(query, "name: doc.name")
      assert_contains(query, "email: doc.email")
      assert_contains(query, "_key: doc._key")
    end

    test("aggregates accept symbols") do
      assert_eq(query_of(AdvUser.sum(:balance)), query_of(AdvUser.sum("balance")))
    end
  end

  describe("aggregates") do
    test("sum keeps the where filter") do
      query = query_of(AdvUser.where("age > @a", {"a": 18}).sum("balance"))
      assert_contains(query, "FILTER doc.age > @a")
      assert_contains(query, "SUM(doc.balance)")
    end

    test("avg, min and max name their function") do
      assert_contains(query_of(AdvUser.avg("score")), "AVG(doc.score)")
      assert_contains(query_of(AdvUser.min("price")), "MIN(doc.price)")
      assert_contains(query_of(AdvUser.max("views")), "MAX(doc.views)")
    end

    test("group_by with a function collects per group") do
      query = query_of(AdvUser.group_by("country", "sum", "balance"))
      assert_contains(query, "COLLECT group = doc.country")
      assert_contains(query, "AGGREGATE result = SUM(doc.balance)")
    end
  end

  describe("where") do
    test("accepts a filter without bind variables") do
      query = query_of(AdvUser.where("doc.active == true"))
      assert_eq(query, "FOR doc IN adv_users FILTER doc.active == true RETURN doc")
    end

    test("accepts an empty bind-variable hash") do
      assert_eq(query_of(AdvUser.where("doc.active == true", {})), query_of(AdvUser.where("doc.active == true")))
    end

    test("passes LOWER, UPPER and TRIM through") do
      ["LOWER(doc.email) == @x", "UPPER(doc.name) == @x", "TRIM(doc.field) == @x"].each do |filter|
        assert_contains(query_of(AdvUser.where(filter, {"x": "value"})), "FILTER #{filter}")
      end
    end

    test("passes a function applied to a bind variable through") do
      query = query_of(AdvUser.where("LOWER(doc.email) == LOWER(@email)", {"email": "Test@Example.COM"}))
      assert_contains(query, "FILTER LOWER(doc.email) == LOWER(@email)")
    end
  end

  describe("offset") do
    test("becomes the first LIMIT argument") do
      assert_contains(query_of(AdvUser.offset(20)), "LIMIT 20,")
    end

    test("chains after where") do
      query = query_of(AdvUser.where("active = @a", {"a": true}).offset(10))
      assert_contains(query, "FILTER doc.active == @a")
      assert_contains(query, "LIMIT 10,")
    end
  end

  describe("scopes") do
    test("a declared scope is a class method returning its filter") do
      query = query_of(AdvUser.big_spenders)
      assert_eq(query, "FOR doc IN adv_users FILTER doc.value > @v RETURN doc")
    end
  end

  describe("soft-delete modes") do
    test("a soft-delete model hides deleted rows by default") do
      assert_contains(query_of(AdvSoft.where("name = @n", {"n": "x"})), "FILTER doc.deleted_at == null")
    end

    test("with_deleted drops the guard") do
      assert_eq(query_of(AdvSoft.with_deleted()), "FOR doc IN adv_softs RETURN doc")
    end

    test("only_deleted inverts it") do
      assert_eq(query_of(AdvSoft.only_deleted()), "FOR doc IN adv_softs FILTER doc.deleted_at != null RETURN doc")
    end
  end
end

describe("against SoliDB") do
  before_each() do
    requires_solidb()
  end

  after_each() do
    AdvUser.delete_all()
    AdvSoft.delete_all()
  end

  describe("create_many") do
    test("inserts every row and reports the count") do
      batch = AdvUser.create_many([
        {"name": "Batch1", "value": 1},
        {"name": "Batch2", "value": 2},
        {"name": "Batch3", "value": 3}
      ])
      assert_eq(batch["created"], 3)
      assert_eq(names_of(AdvUser.order("value").all), ["Batch1", "Batch2", "Batch3"])
    end

    test("an empty batch creates nothing") do
      assert_eq(AdvUser.create_many([])["created"], 0)
      assert_eq(AdvUser.count, 0)
    end
  end

  describe("find_by") do
    test("finds by field value") do
      created = AdvUser.create({"name": "FindByTest", "value": 42})
      found = AdvUser.find_by("name", "FindByTest")
      assert_eq(found._key, created._key)
      assert_eq(found.value, 42)
    end

    test("returns nil for a missing record") do
      assert_null(AdvUser.find_by("name", "NonExistent12345"))
    end
  end

  describe("dynamic find_by_* finders") do
    before_each() do
      AdvUser.create({"name": "Dyn", "role": "admin", "active": true})
    end

    test("one field") do
      assert_eq(AdvUser.find_by_name("Dyn").role, "admin")
    end

    test("two fields are ANDed") do
      assert_eq(AdvUser.find_by_name_and_active("Dyn", true).name, "Dyn")
      assert_null(AdvUser.find_by_name_and_active("Dyn", false))
    end

    test("three fields are ANDed") do
      assert_eq(AdvUser.find_by_name_and_active_and_role("Dyn", true, "admin").name, "Dyn")
      assert_null(AdvUser.find_by_name_and_active_and_role("Dyn", true, "guest"))
    end

    test("no match returns nil") do
      assert_null(AdvUser.find_by_name("nobody"))
    end
  end

  describe("first_by") do
    test("returns the match with the lowest key") do
      AdvUser.create({"name": "FirstByTest", "value": 2}, {"key": "first_by_b"})
      AdvUser.create({"name": "FirstByTest", "value": 1}, {"key": "first_by_a"})
      found = AdvUser.first_by("name", "FirstByTest")
      assert_eq(found._key, "first_by_a")
      assert_eq(found.value, 1)
    end

    test("returns nil without a match") do
      assert_null(AdvUser.first_by("name", "nobody"))
    end
  end

  describe("find_or_create_by") do
    test("returns the existing record untouched") do
      created = AdvUser.create({"name": "FindOrCreate", "value": 100})
      found = AdvUser.find_or_create_by("name", "FindOrCreate", {"value": 999})
      assert_eq(found._key, created._key)
      assert_eq(found.value, 100)
      assert_eq(AdvUser.count, 1)
    end

    test("creates the record from the field and the extra data") do
      found = AdvUser.find_or_create_by("name", "FindOrCreateNew", {"value": 555})
      assert_eq(found.name, "FindOrCreateNew")
      assert_eq(found.value, 555)
      assert_eq(AdvUser.find(found._key).value, 555)
    end
  end

  describe("upsert") do
    test("inserts under the given key when it is free") do
      record = AdvUser.upsert("upsert_key_123", {"name": "UpsertNew", "value": 1})
      assert_eq(record._key, "upsert_key_123")
      assert_eq(AdvUser.find("upsert_key_123").name, "UpsertNew")
    end

    test("replaces the document when the key exists") do
      AdvUser.upsert("upsert_key_456", {"name": "Before", "value": 1})
      AdvUser.upsert("upsert_key_456", {"name": "After", "value": 2})
      found = AdvUser.find("upsert_key_456")
      assert_eq(found.name, "After")
      assert_eq(found.value, 2)
      assert_eq(AdvUser.count, 1)
    end
  end

  describe("transaction") do
    test("commits the block's writes") do
      AdvUser.transaction do
        AdvUser.create({"name": "tx_a"})
        AdvUser.create({"name": "tx_b"})
      end
      assert_eq(AdvUser.count, 2)
    end

    test("rolls back and re-raises when the block throws") do
      assert_raises("boom") do
        create_then_throw("tx_lost")
      end
      assert_null(AdvUser.find_by("name", "tx_lost"))
      assert_eq(AdvUser.count, 0)
    end
  end

  describe("exists") do
    test("is true when a row matches") do
      AdvUser.create({"name": "ExistsTest", "value": 1})
      assert_eq(AdvUser.where("name = @n", {"n": "ExistsTest"}).exists.first, true)
    end

    test("is false when nothing matches") do
      assert_eq(AdvUser.where("name = @n", {"n": "NonExistent99999"}).exists.first, false)
    end
  end

  describe("pluck") do
    test("returns the field values in query order") do
      AdvUser.create({"name": "Pluck2", "value": 2})
      AdvUser.create({"name": "Pluck1", "value": 1})
      names = AdvUser.where("name LIKE @n", {"n": "Pluck%"}).order("value").pluck("name").all
      assert_eq(names, ["Pluck1", "Pluck2"])
    end

    test("several fields return hashes") do
      AdvUser.create({"name": "Pluck1", "value": 1})
      assert_eq(AdvUser.pluck("name", "value").all, [{"name": "Pluck1", "value": 1}])
    end
  end

  describe("aggregates") do
    before_each() do
      AdvUser.create({"name": "Agg1", "value": 10})
      AdvUser.create({"name": "Agg2", "value": 20})
      AdvUser.create({"name": "Agg3", "value": 30})
    end

    test("sum adds the field over the filtered rows") do
      pending(AGGREGATE_BUG)
      assert_eq(AdvUser.where("value > @v", {"v": 10}).sum("value").first, 50)
    end

    test("avg, min and max") do
      pending(AGGREGATE_BUG)
      assert_eq(AdvUser.avg("value").first, 20)
      assert_eq(AdvUser.min("value").first, 10)
      assert_eq(AdvUser.max("value").first, 30)
    end

    test("median goes through a collected list") do
      assert_eq(AdvUser.median("value").first, 20)
    end

    test("a multi-aggregate spec returns one row") do
      row = AdvUser.aggregate({"total": ["sum", "value"], "n": ["count"]}).first
      assert_eq(row["total"], 60)
      assert_eq(row["n"], 3)
    end
  end

  describe("increment and decrement") do
    test("increment adds one, or the given step, and persists") do
      user = AdvUser.create({"name": "IncrementTest", "value": 10})
      user.increment("value")
      assert_eq(user.value, 11)
      user.increment("value", 5)
      assert_eq(user.value, 16)
      assert_eq(AdvUser.find(user._key).value, 16)
    end

    test("increment starts a missing field from zero") do
      user = AdvUser.create({"name": "IncrementMissing"})
      user.increment("visits")
      assert_eq(AdvUser.find(user._key).visits, 1)
    end

    test("decrement subtracts one, or the given step, and persists") do
      user = AdvUser.create({"name": "DecrementTest", "value": 100})
      user.decrement("value")
      assert_eq(user.value, 99)
      user.decrement("value", 9)
      assert_eq(AdvUser.find(user._key).value, 90)
    end

    test("an unsaved record has no key to update") do
      assert_raises("no _key") do
        AdvUser.new({"name": "Unsaved", "value": 1}).increment("value")
      end
    end
  end

  describe("touch") do
    test("moves _updated_at forward and leaves _created_at alone") do
      user = AdvUser.create({"name": "TouchTest", "value": 1})
      created_at = user._created_at
      original_updated = user._updated_at
      user.touch
      user.reload
      assert(user._updated_at > original_updated, "#{user._updated_at} should be after #{original_updated}")
      assert_eq(user._created_at, created_at)
    end
  end

  describe("soft delete") do
    test("delete stamps deleted_at instead of removing the row") do
      record = AdvSoft.create({"name": "SoftDeleteTest"})
      assert_eq(record.delete, true)
      assert_not_null(record.deleted_at)
      assert_eq(AdvSoft.with_deleted().all.length, 1)
    end

    test("filtered queries skip deleted rows") do
      record = AdvSoft.create({"name": "SoftDeleteTest"})
      record.delete
      assert_eq(AdvSoft.where("name = @n", {"n": "SoftDeleteTest"}).all.length, 0)
      assert_eq(AdvSoft.with_deleted().where("name = @n", {"n": "SoftDeleteTest"}).first._key, record._key)
    end

    test("Model.all and Model.count skip deleted rows") do
      pending("bug: unfiltered Model.all / Model.count include soft-deleted rows (only where adds the guard)")
      AdvSoft.create({"name": "Kept"})
      AdvSoft.create({"name": "Gone"}).delete
      assert_eq(names_of(AdvSoft.all), ["Kept"])
      assert_eq(AdvSoft.count, 1)
    end

    test("only_deleted returns just the deleted rows") do
      AdvSoft.create({"name": "Kept"})
      AdvSoft.create({"name": "OnlyDeletedTest"}).delete
      assert_eq(names_of(AdvSoft.only_deleted().all), ["OnlyDeletedTest"])
    end

    test("restore clears deleted_at") do
      record = AdvSoft.create({"name": "RestoreTest"})
      record.delete
      assert_eq(record.restore, true)
      assert_null(record.deleted_at)
      assert_null(AdvSoft.find(record._key).deleted_at)
      assert_eq(AdvSoft.only_deleted().all.length, 0)
      assert_eq(AdvSoft.where("name = @n", {"n": "RestoreTest"}).first._key, record._key)
    end
  end

  describe("delete_all") do
    test("wipes the collection") do
      AdvUser.create({"name": "Wiped"})
      AdvUser.delete_all()
      assert_eq(AdvUser.count, 0)
    end

    test("runs without parentheses like any zero-argument method") do
      pending("bug: static delete_all, clear_mocks, with_deleted, only_deleted without () return the bound fn")
      AdvUser.create({"name": "Wiped"})
      AdvUser.delete_all
      assert_eq(AdvUser.count, 0)
    end
  end

  describe("offset") do
    test("skips the first rows of the ordered result") do
      AdvUser.create({"name": "Offset1", "value": 1})
      AdvUser.create({"name": "Offset2", "value": 2})
      AdvUser.create({"name": "Offset3", "value": 3})
      offset_results = AdvUser.where("name LIKE @n", {"n": "Offset%"}).order("value", "asc").offset(1).all
      assert_eq(names_of(offset_results), ["Offset2", "Offset3"])
    end
  end

  describe("paginate") do
    before_each() do
      AdvUser.create({"name": "PagA", "value": 1})
      AdvUser.create({"name": "PagB", "value": 2})
      AdvUser.create({"name": "PagC", "value": 3})
    end

    test("splits the ordered result into pages") do
      matches = AdvUser.where("name LIKE @n", {"n": "Pag%"}).order("value", "asc")
      first_page = matches.paginate({"page": 1, "per": 2})
      assert_eq(names_of(first_page["records"]), ["PagA", "PagB"])
      assert_eq(first_page["pagination"], {"page": 1, "per": 2, "total": 3, "total_pages": 2})

      second_page = matches.paginate({"page": 2, "per": 2})
      assert_eq(names_of(second_page["records"]), ["PagC"])
      assert_eq(second_page["pagination"], {"page": 2, "per": 2, "total": 3, "total_pages": 2})
    end

    test("the static form pages the whole collection") do
      result = AdvUser.paginate({"page": 3, "per": 1})
      assert_eq(result["records"].length, 1)
      assert_eq(result["pagination"], {"page": 3, "per": 1, "total": 3, "total_pages": 3})
    end

    test("clamps a page past the end to the last page") do
      result = AdvUser.order("value", "asc").paginate({"page": 999, "per": 2})
      assert_eq(names_of(result["records"]), ["PagC"])
      assert_eq(result["pagination"]["page"], 2)
    end

    test("clamps page 0 to the first page") do
      result = AdvUser.order("value", "asc").paginate({"page": 0, "per": 2})
      assert_eq(names_of(result["records"]), ["PagA", "PagB"])
      assert_eq(result["pagination"]["page"], 1)
    end

    test("defaults to page 1 and 25 per page") do
      result = AdvUser.order("value", "asc").paginate({})
      assert_eq(result["records"].length, 3)
      assert_eq(result["pagination"], {"page": 1, "per": 25, "total": 3, "total_pages": 1})
    end

    test("an empty match is one empty page") do
      result = AdvUser.where("name = @n", {"n": "NONEXISTENT_PAGINATE"}).paginate({"page": 1, "per": 10})
      assert_eq(result["records"], [])
      assert_eq(result["pagination"], {"page": 1, "per": 10, "total": 0, "total_pages": 1})
    end
  end
end
