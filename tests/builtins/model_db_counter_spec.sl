# Counter fields: SoliDB numbers a field on insert once a migration has
# declared it (`db.create_counter`). The model does not set the field; it
# reads the number from the insert's answer, on `create` and on `save`.
# In a transaction the number is given at commit: the record has none until
# it is read back.

class DbCounterInvoice < Model
end

# A raw client on the ORM's database, as a migration's `db` is.
def raw_db
  db = Solidb(getenv("SOLIDB_HOST") || "http://localhost:6745", db_name())
  username = getenv("SOLIDB_USERNAME")
  db.auth(username, getenv("SOLIDB_PASSWORD")) if username.present?
  db
end

# The number of an invoice code such as "INV-0007".
def number_of(code)
  code.replace("INV-", "").to_i
end

def declare_counter
  db = raw_db()
  names = db.list_collections().map { |entry| entry["name"] }
  db.create_collection("db_counter_invoices") unless names.includes?("db_counter_invoices")
  return if db.list_counters("db_counter_invoices").length() > 0
  db.create_counter("db_counter_invoices", "code", {"prefix": "INV-", "padding": 4})
end

describe("a field numbered by the database") do
  before_each() do
    requires_solidb()
    declare_counter()
  end

  after_each() do
    DbCounterInvoice.delete_all()
  end

  test("create answers with the number") do
    first = DbCounterInvoice.create({"total": 10})
    second = DbCounterInvoice.create({"total": 20})

    assert_not_null(first._key)
    assert_match(first.code, "^INV-[0-9]{4}$")
    assert_eq(number_of(second.code), number_of(first.code) + 1)
    assert_eq(DbCounterInvoice.find(first._key).code, first.code)
  end

  test("save numbers a new record") do
    invoice = DbCounterInvoice.new({"total": 30})
    invoice.save

    assert_match(invoice.code, "^INV-[0-9]{4}$")
    assert_eq(DbCounterInvoice.find(invoice._key).code, invoice.code)
  end

  test("an update leaves the number alone") do
    invoice = DbCounterInvoice.create({"total": 40})
    code = invoice.code
    invoice.total = 41
    invoice.save

    assert_eq(invoice.code, code)
    assert_eq(DbCounterInvoice.find(invoice._key).code, code)
  end

  test("the number is not the application's to set") do
    invoice = DbCounterInvoice.create({"total": 50, "code": "INV-9999"})

    assert_null(invoice._key)
    assert_eq(DbCounterInvoice.where("doc.total == 50").count, 0)
  end

  test("a record created in a transaction is numbered when it commits") do
    key = nil
    DbCounterInvoice.transaction do
      invoice = DbCounterInvoice.create({"total": 60})
      key = invoice._key
    end

    assert_match(DbCounterInvoice.find(key).code, "^INV-[0-9]{4}$")
  end
end
