# Model methods hand back class instances, and those instances carry the
# persistence API: save/update (with or without a hash), delete, reload, errors.
# Validation failures stop before the database, so they run without one.

class InstProduct < Model
end

class InstValidatedItem < Model
  validates("title", {"presence": true})
  validates("title", {"min_length": 3})
end

class InstBareHashItem < Model
  validates(:name, presence: true)
  validates(:name, min_length: 2)
  validates(:email, presence: true, format: "^[^@]+@[^@]+$")
end

class InstMixedStyleItem < Model
  validates("name", {"presence": true})
  validates(:name, min_length: 2)
  validates(:email, presence: true, format: "^[^@]+@[^@]+$")
end

class InstStringBareHashItem < Model
  validates("title", presence: true)
  validates("title", min_length: 3)
  validates("title", max_length: 100)
end

class InstNumericItem < Model
  validates(:quantity, numericality: true, min: 0, max: 1000)
  validates(:price, presence: true, numericality: true, min: 0.01)
end

class InstUniqueCode < Model
  validates(:code, presence: true, uniqueness: true)
end

class InstCallbackItem < Model
  before_save(:normalize_name)

  def normalize_name
    @name = @name.trim.downcase unless @name.blank?
  end
end

const BLANK = "can't be blank"

# The record seeded by the "_id as a key" suite.
widget = nil

def error(field, message)
  {"field": field, "message": message}
end

# Builds `model` from `attributes`, saves it and returns the errors it collected.
def errors_on_save(model, attributes)
  record = model.new(attributes)
  assert_eq(record.save, false, "save should be refused")
  record.errors
end

def wipe_collections
  [InstProduct, InstValidatedItem, InstUniqueCode, InstCallbackItem, InstStringBareHashItem].each do |model|
    model.delete_all()
  end
end

describe("validation failures (no database reached)") do
  test("a fresh instance has no errors") do
    product = InstProduct.new()
    assert_eq(product.errors, [])
    assert_null(product._errors)
  end

  describe("save") do
    test("a missing title is refused as blank") do
      assert_eq(errors_on_save(InstValidatedItem, {}), [error("title", BLANK)])
    end

    test("a short title fails min_length") do
      errors = errors_on_save(InstValidatedItem, {"title": "ab"})
      assert_eq(errors, [error("title", "is too short (minimum is 3 characters)")])
    end

    test("an empty title fails presence and min_length") do
      errors = errors_on_save(InstValidatedItem, {"title": ""})
      assert_eq(errors, [error("title", BLANK), error("title", "is too short (minimum is 3 characters)")])
    end
  end

  describe("create") do
    test("returns an unpersisted instance of the class, not a hash") do
      item = InstValidatedItem.create({"title": ""})
      assert_eq(item.class, "InstValidatedItem")
      assert(item.is_a?("InstValidatedItem"))
      assert_null(item._key)
    end

    test("_errors lists {field, message} entries") do
      item = InstValidatedItem.create({"title": ""})
      assert_eq(item._errors, [error("title", BLANK), error("title", "is too short (minimum is 3 characters)")])
      assert_eq(item.errors, item._errors)
    end

    test("keeps the attributes it was given") do
      item = InstValidatedItem.create({"title": "ab"})
      assert_eq(item.title, "ab")
      assert_eq(item._errors, [error("title", "is too short (minimum is 3 characters)")])
    end
  end

  describe("bare-hash validates options") do
    test("presence on every declared field") do
      assert_eq(errors_on_save(InstBareHashItem, {}), [error("name", BLANK), error("email", BLANK)])
    end

    test("min_length") do
      errors = errors_on_save(InstBareHashItem, {"name": "a", "email": "a@b"})
      assert_eq(errors, [error("name", "is too short (minimum is 2 characters)")])
    end

    test("format") do
      errors = errors_on_save(InstBareHashItem, {"name": "OK", "email": "not-an-email"})
      assert_eq(errors, [error("email", "is invalid")])
    end

    test("a string field name with a bare hash") do
      assert_eq(errors_on_save(InstStringBareHashItem, {}), [error("title", BLANK)])
    end

    test("min_length with a string field name") do
      errors = errors_on_save(InstStringBareHashItem, {"title": "ab"})
      assert_eq(errors, [error("title", "is too short (minimum is 3 characters)")])
    end

    test("max_length refuses one character too many") do
      errors = errors_on_save(InstStringBareHashItem, {"title": "x" * 101})
      assert_eq(errors, [error("title", "is too long (maximum is 100 characters)")])
    end

    test("numericality refuses a string") do
      errors = errors_on_save(InstNumericItem, {"quantity": "not-a-number", "price": 1})
      assert_eq(errors, [error("quantity", "is not a number")])
    end

    test("numericality min and max bound the value") do
      below = errors_on_save(InstNumericItem, {"quantity": -1, "price": 1})
      assert_eq(below, [error("quantity", "must be greater than or equal to 0")])
      above = errors_on_save(InstNumericItem, {"quantity": 1001, "price": 1})
      assert_eq(above, [error("quantity", "must be less than or equal to 1000")])
    end

    test("presence and a float minimum on the same field") do
      assert_eq(errors_on_save(InstNumericItem, {"quantity": 5}), [error("price", BLANK)])
      too_cheap = errors_on_save(InstNumericItem, {"quantity": 5, "price": 0})
      assert_eq(too_cheap, [error("price", "must be greater than or equal to 0.01")])
    end
  end

  describe("mixed old and new validates syntax") do
    test("presence from both styles") do
      assert_eq(errors_on_save(InstMixedStyleItem, {}), [error("name", BLANK), error("email", BLANK)])
    end

    test("format from the bare-hash style") do
      errors = errors_on_save(InstMixedStyleItem, {"name": "OK", "email": "bad-email"})
      assert_eq(errors, [error("email", "is invalid")])
    end
  end
end

describe("against SoliDB") do
  before_each() do
    requires_solidb()
  end

  after_each() do
    wipe_collections()
  end

  describe("create") do
    test("returns a persisted instance with no errors") do
      product = InstProduct.create({"name": "ShapeOk", "price": 1.0})
      assert(product.is_a?("InstProduct"))
      assert_null(product._errors)
      assert_eq(product.errors, [])
      assert_eq(product.name, "ShapeOk")
      assert_eq(product.price, 1.0)
    end

    test("populates _key, id and _id consistently") do
      product = InstProduct.create({"name": "ShapeIds", "price": 1.0})
      assert_eq(product.id, product._key)
      assert(product._id.ends_with("inst_products/#{product._key}"), product._id)
    end

    test("a value the uniqueness rule has seen is refused") do
      assert_null(InstUniqueCode.create({"code": "A"})._errors)
      duplicate = InstUniqueCode.create({"code": "A"})
      assert_eq(duplicate._errors, [error("code", "has already been taken")])
      assert_null(duplicate._key)
      assert_eq(InstUniqueCode.count, 1)
    end
  end

  describe("_id as a key") do
    before_each() do
      widget = InstProduct.create({"name": "Widget", "price": 9.99})
    end

    test("find accepts the _key or the composite _id") do
      assert_eq(InstProduct.find(widget._key).name, "Widget")
      assert_eq(InstProduct.find(widget._id).name, "Widget")
    end

    test("Model.update accepts the composite _id") do
      InstProduct.update(widget._id, {"name": "Updated Widget"})
      updated = InstProduct.find(widget._key)
      assert_eq(updated.name, "Updated Widget")
      assert_eq(updated.price, 9.99)
    end

    test("Model.delete accepts the composite _id") do
      InstProduct.delete(widget._id)
      assert_raises("not found") do
        InstProduct.find(widget._key)
      end
    end
  end

  describe("find") do
    test("returns a class instance") do
      key = InstProduct.create({"name": "Findable", "price": 7.0})._key
      found = InstProduct.find(key)
      assert(found.is_a?("InstProduct"))
      assert_eq(found.name, "Findable")
      assert_eq(found._key, key)
    end

    test("raises RecordNotFound naming the model and the key") do
      message = assert_raises() do
        InstProduct.find("nonexistent_key_12345")
      end
      assert_contains(message, "InstProduct with id 'nonexistent_key_12345' not found")
    end
  end

  describe("all and count") do
    before_each() do
      InstProduct.create({"name": "AllTest1", "price": 1.0})
      InstProduct.create({"name": "AllTest2", "price": 2.0})
    end

    test("all returns class instances") do
      products = InstProduct.order("name").all
      assert_eq(products.map { |product| product.name }, ["AllTest1", "AllTest2"])
      assert(products.all? { |product| product.is_a?("InstProduct") })
    end

    test("all runs without parentheses") do
      assert_eq(type(InstProduct.all), "array")
      assert_eq(InstProduct.all.length, InstProduct.all().length)
    end

    test("a chain continues on the auto-invoked result") do
      assert_eq(InstProduct.all.length, 2)
    end

    test("count runs without parentheses") do
      assert_eq(InstProduct.count, 2)
      assert_eq(InstProduct.count(), 2)
      assert_eq(type(InstProduct.count), "int")
    end

    test("all_json runs without parentheses and returns the raw cursor") do
      json = InstProduct.all_json
      assert_eq(type(json), "string")
      assert_eq(json_parse(json)["count"], 2)
      assert_eq(json_parse(InstProduct.all_json())["count"], 2)
    end
  end

  describe("query builder results") do
    test("where.first returns an instance") do
      InstProduct.create({"name": "QBFirst", "price": 42.0})
      found = InstProduct.where("name = @n", {"n": "QBFirst"}).first
      assert(found.is_a?("InstProduct"))
      assert_eq(found.price, 42.0)
    end

    test("where.all returns every match as an instance") do
      InstProduct.create({"name": "QBAll", "price": 1.0})
      InstProduct.create({"name": "QBAll", "price": 2.0})
      InstProduct.create({"name": "Other", "price": 3.0})
      results = InstProduct.where("name = @n", {"n": "QBAll"}).order("price").all
      assert_eq(results.map { |product| product.price }, [1.0, 2.0])
      assert(results[0].is_a?("InstProduct"))
    end

    test("order.first returns the first instance in that order") do
      InstProduct.create({"name": "QBOrder B", "price": 200.0})
      InstProduct.create({"name": "QBOrder A", "price": 100.0})
      first = InstProduct.order("name", "asc").first
      assert(first.is_a?("InstProduct"))
      assert_eq(first.name, "QBOrder A")
    end

    test("limit caps the instances returned") do
      InstProduct.create({"name": "QBLimit1", "price": 1.0})
      InstProduct.create({"name": "QBLimit2", "price": 2.0})
      results = InstProduct.limit(1).all
      assert_eq(results.length, 1)
      assert(results[0].is_a?("InstProduct"))
    end
  end

  describe("field access") do
    test("reads persisted fields") do
      product = InstProduct.create({"name": "FieldAccess", "price": 25.0})
      assert_eq(product.name, "FieldAccess")
      assert_eq(product.price, 25.0)
      assert_eq(product._id, InstProduct.find(product._key)._id)
    end

    test("an assignment changes the instance but not the stored row") do
      product = InstProduct.create({"name": "SetField", "price": 30.0})
      product.name = "NewName"
      assert_eq(product.name, "NewName")
      assert_eq(InstProduct.find(product._key).name, "SetField")
    end
  end

  describe("save") do
    test("inserts a record without a _key and returns true") do
      product = InstProduct.new()
      product.name = "SaveNew"
      product.price = 99.0
      assert_eq(product.save, true)
      assert_eq(product.errors, [])
      found = InstProduct.find(product._key)
      assert_eq(found.name, "SaveNew")
      assert_eq(found.price, 99.0)
    end

    test("updates a record that has a _key and returns true") do
      product = InstProduct.create({"name": "SaveExisting", "price": 10.0})
      key = product._key
      product.name = "SaveUpdated"
      assert_eq(product.save, true)
      assert_eq(product._key, key)
      assert_eq(InstProduct.find(key).name, "SaveUpdated")
      assert_eq(InstProduct.count, 1)
    end

    test("runs a before_save given as a symbol") do
      item = InstCallbackItem.new()
      item.name = "  Hello  "
      # `save()` with parentheses: the bare form skips callbacks (see the next test).
      assert_eq(item.save(), true)
      assert_eq(item.name, "hello")
      assert_eq(InstCallbackItem.find(item._key).name, "hello")
    end

    test("a bare save runs before_save too") do
      item = InstCallbackItem.new()
      item.name = "  Hello  "
      item.save
      assert_eq(InstCallbackItem.find(item._key).name, "hello")
    end

    test("a refused update returns false and keeps the stored row") do
      item = InstValidatedItem.create({"title": "Valid Title"})
      item.title = ""
      assert_eq(item.update, false)
      assert_eq(item.errors, [error("title", BLANK), error("title", "is too short (minimum is 3 characters)")])
      assert_eq(InstValidatedItem.find(item._key).title, "Valid Title")
    end

    test("a successful save clears the previous errors") do
      item = InstValidatedItem.new()
      assert_eq(item.save, false)
      assert_eq(item.errors, [error("title", BLANK)])

      item.title = "Now Valid"
      assert_eq(item.save, true)
      assert_eq(item.errors, [])
    end
  end

  describe("save(hash)") do
    test("applies the hash, then inserts") do
      product = InstProduct.new()
      assert_eq(product.save({"name": "BulkSave", "price": 12.5}), true)
      assert_eq(product.name, "BulkSave")
      assert_eq(product.price, 12.5)
      assert_eq(InstProduct.find(product._key).name, "BulkSave")
    end

    test("keeps fields the hash does not mention") do
      product = InstProduct.new()
      product.name = "Original"
      assert_eq(product.save({"price": 99.0}), true)
      found = InstProduct.find(product._key)
      assert_eq(found.name, "Original")
      assert_eq(found.price, 99.0)
    end

    test("the hash wins over a field assigned before") do
      product = InstProduct.new()
      product.name = "Old"
      product.save({"name": "New"})
      assert_eq(product.name, "New")
      assert_eq(InstProduct.find(product._key).name, "New")
    end

    test("updates a record that has a _key") do
      product = InstProduct.create({"name": "SaveHashSeed", "price": 1.0})
      assert_eq(product.save({"name": "SaveHashRenamed", "price": 2.0}), true)
      found = InstProduct.find(product._key)
      assert_eq(found.name, "SaveHashRenamed")
      assert_eq(found.price, 2.0)
    end

    test("returns false and collects errors when the hash makes it invalid") do
      item = InstValidatedItem.new()
      assert_eq(item.save({"title": "no"}), false)
      assert_eq(item.errors, [error("title", "is too short (minimum is 3 characters)")])
      assert_eq(InstValidatedItem.count, 0)
    end

    test("refuses an argument that is not a hash") do
      assert_raises("expected a Hash of attributes, got string") do
        InstProduct.new().save("not a hash")
      end
    end
  end

  describe("update") do
    test("update(hash) applies the hash and persists it") do
      product = InstProduct.create({"name": "UpdHashSeed", "price": 1.0})
      assert_eq(product.update({"name": "UpdHashRenamed", "price": 2.0}), true)
      assert_eq(product.name, "UpdHashRenamed")
      assert_eq(product.price, 2.0)
      found = InstProduct.find(product._key)
      assert_eq(found.name, "UpdHashRenamed")
      assert_eq(found.price, 2.0)
    end

    test("update without arguments persists assigned fields") do
      product = InstProduct.create({"name": "UpdateBool", "price": 10.0})
      product.name = "UpdatedBool"
      assert_eq(product.update, true)
      assert_eq(product.errors, [])
      assert_eq(InstProduct.find(product._key).name, "UpdatedBool")
    end

    test("update(hash) returns false and keeps the stored row when invalid") do
      item = InstValidatedItem.create({"title": "Valid Title"})
      assert_eq(item.update({"title": "ab"}), false)
      assert_eq(item.errors, [error("title", "is too short (minimum is 3 characters)")])
      assert_eq(InstValidatedItem.find(item._key).title, "Valid Title")
    end

    test("refuses an argument that is not a hash") do
      product = InstProduct.create({"name": "UpdHashArgType", "price": 1.0})
      assert_raises("expected a Hash of attributes, got int") do
        product.update(42)
      end
    end

    test("Model.update accepts an instance as the data") do
      product = InstProduct.create({"name": "StaticUpdate", "price": 15.0})
      product.name = "StaticUpdated"
      InstProduct.update(product._key, product)
      found = InstProduct.find(product._key)
      assert_eq(found.name, "StaticUpdated")
      assert_eq(found.price, 15.0)
    end
  end

  describe("delete") do
    test("removes the document") do
      product = InstProduct.create({"name": "Deletable", "price": 3.0})
      product.delete
      assert_raises("not found") do
        InstProduct.find(product._key)
      end
      assert_eq(InstProduct.count, 0)
    end
  end

  describe("reload") do
    test("drops unsaved local changes") do
      product = InstProduct.create({"name": "ReloadMe", "price": 10.0})
      product.name = "LocalOnly"
      product.reload
      assert_eq(product.name, "ReloadMe")
    end

    test("picks up changes written elsewhere") do
      product = InstProduct.create({"name": "BeforeUpdate", "price": 20.0})
      InstProduct.update(product._key, {"name": "AfterUpdate"})
      assert_eq(product.name, "BeforeUpdate")
      product.reload
      assert_eq(product.name, "AfterUpdate")
    end

    test("returns the instance itself") do
      product = InstProduct.create({"name": "ReloadReturn", "price": 5.0})
      reloaded = product.reload
      assert(reloaded.is_a?("InstProduct"))
      assert_eq(reloaded._key, product._key)
      assert_eq(reloaded.name, "ReloadReturn")
    end
  end
end
