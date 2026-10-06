# Column-aware models on a SQL adapter: CRUD on real columns, `encrypts`
# (ciphertext at rest, plaintext in memory), and STI (`type` column, subclass
# scope, descendant counts).
#
# `cargo test` (src/migration.rs) creates `sql_invoices` / `sql_people` with a
# real SQLite migration, then runs this file. Under `soli test` against SoliDB
# those tables do not exist, so every test is skipped.

class SqlInvoice < Model
  table("sql_invoices")
end

class SqlPerson < Model
  table("sql_people")
  encrypts(:ssn)
end

class SqlAdmin < SqlPerson
end

class SqlSuperAdmin < SqlAdmin
end

def sql_tables_ready
  !(SqlInvoice.count rescue nil).nil?
end

describe("column-aware models on SQL") do
  before_each() do
    skip("needs the sql_invoices / sql_people tables (run by cargo test)") unless sql_tables_ready()
  end

  after_each() do
    SqlInvoice.delete_all()
    SqlPerson.delete_all()
  end

  describe("SqlInvoice CRUD") do
    test("create returns the typed row with its generated id and timestamp") do
      created = SqlInvoice.create({"code": "INV-1", "qty": 2, "paid": true})

      assert_null(created._errors)
      assert_eq(created.code, "INV-1")
      assert_eq(created.qty, 2)
      assert_eq(created.paid, true)
      assert_eq(type(created.id), "int")
      assert_not_null(created.created_at)
    end

    test("a NOT NULL column without a value is refused") do
      created = SqlInvoice.create({"qty": 1})

      assert_eq(created._errors, [{"field": "code", "message": "can't be blank"}])
      assert_eq(SqlInvoice.count, 0)
    end

    test("find and find_by read the row back") do
      created = SqlInvoice.create({"code": "INV-1", "qty": 2})
      fetched = SqlInvoice.find(created.id)

      assert_eq(fetched.code, "INV-1")
      assert_eq(fetched.qty, 2)
      assert_eq(SqlInvoice.find_by("code", "INV-1").id, created.id)
    end

    test("where filters on real columns") do
      created = SqlInvoice.create({"code": "INV-1", "qty": 5, "paid": false})
      SqlInvoice.create({"code": "OTHER", "qty": 1, "paid": true})

      matches = SqlInvoice.where({"code": "INV-1"}).all
      assert_eq(matches.length, 1)
      assert_eq(matches[0].id, created.id)
      assert_eq(SqlInvoice.where({"paid": false}).count, 1)
      assert_eq(SqlInvoice.where({"qty": {"gte": 5}}).count, 1)
      assert_eq(SqlInvoice.where({"code": {"like": "INV%"}}).count, 1)
      assert_eq(SqlInvoice.where({"qty": [5, 99]}).count, 1)
      assert_eq(SqlInvoice.where({"or": [{"qty": 5}, {"qty": 0}]}).count, 1)
    end

    test("save writes changed columns") do
      created = SqlInvoice.create({"code": "INV-1", "qty": 2, "paid": true})
      fetched = SqlInvoice.find(created.id)
      fetched.qty = 5
      fetched.paid = false

      assert_eq(fetched.save, true)
      again = SqlInvoice.find(created.id)
      assert_eq(again.qty, 5)
      assert_eq(again.paid, false)
    end

    test("delete removes the row") do
      created = SqlInvoice.create({"code": "INV-1", "qty": 2})
      created.delete

      assert_null(SqlInvoice.find_by("code", "INV-1"))
      assert_eq(SqlInvoice.count, 0)
    end

    test("find raises RecordNotFound on a missing integer key") do
      assert_raises("SqlInvoice with id '999999' not found") do
        SqlInvoice.find(999999)
      end
    end
  end

  describe("encrypts") do
    test("round-trips on a real column") do
      created = SqlPerson.create({"name": "Ada", "ssn": "123-45-6789"})
      assert_null(created._errors)
      assert_eq(created.ssn, "123-45-6789")

      fetched = SqlPerson.find(created.id)
      assert_eq(fetched.ssn, "123-45-6789")
      assert_eq(fetched.name, "Ada")

      fetched.ssn = "987-65-4321"
      assert_eq(fetched.save, true)
      assert_eq(SqlPerson.find(created.id).ssn, "987-65-4321")
    end

    test("stores ciphertext and cannot be filtered by plaintext") do
      created = SqlPerson.create({"name": "Cipher", "ssn": "111-22-3333"})
      assert_null(created._errors)

      # A projection aliased away from the field name is not decrypted.
      rows = SqlPerson.find_by_sql("SELECT ssn AS cipher FROM sql_people WHERE name = ?", ["Cipher"])
      assert_eq(rows.length, 1)
      assert_ne(rows[0].cipher, "111-22-3333")
      assert_gt(rows[0].cipher.length, 20)

      # AES-GCM is non-deterministic: equality on the plaintext never matches.
      assert_eq(SqlPerson.where({"ssn": "111-22-3333"}).count, 0)
    end
  end

  describe("STI") do
    test("subclasses share the table and record their type") do
      admin = SqlAdmin.create({"name": "Root", "ssn": "000-00-0001"})
      super_admin = SqlSuperAdmin.create({"name": "Super", "ssn": "000-00-0002"})

      assert_null(admin._errors)
      assert_null(super_admin._errors)
      assert_eq(admin.type, "SqlAdmin")
      assert_eq(super_admin.type, "SqlSuperAdmin")
      assert_eq(SqlAdmin.find(admin.id).name, "Root")
      assert_eq(SqlAdmin.find(admin.id).ssn, "000-00-0001")
      assert_eq(SqlAdmin.find(super_admin.id).type, "SqlSuperAdmin")
      assert_eq(SqlPerson.find(admin.id).type, "SqlAdmin")
    end

    test("queries on a subclass are scoped to its hierarchy") do
      person = SqlPerson.create({"name": "Base"})
      admin = SqlAdmin.create({"name": "Root"})
      super_admin = SqlSuperAdmin.create({"name": "Super"})

      assert_null(person._errors)
      assert_eq(SqlPerson.where({"name": "Root"}).count, 1)
      assert_eq(SqlAdmin.where({"name": "Root"}).count, 1)
      assert_eq(SqlAdmin.where({"name": "Base"}).count, 0)
      assert_eq(SqlAdmin.where({"name": "Super"}).count, 1)
      assert_eq(SqlSuperAdmin.where({"name": "Root"}).count, 0)
      assert_eq(SqlAdmin.find_by("name", "Root").id, admin.id)
      assert_null(SqlAdmin.find_by("name", "Base"))
      assert_eq(SqlAdmin.first_by("name", "Super").id, super_admin.id)
      assert_null(SqlAdmin.first_by("name", "Base"))
    end

    test("count includes descendants") do
      SqlPerson.create({"name": "Base"})
      SqlAdmin.create({"name": "Root"})
      SqlSuperAdmin.create({"name": "Super"})

      assert_eq(SqlPerson.count, 3)
      assert_eq(SqlAdmin.count, 2)
      assert_eq(SqlSuperAdmin.count, 1)
    end

    test("a subclass cannot find a row of its parent class") do
      person = SqlPerson.create({"name": "Base"})

      assert_raises("SqlAdmin with id '#{person.id}' not found") do
        SqlAdmin.find(person.id)
      end
    end

    test("a subclass delete_all only removes its hierarchy") do
      SqlPerson.create({"name": "Keep"})
      SqlAdmin.create({"name": "Drop"})
      SqlAdmin.delete_all()

      assert_eq(SqlPerson.find_by("name", "Keep").name, "Keep")
      assert_null(SqlPerson.find_by("name", "Drop"))
    end
  end
end
