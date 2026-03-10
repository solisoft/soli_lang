// ============================================================================
// Model Instances Test Suite
// Tests that Model methods return class instances and instance methods work
// ============================================================================

// Define test models
class Product extends Model
end

class Order extends Model
    has_many("products")
end

// ============================================================================
// normalize_key: _id composite keys should work with find/update/delete
// ============================================================================

describe("Model _id key normalization", fn() {
    test("Model.find uses normalized key from _id", fn() {
        // Create a product so we have a real _id to test with
        let result = Product.create({ "name": "Widget", "price": 9.99 });
        assert(result["valid"]);

        let record = result["record"];
        let id = record._id;
        let key = record._key;

        // _id contains collection prefix (e.g., "default:products/UUID")
        // find() should work with both _id and _key
        let found_by_key = Product.find(key);
        assert_not_null(found_by_key);
        assert_eq(found_by_key.name, "Widget");

        let found_by_id = Product.find(id);
        assert_not_null(found_by_id);
        assert_eq(found_by_id.name, "Widget");

        // Cleanup
        found_by_key.delete();
    });

    test("Model.update works with _id composite key", fn() {
        let result = Product.create({ "name": "Gadget", "price": 19.99 });
        let record = result["record"];

        // Update using _id (the composite key) should not 404
        Product.update(record._id, { "name": "Updated Gadget" });

        let updated = Product.find(record._key);
        assert_eq(updated.name, "Updated Gadget");

        // Cleanup
        updated.delete();
    });

    test("Model.delete works with _id composite key", fn() {
        let result = Product.create({ "name": "Temporary", "price": 1.00 });
        let record = result["record"];

        Product.delete(record._id);

        let deleted = Product.find(record._key);
        assert_null(deleted);
    });
});

// ============================================================================
// Return class instances from Model methods
// ============================================================================

describe("Model.create returns instance", fn() {
    test("record is a class instance", fn() {
        let result = Product.create({ "name": "Test Item", "price": 5.00 });
        assert(result["valid"]);

        let record = result["record"];
        assert(record.is_a?(Product));
        assert_eq(record.name, "Test Item");
        assert_not_null(record._key);

        // Cleanup
        record.delete();
    });
});

describe("Model.find returns instance", fn() {
    test("returns a class instance", fn() {
        let result = Product.create({ "name": "Findable", "price": 7.00 });
        let key = result["record"]._key;

        let found = Product.find(key);
        assert(found.is_a?(Product));
        assert_eq(found.name, "Findable");
        assert_eq(found._key, key);

        // Cleanup
        found.delete();
    });

    test("returns null for missing document", fn() {
        let found = Product.find("nonexistent_key_12345");
        assert_null(found);
    });
});

describe("Model.all returns instances", fn() {
    test("returns array of class instances", fn() {
        let r1 = Product.create({ "name": "AllTest1", "price": 1.00 });
        let r2 = Product.create({ "name": "AllTest2", "price": 2.00 });

        let all = Product.all();
        assert(len(all) >= 2);

        // Each element should be a Product instance
        let first = all[0];
        assert(first.is_a?(Product));
        assert_not_null(first._key);
        assert_not_null(first.name);

        // Cleanup
        r1["record"].delete();
        r2["record"].delete();
    });
});

// ============================================================================
// Instance methods: .update() and .delete()
// ============================================================================

describe("Instance .update()", fn() {
    test("persists changed fields to DB", fn() {
        let result = Product.create({ "name": "Original", "price": 10.00 });
        let product = result["record"];

        // Modify the instance field
        product.name = "Modified";
        let ok = product.update();
        assert_eq(ok, true);

        // Reload from DB to confirm
        let reloaded = Product.find(product._key);
        assert_eq(reloaded.name, "Modified");

        // Cleanup
        reloaded.delete();
    });
});

describe("Instance .delete()", fn() {
    test("removes document from DB", fn() {
        let result = Product.create({ "name": "Deletable", "price": 3.00 });
        let product = result["record"];
        let key = product._key;

        product.delete();

        let gone = Product.find(key);
        assert_null(gone);
    });
});

// ============================================================================
// Model.update static method accepts instance as data
// ============================================================================

describe("Model.update with instance data", fn() {
    test("accepts instance as data argument", fn() {
        let result = Product.create({ "name": "StaticUpdate", "price": 15.00 });
        let product = result["record"];

        product.name = "StaticUpdated";
        Product.update(product._key, product);

        let reloaded = Product.find(product._key);
        assert_eq(reloaded.name, "StaticUpdated");

        // Cleanup
        reloaded.delete();
    });
});

// ============================================================================
// QueryBuilder returns instances
// ============================================================================

describe("QueryBuilder returns instances", fn() {
    test("where().first returns an instance", fn() {
        let result = Product.create({ "name": "QBFirst", "price": 42.00 });

        let found = Product.where("name = @n", { "n": "QBFirst" }).first;
        assert_not_null(found);
        assert(found.is_a?(Product));
        assert_eq(found.name, "QBFirst");

        // Cleanup
        found.delete();
    });

    test("where().all returns array of instances", fn() {
        let r1 = Product.create({ "name": "QBAll", "price": 1.00 });
        let r2 = Product.create({ "name": "QBAll", "price": 2.00 });

        let results = Product.where("name = @n", { "n": "QBAll" }).all;
        assert(len(results) >= 2);
        assert(results[0].is_a?(Product));

        // Cleanup
        r1["record"].delete();
        r2["record"].delete();
    });

    test("order().first returns an instance", fn() {
        let r1 = Product.create({ "name": "QBOrder A", "price": 100.00 });
        let r2 = Product.create({ "name": "QBOrder B", "price": 200.00 });

        let first = Product.order("name", "asc").first;
        assert_not_null(first);
        assert(first.is_a?(Product));

        // Cleanup
        r1["record"].delete();
        r2["record"].delete();
    });

    test("limit returns instances", fn() {
        let r1 = Product.create({ "name": "QBLimit1", "price": 1.00 });
        let r2 = Product.create({ "name": "QBLimit2", "price": 2.00 });

        let results = Product.limit(1).all;
        assert_eq(len(results), 1);
        assert(results[0].is_a?(Product));

        // Cleanup
        r1["record"].delete();
        r2["record"].delete();
    });
});

// ============================================================================
// Instance field access
// ============================================================================

describe("Instance field access", fn() {
    test("can read all fields from instance", fn() {
        let result = Product.create({ "name": "FieldAccess", "price": 25.00 });
        let product = result["record"];

        assert_eq(product.name, "FieldAccess");
        assert_not_null(product._key);
        assert_not_null(product._id);

        // Cleanup
        product.delete();
    });

    test("can set fields on instance", fn() {
        let result = Product.create({ "name": "SetField", "price": 30.00 });
        let product = result["record"];

        product.name = "NewName";
        assert_eq(product.name, "NewName");

        // Cleanup
        product.delete();
    });
});

// ============================================================================
// Instance .save() — insert or update, returns true/false
// ============================================================================

describe("Instance .save()", fn() {
    test("inserts new record when no _key, returns true", fn() {
        let product = Product.new();
        product.name = "SaveNew";
        product.price = 99.00;

        let result = product.save();
        assert_eq(result, true);
        assert_not_null(product._key);
        assert_eq(product.name, "SaveNew");

        // Verify in DB
        let found = Product.find(product._key);
        assert_not_null(found);
        assert_eq(found.name, "SaveNew");

        // Cleanup
        product.delete();
    });

    test("updates existing record when _key present, returns true", fn() {
        let result = Product.create({ "name": "SaveExisting", "price": 10.00 });
        let product = result["record"];

        product.name = "SaveUpdated";
        let ok = product.save();
        assert_eq(ok, true);

        // Verify in DB
        let found = Product.find(product._key);
        assert_eq(found.name, "SaveUpdated");

        // Cleanup
        found.delete();
    });

    test("populates _key on instance after insert", fn() {
        let product = Product.new();
        product.name = "SaveReturn";
        product.price = 5.00;

        product.save();
        assert_not_null(product._key);

        // Cleanup
        product.delete();
    });

    test("errors is empty after successful save", fn() {
        let product = Product.new();
        product.name = "NoErrors";
        product.price = 1.00;

        product.save();
        assert_eq(len(product.errors), 0);

        // Cleanup
        product.delete();
    });
});

// ============================================================================
// Instance .update() returns true/false
// ============================================================================

describe("Instance .update() returns boolean", fn() {
    test("returns true on success", fn() {
        let result = Product.create({ "name": "UpdateBool", "price": 10.00 });
        let product = result["record"];

        product.name = "UpdatedBool";
        let ok = product.update();
        assert_eq(ok, true);

        // Verify in DB
        let found = Product.find(product._key);
        assert_eq(found.name, "UpdatedBool");

        // Cleanup
        found.delete();
    });

    test("errors is empty after successful update", fn() {
        let result = Product.create({ "name": "UpdateNoErr", "price": 10.00 });
        let product = result["record"];

        product.name = "UpdatedNoErr";
        product.update();
        assert_eq(len(product.errors), 0);

        // Cleanup
        product.delete();
    });
});

// ============================================================================
// Validation errors on .save() and .update()
// ============================================================================

class ValidatedItem extends Model
    validates("title", { "presence": true })
    validates("title", { "min_length": 3 })
end

describe("Instance .save() with validation errors", fn() {
    test("returns false when validation fails on insert", fn() {
        let item = ValidatedItem.new();
        // title is missing — presence validation should fail
        let ok = item.save();
        assert_eq(ok, false);
    });

    test("stores errors on instance after failed save", fn() {
        let item = ValidatedItem.new();
        item.save();

        let errors = item.errors;
        assert(len(errors) > 0);
        assert_eq(errors[0]["field"], "title");
        assert_eq(errors[0]["message"], "can't be blank");
    });

    test("returns false when validation fails on update", fn() {
        // First create a valid record
        let result = ValidatedItem.create({ "title": "Valid Title" });
        assert(result["valid"]);
        let item = result["record"];

        // Now blank the field and try to update
        item.title = "";
        let ok = item.update();
        assert_eq(ok, false);

        let errors = item.errors;
        assert(len(errors) > 0);

        // Cleanup
        item.title = "Valid Title";
        item.save();
        item.delete();
    });

    test("clears errors after successful save", fn() {
        let item = ValidatedItem.new();
        item.save();  // fails — no title
        assert(len(item.errors) > 0);

        item.title = "Now Valid";
        let ok = item.save();
        assert_eq(ok, true);
        assert_eq(len(item.errors), 0);

        // Cleanup
        item.delete();
    });

    test("min_length validation on save", fn() {
        let item = ValidatedItem.new();
        item.title = "ab";  // too short (min 3)
        let ok = item.save();
        assert_eq(ok, false);

        let errors = item.errors;
        assert(len(errors) > 0);
        // Should have min_length error
        let has_length_error = false;
        for e in errors
            if e["message"].includes?("too short")
                has_length_error = true;
            end
        end
        assert(has_length_error);
    });
});

describe("Instance .errors", fn() {
    test("returns empty array on fresh instance", fn() {
        let product = Product.new();
        assert_eq(len(product.errors), 0);
    });

    test("returns empty array after successful operations", fn() {
        let result = Product.create({ "name": "ErrTest", "price": 5.00 });
        let product = result["record"];

        product.name = "ErrTestUpdated";
        product.save();
        assert_eq(len(product.errors), 0);

        // Cleanup
        product.delete();
    });
});

// ============================================================================
// Instance .reload()
// ============================================================================

describe("Instance .reload()", fn() {
    test("refreshes fields from DB", fn() {
        let result = Product.create({ "name": "ReloadMe", "price": 10.00 });
        let product = result["record"];

        // Modify locally without saving
        product.name = "LocalOnly";
        assert_eq(product.name, "LocalOnly");

        // Reload should restore DB values
        product.reload();
        assert_eq(product.name, "ReloadMe");

        // Cleanup
        product.delete();
    });

    test("picks up changes made by others", fn() {
        let result = Product.create({ "name": "BeforeUpdate", "price": 20.00 });
        let product = result["record"];

        // Update via static method (simulating another process)
        Product.update(product._key, { "name": "AfterUpdate" });

        // Instance still has old value
        assert_eq(product.name, "BeforeUpdate");

        // Reload fetches the updated value
        product.reload();
        assert_eq(product.name, "AfterUpdate");

        // Cleanup
        product.delete();
    });

    test("returns the instance itself", fn() {
        let result = Product.create({ "name": "ReloadReturn", "price": 5.00 });
        let product = result["record"];

        let reloaded = product.reload();
        assert(reloaded.is_a?(Product));
        assert_eq(reloaded._key, product._key);

        // Cleanup
        product.delete();
    });
});
