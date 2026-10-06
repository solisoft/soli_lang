# Columnar models: the `columnar`/`column` class-body DSL, insert_rows(),
# aggregate() (scalar and grouped), query() projection filters, count, the
# column index lifecycle, and the document-API lockout. The store is dropped
# after each test; the next insert re-creates it from the declared schema.

class ColTestView < Model
  columnar
  column("url", "string")
  column("ms", "int", nullable: true)
  column("country", "string", indexed: true)
end

const NO_DOCUMENT_API = "ColTestView is a columnar model; columnar stores have no document API"

# A raw client on the ORM's database: the only way to drop a columnar store.
def raw_db
  db = Solidb(getenv("SOLIDB_HOST") || "http://localhost:6745", db_name())
  username = getenv("SOLIDB_USERNAME")
  db.auth(username, getenv("SOLIDB_PASSWORD")) if username.present?
  db
end

# fr: ms 120 and 80 (avg 100); de: ms 200.
def insert_standard_rows
  ColTestView.insert_rows([
    {"url": "/a", "ms": 120, "country": "fr"},
    {"url": "/b", "ms": 80, "country": "fr"},
    {"url": "/c", "ms": 200, "country": "de"}
  ])
end

def query_where(column, op, value)
  ColTestView.query({"columns": ["url"], "filter": {"column": column, "op": op, "value": value}})
end

describe("Columnar DSL validation (no DB)") do
  test("column DSL rejects unknown column types at class-body time") do
    message = assert_raises("column \"x\": unknown type \"wrongtype\"") do
      class ColBadType < Model
        columnar
        column("x", "wrongtype")
      end
    end

    assert_contains(message, "expected one of int, integer")
  end
end

describe("Columnar document-API lockout (no DB)") do
  test("where() raises no document API") do
    assert_raises("ColTestView.where: #{NO_DOCUMENT_API}") do
      ColTestView.where({"country": "FR"})
    end
  end

  test("create() raises no document API") do
    assert_raises("ColTestView.create: #{NO_DOCUMENT_API}") do
      ColTestView.create({"url": "/x"})
    end
  end

  test("find() raises no document API") do
    assert_raises("ColTestView.find: #{NO_DOCUMENT_API}") do
      ColTestView.find("x")
    end
  end

  test("all raises no document API") do
    assert_raises("ColTestView.all: #{NO_DOCUMENT_API}") do
      ColTestView.all
    end
  end

  test("delete_all raises no document API") do
    assert_raises("ColTestView.delete_all: #{NO_DOCUMENT_API}") do
      ColTestView.delete_all()
    end
  end
end

describe("Columnar argument validation (no DB)") do
  test("query() op in requires an array value") do
    assert_raises("query() filter op \"in\" requires an array value") do
      query_where("country", "in", "FR")
    end
  end

  test("query() rejects unknown filter ops") do
    assert_raises("query() unknown filter op 'like': expected one of eq, ne, gt, gte, lt, lte, in") do
      query_where("country", "like", "FR")
    end
  end

  test("aggregate() rejects unknown operations") do
    assert_raises("ColTestView.aggregate unknown operation 'frobnicate'") do
      ColTestView.aggregate("ms", "frobnicate")
    end
  end
end

describe("Columnar insert/aggregate/query/count (DB)") do
  before_each() do
    requires_solidb()
  end

  after_each() do
    raw_db().drop_columnar("col_test_views")
  end

  test("insert_rows reports the inserted count and row ids") do
    result = insert_standard_rows()

    assert_eq(result["inserted"], 3)
    assert_eq(result["ids"].length, 3)
    assert_eq(result["ids"].uniq.length, 3)
  end

  test("aggregate returns a scalar without group_by") do
    insert_standard_rows()

    assert_eq(ColTestView.aggregate("ms", "sum"), 400)
    assert_eq(ColTestView.aggregate("ms", "min"), 80)
    assert_eq(ColTestView.aggregate("ms", "max"), 200)
    assert_eq(ColTestView.aggregate("ms", "count_distinct"), 3)
  end

  test("avg over an int column") do
    ColTestView.insert_rows([{"url": "/a", "ms": 100, "country": "fr"}, {"url": "/b", "ms": 200, "country": "fr"}])

    assert_eq(ColTestView.aggregate("ms", "avg"), 150)
  end

  test("grouped aggregate returns one row per group key with its value") do
    insert_standard_rows()

    rows = ColTestView.aggregate("ms", "avg", {"group_by": ["country"]})

    assert_eq(rows.length, 2)
    assert_eq(rows.filter { |row| row["country"] == "fr" }, [{"country": "fr", "value": 100}])
    assert_eq(rows.filter { |row| row["country"] == "de" }, [{"country": "de", "value": 200}])
  end

  test("query projects the requested columns under an eq filter") do
    insert_standard_rows()

    rows = ColTestView.query({
      "columns": ["url", "ms"],
      "filter": {"column": "country", "op": "eq", "value": "fr"},
      "limit": 10
    })

    assert_eq(rows.sort_by { |row| row["url"] }, [{"url": "/a", "ms": 120}, {"url": "/b", "ms": 80}])
  end

  test("query filters with gt and in") do
    insert_standard_rows()

    assert_eq(query_where("ms", "gt", 100).map { |row| row["url"] }.sort(), ["/a", "/c"])
    assert_eq(query_where("country", "in", ["de"]), [{"url": "/c"}])
    assert_eq(query_where("country", "eq", "nowhere"), [])
  end

  test("count goes through the columnar engine") do
    insert_standard_rows()
    assert_eq(ColTestView.count, 3)

    ColTestView.insert_rows([{"url": "/d", "ms": nil, "country": "fr"}])
    assert_eq(ColTestView.count, 4)
  end

  test("column index lifecycle: add, list, drop") do
    insert_standard_rows()

    # `url` is not schema-indexed, so the full lifecycle runs on it.
    ColTestView.add_column_index("url", "bitmap")
    indexes = ColTestView.column_indexes
    assert_eq(indexes.map { |index| [index["column"], index["index_type"]] }, [["url", "bitmap"]])

    ColTestView.drop_column_index("url")
    assert_eq(ColTestView.column_indexes, [])
  end

  test("a schema-declared indexed column already has an index") do
    insert_standard_rows()

    assert_raises("Column 'country' already has an index") do
      ColTestView.add_column_index("country", "bitmap")
    end
  end
end
