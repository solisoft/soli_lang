# Column-aware models on a SQL adapter.
#
# `cargo test` creates `sql_invoices` with a real migration first, then runs
# this file via the test runner so the assertions actually execute.
#
# `soli test` against SoliDB (no table) probes `SqlInvoice.count()`, fails the
# probe, and each test returns without asserting — the suite stays green.

class SqlInvoice < Model
  table "sql_invoices"
end

class SqlPerson < Model
  table "sql_people"
  encrypts(:ssn)
end

class SqlAdmin < SqlPerson
end

let __sql_ready = false
try
  SqlInvoice.count()
  __sql_ready = true
catch e
end

describe("column-aware SqlInvoice on SQL", fn() {
  test("creates, finds, filters, updates, and deletes a real-column row", fn() {
    return unless __sql_ready

    let created = SqlInvoice.create({
      "code": "INV-1",
      "qty": 2,
      "paid": true
    })
    assert(created._errors.nil?)
    assert_eq(created.code, "INV-1")
    assert_eq(created.qty, 2)
    assert_eq(created.paid, true)
    assert(created.id != null)
    assert(created.created_at != null)

    let fetched = SqlInvoice.find(created.id)
    assert_eq(fetched.code, "INV-1")
    assert_eq(fetched.qty, 2)

    let matches = SqlInvoice.where({ "code": "INV-1" }).all()
    assert_eq(matches.length(), 1)
    assert_eq(matches[0].id, created.id)

    let by_code = SqlInvoice.find_by("code", "INV-1")
    assert_eq(by_code.id, created.id)

    fetched.qty = 5
    fetched.paid = false
    assert(fetched.save())
    let again = SqlInvoice.find(created.id)
    assert_eq(again.qty, 5)
    assert_eq(again.paid, false)

    assert_eq(SqlInvoice.where({ "paid": false }).count(), 1)
    assert_eq(SqlInvoice.where({ "qty": { "gte": 5 } }).count(), 1)
    assert_eq(SqlInvoice.where({ "code": { "like": "INV%" } }).count(), 1)
    assert_eq(SqlInvoice.where({ "qty": [5, 99] }).count(), 1)
    assert_eq(SqlInvoice.where({ "or": [{ "qty": 5 }, { "qty": 0 }] }).count(), 1)

    again.delete()
    assert_null(SqlInvoice.find_by("code", "INV-1"))
    assert_eq(SqlInvoice.count(), 0)
  })

  test("encrypts round-trips on a real column", fn() {
    return unless __sql_ready

    let created = SqlPerson.create({ "name": "Ada", "ssn": "123-45-6789" })
    assert(created._errors.nil?)
    assert_eq(created.ssn, "123-45-6789")

    let fetched = SqlPerson.find(created.id)
    assert_eq(fetched.ssn, "123-45-6789")
    assert_eq(fetched.name, "Ada")

    fetched.ssn = "987-65-4321"
    assert(fetched.save())
    assert_eq(SqlPerson.find(created.id).ssn, "987-65-4321")
    created.delete()
  })

  test("STI subclasses share the table and scope by type", fn() {
    return unless __sql_ready

    let person = SqlPerson.create({ "name": "Base" })
    let admin = SqlAdmin.create({ "name": "Root", "ssn": "000-00-0001" })
    assert(person._errors.nil?)
    assert(admin._errors.nil?)
    assert_eq(admin.type, "SqlAdmin")
    assert_eq(SqlAdmin.find(admin.id).name, "Root")
    assert_eq(SqlAdmin.find(admin.id).ssn, "000-00-0001")

    assert_eq(SqlPerson.where({ "name": "Root" }).count(), 1)
    assert_eq(SqlAdmin.where({ "name": "Root" }).count(), 1)
    assert_eq(SqlAdmin.where({ "name": "Base" }).count(), 0)
    assert_eq(SqlAdmin.find_by("name", "Root").id, admin.id)
    assert_null(SqlAdmin.find_by("name", "Base"))

    let raised = false
    try
      SqlAdmin.find(person.id)
    catch error
      raised = true
    end
    assert(raised)

    person.delete()
    admin.delete()
  })

  test("find raises RecordNotFound on a missing integer key", fn() {
    return unless __sql_ready

    let raised = false
    try
      SqlInvoice.find(999999)
    catch error
      raised = true
    end
    assert(raised)
  })
})
