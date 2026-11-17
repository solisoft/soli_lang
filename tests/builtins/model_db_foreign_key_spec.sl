# Foreign keys: SoliDB refuses a child whose parent is not there, and a
# delete of a parent that children still refer to, once a migration has
# declared the key (`db.add_foreign_key`). The model reports the refusal as
# a validation error: on the field for a save, on `_base` for a delete.

class DbFkCustomer < Model
end

class DbFkOrder < Model
end

class DbFkNote < Model
end

# A raw client on the ORM's database, as a migration's `db` is.
def raw_db
  db = Solidb(getenv("SOLIDB_HOST") || "http://localhost:6745", db_name())
  username = getenv("SOLIDB_USERNAME")
  db.auth(username, getenv("SOLIDB_PASSWORD")) if username.present?
  db
end

def declare_keys
  db = raw_db()
  names = db.list_collections().map do |entry|
    entry["name"]
  end
  for name in ["db_fk_customers", "db_fk_orders", "db_fk_notes"]
    db.create_collection(name) unless names.includes?(name)
  end
  if db.list_foreign_keys("db_fk_orders")["foreign_keys"].length() == 0
    db.create_foreign_key("db_fk_orders", "customer_id", "db_fk_customers")
  end
  if db.list_foreign_keys("db_fk_notes")["foreign_keys"].length() == 0
    db.create_foreign_key("db_fk_notes", "customer_id", "db_fk_customers", {"on_delete": "cascade"})
  end
end

describe("a foreign key declared in the database") do
  before_each() do
    requires_solidb()
    declare_keys()
  end

  after_each() do
    DbFkOrder.delete_all()
    DbFkNote.delete_all()
    DbFkCustomer.delete_all()
  end

  test("a child of a parent that is there is saved") do
    customer = DbFkCustomer.create({"name": "Ada"})
    order = DbFkOrder.create({"customer_id": customer._key, "total": 10})

    assert_not_null(order._key)
    assert_null(order._errors)
  end

  test("a child of a missing parent is refused, with an error on the field") do
    order = DbFkOrder.create({"customer_id": "nobody", "total": 10})

    assert_null(order._key)
    assert_eq(order._errors, [{"field": "customer_id", "message": "must reference an existing record"}])
    assert_eq(DbFkOrder.count, 0)
  end

  test("save reports it the same way, and an update too") do
    customer = DbFkCustomer.create({"name": "Ada"})
    order = DbFkOrder.new({"customer_id": "nobody"})

    assert_eq(order.save, false)
    assert_eq(order._errors[0]["field"], "customer_id")

    order.customer_id = customer._key
    assert_eq(order.save, true)

    order.customer_id = "gone"
    assert_eq(order.save, false)
    assert_eq(order._errors[0]["message"], "must reference an existing record")
    assert_eq(DbFkOrder.find(order._key).customer_id, customer._key)
  end

  test("a child without a parent is allowed") do
    order = DbFkOrder.create({"total": 5})

    assert_not_null(order._key)
  end

  test("a parent that orders refer to is not deleted") do
    customer = DbFkCustomer.create({"name": "Ada"})
    DbFkOrder.create({"customer_id": customer._key})

    assert_eq(customer.delete, false)
    assert_eq(customer._errors, [{
      "field": "_base",
      "message": "is still referenced by other records"
    }])
    assert_not_null(DbFkCustomer.find_by("name", "Ada"))
  end

  test("a cascading key deletes the children with the parent") do
    customer = DbFkCustomer.create({"name": "Grace"})
    DbFkNote.create({"customer_id": customer._key, "text": "one"})
    DbFkNote.create({"customer_id": customer._key, "text": "two"})

    customer.delete

    assert_eq(DbFkNote.count, 0)
    assert_null(DbFkCustomer.find_by("name", "Grace"))
  end
end
