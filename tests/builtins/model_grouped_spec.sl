# grouped(fn() { ... }) — request coalescing. Reads inside the block are
# deferred and combined into one round-trip; afterwards they are ordinary
# values.

class GroupItem < Model
end

# A raw client on the ORM's database, with the credentials the models use.
def raw_db
  db = Solidb(getenv("SOLIDB_HOST") || "http://localhost:6745", db_name())
  username = getenv("SOLIDB_USERNAME")
  db.auth(username, getenv("SOLIDB_PASSWORD")) if username.present?
  db
end

describe("grouped block control flow") do
  test("runs the block and returns its value") do
    assert_eq(grouped(fn() { 1 + 2 }), 3)
  end

  test("a block returning nil yields nil") do
    assert_null(grouped(fn() { nil }))
  end

  test("misuse (non-block argument) raises") do
    assert_raises("grouped() expects a function block") do
      grouped(42)
    end
  end
end

describe("grouped coalesces reads") do
  before_each() do
    requires_solidb()
    GroupItem.create({"name": "g1", "n": 1})
    GroupItem.create({"name": "g2", "n": 2})
    GroupItem.create({"name": "g3", "n": 3})
  end

  after_each() do
    GroupItem.delete_all()
  end

  test("a .all and a .count return the same as running them separately") do
    result = grouped(fn() {
      items = GroupItem.all
      total = GroupItem.count
      {"items": items, "total": total}
    })

    assert_eq(result["items"].map { |item| item.name }.sort(), ["g1", "g2", "g3"])
    assert_eq(result["total"], 3)
  end

  test("several counts (incl. a filtered count) and an .all coalesce correctly") do
    # Mirrors the admin-dashboard shape that regressed: bare `.count` reads
    # (`RETURN COLLECTION_COUNT(...)`) plus a `where(...).count`
    # (`RETURN LENGTH(FOR ... RETURN 1)`) and an `.all` in one block. Each
    # bare RETURN must be unwrapped when combined.
    result = grouped(fn() {
      total = GroupItem.count
      twos = GroupItem.where({"n": 2}).count
      items = GroupItem.all
      {"total": total, "twos": twos, "items": items}
    })

    assert_eq(result["total"], 3)
    assert_eq(result["twos"], 1)
    assert_eq(result["items"].length, 3)
  end

  test("db.query inside grouped returns the same rows as a Model read") do
    # A raw read against the ORM database joins the coalesced batch, so
    # Model.timeout on a sibling covers it rather than a second round-trip.
    db = raw_db()
    result = grouped(fn() {
      items = GroupItem.timeout(30).all
      raw = db.query("FOR d IN group_items RETURN d.name")
      {"items": items, "raw": raw}
    })

    assert_eq(result["items"].length, 3)
    assert_eq(result["raw"].sort(), ["g1", "g2", "g3"])
  end

  test("db.timeout().query and query(..., {timeout}) accept the timeout forms") do
    db = raw_db()
    via_chain = db.timeout(30).query("FOR d IN group_items SORT d.n RETURN d.name")
    via_opts = db.query("FOR d IN group_items SORT d.n RETURN d.name", {}, {"timeout": 30})

    assert_eq(via_chain, ["g1", "g2", "g3"])
    assert_eq(via_opts, ["g1", "g2", "g3"])
  end

  test("find_by inside grouped resolves to the right record") do
    found = grouped(fn() { GroupItem.find_by("name", "g2") })

    assert_eq(found.name, "g2")
    assert_eq(found.n, 2)
  end

  test("find_by on a miss inside grouped resolves to nil") do
    found = grouped(fn() { GroupItem.find_by("name", "nope") })

    assert_null(found)
  end

  test("assert_null sees the resolved value of an inline grouped() call") do
    pending("bug: assert_null(grouped(fn() { M.find_by(\"f\", \"miss\") })) fails with \"expected nil, got null\"")
    assert_null(grouped(fn() { GroupItem.find_by("name", "nope") }))
  end

  test("reading a result mid-block auto-flushes and stays correct") do
    result = grouped(fn() {
      items = GroupItem.all
      seen = items.length # forces a flush here
      total = GroupItem.count # registered into a fresh batch
      {"seen": seen, "total": total}
    })

    assert_eq(result["seen"], 3)
    assert_eq(result["total"], 3)
  end

  test("find on a missing id inside grouped still raises") do
    assert_raises("not found") do
      grouped(fn() { GroupItem.find("does-not-exist-xyz") })
    end
  end
end
