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

class ValidatedItem extends Model
    validates("title", { "presence": true })
    validates("title", { "min_length": 3 })
end

// Detect DB availability
let __db_available = false;
try
    let __probe = Product.create({ "name": "__probe__", "price": 0 });
    if !__probe.nil? and __probe["valid"]
        __db_available = true;
        __probe["record"].delete();
    end
catch e
end

// ============================================================================
// Tests that do NOT require a DB connection
// ============================================================================

describe("Instance .errors (no DB)", fn() {
    test("returns empty array on fresh instance", fn() {
        let product = Product.new();
        assert_eq(len(product.errors), 0);
    });
});

describe("Instance .save() with validation errors (no DB)", fn() {
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

    test("min_length validation on save", fn() {
        let item = ValidatedItem.new();
        item.title = "ab";  // too short (min 3)
        let ok = item.save();
        assert_eq(ok, false);

        let errors = item.errors;
        assert(len(errors) > 0);
        let has_length_error = false;
        for e in errors
            if e["message"].contains("too short")
                has_length_error = true;
            end
        end
        assert(has_length_error);
    });
});

// ============================================================================
// Tests that REQUIRE a DB connection
// ============================================================================

if __db_available

describe("Model _id key normalization", fn() {
    test("Model.find uses normalized key from _id", fn() {
        let result = Product.create({ "name": "Widget", "price": 9.99 });
        assert(result["valid"]);

        let record = result["record"];
        let id = record._id;
        let key = record._key;

        let found_by_key = Product.find(key);
        assert_not_null(found_by_key);
        assert_eq(found_by_key.name, "Widget");

        let found_by_id = Product.find(id);
        assert_not_null(found_by_id);
        assert_eq(found_by_id.name, "Widget");

        found_by_key.delete();
    });

    test("Model.update works with _id composite key", fn() {
        let result = Product.create({ "name": "Gadget", "price": 19.99 });
        let record = result["record"];

        Product.update(record._id, { "name": "Updated Gadget" });

        let updated = Product.find(record._key);
        assert_eq(updated.name, "Updated Gadget");

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

describe("Model.create returns instance", fn() {
    test("record is a class instance", fn() {
        let result = Product.create({ "name": "Test Item", "price": 5.00 });
        assert(result["valid"]);

        let record = result["record"];
        assert(record.is_a?(Product));
        assert_eq(record.name, "Test Item");
        assert_not_null(record._key);

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

        let first = all[0];
        assert(first.is_a?(Product));
        assert_not_null(first._key);
        assert_not_null(first.name);

        r1["record"].delete();
        r2["record"].delete();
    });
});

describe("Instance .update()", fn() {
    test("persists changed fields to DB", fn() {
        let result = Product.create({ "name": "Original", "price": 10.00 });
        let product = result["record"];

        product.name = "Modified";
        let ok = product.update();
        assert_eq(ok, true);

        let reloaded = Product.find(product._key);
        assert_eq(reloaded.name, "Modified");

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

describe("Model.update with instance data", fn() {
    test("accepts instance as data argument", fn() {
        let result = Product.create({ "name": "StaticUpdate", "price": 15.00 });
        let product = result["record"];

        product.name = "StaticUpdated";
        Product.update(product._key, product);

        let reloaded = Product.find(product._key);
        assert_eq(reloaded.name, "StaticUpdated");

        reloaded.delete();
    });
});

describe("QueryBuilder returns instances", fn() {
    test("where().first returns an instance", fn() {
        let result = Product.create({ "name": "QBFirst", "price": 42.00 });

        let found = Product.where("name = @n", { "n": "QBFirst" }).first;
        assert_not_null(found);
        assert(found.is_a?(Product));
        assert_eq(found.name, "QBFirst");

        found.delete();
    });

    test("where().all returns array of instances", fn() {
        let r1 = Product.create({ "name": "QBAll", "price": 1.00 });
        let r2 = Product.create({ "name": "QBAll", "price": 2.00 });

        let results = Product.where("name = @n", { "n": "QBAll" }).all;
        assert(len(results) >= 2);
        assert(results[0].is_a?(Product));

        r1["record"].delete();
        r2["record"].delete();
    });

    test("order().first returns an instance", fn() {
        let r1 = Product.create({ "name": "QBOrder A", "price": 100.00 });
        let r2 = Product.create({ "name": "QBOrder B", "price": 200.00 });

        let first = Product.order("name", "asc").first;
        assert_not_null(first);
        assert(first.is_a?(Product));

        r1["record"].delete();
        r2["record"].delete();
    });

    test("limit returns instances", fn() {
        let r1 = Product.create({ "name": "QBLimit1", "price": 1.00 });
        let r2 = Product.create({ "name": "QBLimit2", "price": 2.00 });

        let results = Product.limit(1).all;
        assert_eq(len(results), 1);
        assert(results[0].is_a?(Product));

        r1["record"].delete();
        r2["record"].delete();
    });
});

describe("Instance field access", fn() {
    test("can read all fields from instance", fn() {
        let result = Product.create({ "name": "FieldAccess", "price": 25.00 });
        let product = result["record"];

        assert_eq(product.name, "FieldAccess");
        assert_not_null(product._key);
        assert_not_null(product._id);

        product.delete();
    });

    test("can set fields on instance", fn() {
        let result = Product.create({ "name": "SetField", "price": 30.00 });
        let product = result["record"];

        product.name = "NewName";
        assert_eq(product.name, "NewName");

        product.delete();
    });
});

describe("Instance .save()", fn() {
    test("inserts new record when no _key, returns true", fn() {
        let product = Product.new();
        product.name = "SaveNew";
        product.price = 99.00;

        let result = product.save();
        assert_eq(result, true);
        assert_not_null(product._key);
        assert_eq(product.name, "SaveNew");

        let found = Product.find(product._key);
        assert_not_null(found);
        assert_eq(found.name, "SaveNew");

        product.delete();
    });

    test("updates existing record when _key present, returns true", fn() {
        let result = Product.create({ "name": "SaveExisting", "price": 10.00 });
        let product = result["record"];

        product.name = "SaveUpdated";
        let ok = product.save();
        assert_eq(ok, true);

        let found = Product.find(product._key);
        assert_eq(found.name, "SaveUpdated");

        found.delete();
    });

    test("populates _key on instance after insert", fn() {
        let product = Product.new();
        product.name = "SaveReturn";
        product.price = 5.00;

        product.save();
        assert_not_null(product._key);

        product.delete();
    });

    test("errors is empty after successful save", fn() {
        let product = Product.new();
        product.name = "NoErrors";
        product.price = 1.00;

        product.save();
        assert_eq(len(product.errors), 0);

        product.delete();
    });
});

describe("Instance .save(hash)", fn() {
    test("applies hash attributes then inserts", fn() {
        let p = Product.new();
        let ok = p.save({ "name": "BulkSave", "price": 12.50 });
        assert_eq(ok, true);
        assert_not_null(p._key);
        assert_eq(p.name, "BulkSave");
        assert_eq(p.price, 12.50);

        let found = Product.find(p._key);
        assert_eq(found.name, "BulkSave");

        p.delete();
    });

    test("merges hash onto pre-assigned fields without overwriting unmentioned", fn() {
        let p = Product.new();
        p.name = "Original";
        let ok = p.save({ "price": 99.00 });
        assert_eq(ok, true);
        assert_eq(p.name, "Original");
        assert_eq(p.price, 99.00);

        p.delete();
    });

    test("hash value wins over pre-assigned field on conflict", fn() {
        let p = Product.new();
        p.name = "Old";
        p.save({ "name": "New" });
        assert_eq(p.name, "New");

        p.delete();
    });

    test("updates existing record when _key is present", fn() {
        let result = Product.create({ "name": "SaveHashSeed", "price": 1.00 });
        let p = result["record"];

        let ok = p.save({ "name": "SaveHashRenamed", "price": 2.00 });
        assert_eq(ok, true);

        let found = Product.find(p._key);
        assert_eq(found.name, "SaveHashRenamed");
        assert_eq(found.price, 2.00);

        found.delete();
    });

    test("surfaces validation errors when hash produces invalid state", fn() {
        let item = ValidatedItem.new();
        let ok = item.save({ "title": "" });
        assert_eq(ok, false);
        assert(len(item.errors) > 0);
    });

    test("non-hash argument raises", fn() {
        let p = Product.new();
        let raised = false;
        try
            p.save("not a hash");
        catch e
            raised = true;
        end
        assert_eq(raised, true);
    });
});

describe("Instance .update(hash)", fn() {
    test("applies hash then updates existing record", fn() {
        let result = Product.create({ "name": "UpdHashSeed", "price": 1.00 });
        let p = result["record"];

        let ok = p.update({ "name": "UpdHashRenamed", "price": 2.00 });
        assert_eq(ok, true);
        assert_eq(p.name, "UpdHashRenamed");
        assert_eq(p.price, 2.00);

        let found = Product.find(p._key);
        assert_eq(found.name, "UpdHashRenamed");
        assert_eq(found.price, 2.00);

        found.delete();
    });

    test("no-arg update() still works (backcompat)", fn() {
        let result = Product.create({ "name": "UpdBackcompat", "price": 1.00 });
        let p = result["record"];

        p.name = "UpdBackcompatRenamed";
        let ok = p.update();
        assert_eq(ok, true);

        p.delete();
    });

    test("surfaces validation errors when hash produces invalid state", fn() {
        let result = ValidatedItem.create({ "title": "Valid Title" });
        let item = result["record"];

        let ok = item.update({ "title": "" });
        assert_eq(ok, false);
        assert(len(item.errors) > 0);

        item.update({ "title": "Valid Title" });
        item.delete();
    });

    test("non-hash argument raises", fn() {
        let result = Product.create({ "name": "UpdHashArgType", "price": 1.00 });
        let p = result["record"];

        let raised = false;
        try
            p.update(42);
        catch e
            raised = true;
        end
        assert_eq(raised, true);

        p.delete();
    });
});

describe("Instance .update() returns boolean", fn() {
    test("returns true on success", fn() {
        let result = Product.create({ "name": "UpdateBool", "price": 10.00 });
        let product = result["record"];

        product.name = "UpdatedBool";
        let ok = product.update();
        assert_eq(ok, true);

        let found = Product.find(product._key);
        assert_eq(found.name, "UpdatedBool");

        found.delete();
    });

    test("errors is empty after successful update", fn() {
        let result = Product.create({ "name": "UpdateNoErr", "price": 10.00 });
        let product = result["record"];

        product.name = "UpdatedNoErr";
        product.update();
        assert_eq(len(product.errors), 0);

        product.delete();
    });
});

describe("Instance .save() with validation errors (DB)", fn() {
    test("returns false when validation fails on update", fn() {
        let result = ValidatedItem.create({ "title": "Valid Title" });
        assert(result["valid"]);
        let item = result["record"];

        item.title = "";
        let ok = item.update();
        assert_eq(ok, false);

        let errors = item.errors;
        assert(len(errors) > 0);

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

        item.delete();
    });
});

describe("Instance .errors (DB)", fn() {
    test("returns empty array after successful operations", fn() {
        let result = Product.create({ "name": "ErrTest", "price": 5.00 });
        let product = result["record"];

        product.name = "ErrTestUpdated";
        product.save();
        assert_eq(len(product.errors), 0);

        product.delete();
    });
});

describe("Instance .reload()", fn() {
    test("refreshes fields from DB", fn() {
        let result = Product.create({ "name": "ReloadMe", "price": 10.00 });
        let product = result["record"];

        product.name = "LocalOnly";
        assert_eq(product.name, "LocalOnly");

        product.reload();
        assert_eq(product.name, "ReloadMe");

        product.delete();
    });

    test("picks up changes made by others", fn() {
        let result = Product.create({ "name": "BeforeUpdate", "price": 20.00 });
        let product = result["record"];

        Product.update(product._key, { "name": "AfterUpdate" });

        assert_eq(product.name, "BeforeUpdate");

        product.reload();
        assert_eq(product.name, "AfterUpdate");

        product.delete();
    });

    test("returns the instance itself", fn() {
        let result = Product.create({ "name": "ReloadReturn", "price": 5.00 });
        let product = result["record"];

        let reloaded = product.reload();
        assert(reloaded.is_a?(Product));
        assert_eq(reloaded._key, product._key);

        product.delete();
    });
});

end // if __db_available
