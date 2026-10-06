# find_each / in_batches — keyset-paged iteration over a whole collection.
# Each batch filters `_key > <last key seen>`, so only one batch is held in
# memory and writes during the scan cannot make it skip or repeat rows.

class BatchItem < Model
end

def seed(names)
  names.each do |name|
    BatchItem.create({"name": name})
  end
end

def find_each_inside_grouped
  grouped(fn() { BatchItem.find_each { |record| record } })
end

describe("find_each argument checking") do
  test("a non-callable first argument raises") do
    assert_raises("find_each() expects a function as its first argument") do
      BatchItem.find_each(42)
    end
  end

  test("no arguments raises") do
    assert_raises("find_each() expects a block and an optional options hash") do
      BatchItem.find_each()
    end
  end

  test("a non-integer batch_size raises") do
    assert_raises("batch_size must be an integer between 1 and 10000") do
      BatchItem.find_each(fn(record) { record }, {"batch_size": "many"})
    end
  end

  test("a zero batch_size raises") do
    assert_raises("batch_size must be an integer between 1 and 10000") do
      BatchItem.find_each(fn(record) { record }, {"batch_size": 0})
    end
  end

  test("a batch_size above the ceiling raises") do
    assert_raises("batch_size must be an integer between 1 and 10000") do
      BatchItem.find_each(fn(record) { record }, {"batch_size": 10001})
    end
  end
end

# Refused loudly rather than silently ignored: these methods drive
# data-correction jobs, where a dropped clause is a wrong run that still
# reports success.
describe("find_each rejects clauses keyset paging cannot serve") do
  test("an explicit .order raises") do
    assert_raises("find_each() cannot be combined with .order()") do
      BatchItem.order("name", "asc").find_each { |record| record }
    end
  end

  test("an explicit .limit raises") do
    assert_raises("find_each() cannot be combined with .limit()") do
      BatchItem.limit(5).find_each { |record| record }
    end
  end

  test("an explicit .offset raises") do
    assert_raises("find_each() cannot be combined with .offset()") do
      BatchItem.offset(5).find_each { |record| record }
    end
  end

  test(".pluck raises — projected rows carry no _key") do
    assert_raises("find_each() cannot be combined with .pluck()") do
      BatchItem.pluck("name").find_each { |record| record }
    end
  end

  test("an aggregate raises — it returns one value, not records") do
    assert_raises("find_each() cannot be combined with an aggregate") do
      BatchItem.sum("n").find_each { |record| record }
    end
  end

  test("inside grouped() raises — batches cannot be coalesced") do
    assert_raises("find_each() cannot run inside grouped()") do
      find_each_inside_grouped()
    end
  end
end

describe("find_each walks every record") do
  before_each() do
    requires_solidb()
  end

  after_each() do
    BatchItem.delete_all()
  end

  test("visits every record exactly once across several batches") do
    seed(["b1", "b2", "b3", "b4", "b5"])

    # batch_size 2 over 5 rows: three round-trips, the last one short.
    seen = []
    BatchItem.find_each(fn(item) { seen.push(item.name) }, {"batch_size": 2})

    assert_eq(seen.sort(), ["b1", "b2", "b3", "b4", "b5"])
  end

  test("takes a trailing block when no options are given") do
    seed(["b1", "b2", "b3"])

    seen = []
    BatchItem.find_each do |item|
      seen.push(item.name)
    end

    assert_eq(seen.sort(), ["b1", "b2", "b3"])
  end

  test("yields model instances") do
    seed(["b1"])

    classes = []
    BatchItem.find_each do |item|
      classes.push(item.class)
    end

    assert_eq(classes, ["BatchItem"])
  end

  test("returns nil — it is a traversal, not a projection") do
    seed(["b1"])

    assert_null(BatchItem.find_each { |item| item.name })
  end

  test("a batch_size larger than the collection still visits everything") do
    seed(["b1", "b2", "b3"])

    seen = 0
    BatchItem.find_each(fn(item) { seen += 1 }, {"batch_size": 1000})

    assert_eq(seen, 3)
  end

  test("a batch_size of 1 still terminates") do
    seed(["b1", "b2", "b3"])

    seen = 0
    BatchItem.find_each(fn(item) { seen += 1 }, {"batch_size": 1})

    assert_eq(seen, 3)
  end

  test("composes with a filter") do
    seed(["keep1", "keep2", "skip1"])

    seen = []
    BatchItem.where("name LIKE @n", {"n": "keep%"}).find_each(fn(item) { seen.push(item.name) }, {"batch_size": 1})

    assert_eq(seen.sort(), ["keep1", "keep2"])
  end

  test("an empty result set never calls the block") do
    seed(["b1"])

    called = false
    BatchItem.where("name == @n", {"n": "nothing-matches-this"}).find_each do |item|
      called = true
    end

    assert_eq(called, false)
  end
end

describe("in_batches yields arrays") do
  before_each() do
    requires_solidb()
  end

  after_each() do
    BatchItem.delete_all()
  end

  test("hands the block one array per batch") do
    seed(["c1", "c2", "c3", "c4", "c5"])

    sizes = []
    BatchItem.in_batches(fn(batch) { sizes.push(batch.length) }, {"batch_size": 2})

    # 5 rows at 2 per batch: 2 + 2 + 1.
    assert_eq(sizes, [2, 2, 1])
  end

  test("a collection that divides evenly ends without an empty batch") do
    seed(["c1", "c2", "c3", "c4"])

    sizes = []
    BatchItem.in_batches(fn(batch) { sizes.push(batch.length) }, {"batch_size": 2})

    assert_eq(sizes, [2, 2])
  end

  test("an empty collection never calls the block") do
    batches = 0
    BatchItem.in_batches(fn(batch) { batches += 1 }, {"batch_size": 2})

    assert_eq(batches, 0)
  end

  test("find_in_batches is the same method") do
    seed(["c1", "c2", "c3", "c4", "c5"])

    names = []
    BatchItem.find_in_batches(fn(batch) { batch.each { |item| names.push(item.name) } }, {"batch_size": 2})

    assert_eq(names.sort(), ["c1", "c2", "c3", "c4", "c5"])
  end

  test("deleting inside the block does not skip records") do
    seed(["d1", "d2", "d3", "d4", "d5"])

    # The cursor is read *before* the block runs, so a batch that deletes
    # its own rows still positions the next query correctly. Asserted on
    # the visit count rather than a follow-up `.count`, which SoliDB's
    # read-result cache can answer from before the deletes.
    seen = []
    BatchItem.in_batches(fn(batch) {
      batch.each do |item|
        seen.push(item.name)
        item.delete
      end
    }, {"batch_size": 2})

    assert_eq(seen.sort(), ["d1", "d2", "d3", "d4", "d5"])
  end
end
