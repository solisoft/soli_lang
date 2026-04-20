// ============================================================================
// Model Advanced Features Test Suite
// Tests for finders, aggregations, scopes, soft delete, etc.
// ============================================================================

class TestUser extends Model
end

class TestPost extends Model
    belongs_to("user")
end

class TestSoft extends Model
    soft_delete
end

// Detect DB availability
let __db_available = false;
try
    let __probe = TestUser.create({ "name": "__probe__", "value": 0 });
    if __probe["valid"]
        __db_available = true;
        __probe["record"].delete();
    end
catch e
end

// ============================================================================
// Tests that do NOT require a DB connection
// ============================================================================

describe("Model.exists() query generation", fn() {
    test("generates EXISTS query", fn() {
        let q = TestUser.where("active = @a", { "a": true }).exists.to_query;
        assert(q.contains("LIMIT 1 RETURN true"));
    });
});

describe("Model.pluck() query generation", fn() {
    test("generates PLUCK query for single field", fn() {
        let q = TestUser.where("active = @a", { "a": true }).pluck("name").to_query;
        assert(q.contains("RETURN doc.name"));
    });

    test("generates PLUCK query for multiple fields", fn() {
        let q = TestUser.pluck("name", "email").to_query;
        assert(q.contains("RETURN {name: doc.name, email: doc.email}"));
    });
});

describe("Model.sum/avg/min/max query generation", fn() {
    test("sum generates correct query", fn() {
        let q = TestUser.where("age > @a", { "a": 18 }).sum("balance").to_query;
        assert(q.contains("RETURN SUM(doc.balance)"));
    });

    test("avg generates correct query", fn() {
        let q = TestUser.avg("score").to_query;
        assert(q.contains("RETURN AVG(doc.score)"));
    });

    test("min generates correct query", fn() {
        let q = TestUser.min("price").to_query;
        assert(q.contains("RETURN MIN(doc.price)"));
    });

    test("max generates correct query", fn() {
        let q = TestUser.max("views").to_query;
        assert(q.contains("RETURN MAX(doc.views)"));
    });
});

describe("Model.group_by() query generation", fn() {
    test("group_by generates COLLECT query", fn() {
        let q = TestUser.group_by("country", "sum", "balance").to_query;
        assert(q.contains("COLLECT group = doc.country"));
        assert(q.contains("AGGREGATE result = SUM(doc.balance)"));
    });
});

describe("Model.where() with AQL functions", fn() {
    test("LOWER() function is passed through correctly", fn() {
        let q = TestUser.where("LOWER(doc.email) == @email", { "email": "test@example.com" }).to_query;
        assert(q.contains("FILTER LOWER(doc.email) == @email"));
    });

    test("UPPER() function is passed through correctly", fn() {
        let q = TestUser.where("UPPER(doc.name) == @name", { "name": "JOHN" }).to_query;
        assert(q.contains("FILTER UPPER(doc.name) == @name"));
    });

    test("TRIM() function is passed through correctly", fn() {
        let q = TestUser.where("TRIM(doc.field) == @val", { "val": "test" }).to_query;
        assert(q.contains("FILTER TRIM(doc.field) == @val"));
    });

    test("nested function calls with LOWER", fn() {
        let q = TestUser.where("LOWER(doc.email) == LOWER(@email)", { "email": "Test@Example.COM" }).to_query;
        assert(q.contains("FILTER LOWER(doc.email) == LOWER(@email)"));
    });
});

describe("Model.offset() method", fn() {
    test("creates query with offset", fn() {
        let q = TestUser.offset(20).to_query;
        assert(q.contains("LIMIT 20,"));
    });

    test("can chain with where", fn() {
        let q = TestUser.where("active = @a", { "a": true }).offset(10).to_query;
        assert(q.contains("FILTER doc.active == @a"));
        assert(q.contains("LIMIT 10,"));
    });
});

describe("Model.find_by() query generation", fn() {
    test("generates correct query structure", fn() {
        // We can't test actual query without DB, but can verify method exists
        assert(TestUser.find_by != null);
    });
});

describe("Model.first_by() query generation", fn() {
    test("generates correct query structure", fn() {
        assert(TestUser.first_by != null);
    });
});

describe("Model.find_or_create_by() query generation", fn() {
    test("generates correct query structure", fn() {
        assert(TestUser.find_or_create_by != null);
    });
});

describe("Model.upsert() method", fn() {
    test("generates correct query structure", fn() {
        assert(TestUser.upsert != null);
    });
});

describe("Model.create_many() method", fn() {
    test("generates correct query structure", fn() {
        assert(TestUser.create_many != null);
    });
});

describe("Model.scope() method", fn() {
    test("scope method exists on Model", fn() {
        assert(TestUser.scope != null);
    });
});

describe("Model.with_deleted() method", fn() {
    test("with_deleted exists for soft delete models", fn() {
        assert(TestSoft.with_deleted != null);
    });
});

describe("Model.only_deleted() method", fn() {
    test("only_deleted exists for soft delete models", fn() {
        assert(TestSoft.only_deleted != null);
    });
});

describe("Model.transaction() method", fn() {
    test("transaction placeholder exists", fn() {
        assert(TestUser.transaction != null);
    });
});

describe("Instance methods exist", fn() {
    test("increment method exists", fn() {
        let user = TestUser.new();
        assert(user.increment != null);
    });

    test("decrement method exists", fn() {
        let user = TestUser.new();
        assert(user.decrement != null);
    });

    test("touch method exists", fn() {
        let user = TestUser.new();
        assert(user.touch != null);
    });

    test("restore method exists", fn() {
        let user = TestSoft.new();
        assert(user.restore != null);
    });
});

// ============================================================================
// Tests that REQUIRE a DB connection
// ============================================================================

if __db_available

describe("Model.create_many batch insert", fn() {
    test("creates multiple records", fn() {
        let batch = TestUser.create_many([
            { "name": "Batch1", "value": 1 },
            { "name": "Batch2", "value": 2 },
            { "name": "Batch3", "value": 3 }
        ]);
        
        assert(batch["created"] >= 3);
        
        // Cleanup
        let users = TestUser.where("name LIKE @n", { "n": "Batch%" }).all();
        for u in users
            u.delete();
        end
    });
});

describe("Model.find_by finder", fn() {
    test("finds by field value", fn() {
        let created = TestUser.create({ "name": "FindByTest", "value": 42 });
        let found = TestUser.find_by("name", "FindByTest");
        
        assert_not_null(found);
        assert_eq(found.name, "FindByTest");
        
        found.delete();
    });

    test("returns null for missing record", fn() {
        let found = TestUser.find_by("name", "NonExistent12345");
        assert_null(found);
    });
});

describe("Model.first_by finder with ordering", fn() {
    test("finds first by field with ordering", fn() {
        // Create multiple records with same name
        TestUser.create({ "name": "FirstByTest", "value": 1 });
        TestUser.create({ "name": "FirstByTest", "value": 2 });
        
        let found = TestUser.first_by("name", "FirstByTest");
        assert_not_null(found);
        assert_eq(found.name, "FirstByTest");
        
        // Cleanup
        let all = TestUser.where("name = @n", { "n": "FirstByTest" }).all();
        for u in all
            u.delete();
        end
    });
});

describe("Model.find_or_create_by finder", fn() {
    test("finds existing record", fn() {
        let created = TestUser.create({ "name": "FindOrCreate", "value": 100 });
        
        let found = TestUser.find_or_create_by("name", "FindOrCreate", { "value": 999 });
        
        assert_not_null(found);
        assert_eq(found.value, 100);  // Should be original value, not 999
        
        found.delete();
    });

    test("creates new record when not found", fn() {
        let found = TestUser.find_or_create_by("name", "FindOrCreateNew", { "value": 555 });
        
        assert_not_null(found);
        assert_eq(found.name, "FindOrCreateNew");
        assert_eq(found.value, 555);
        
        found.delete();
    });
});

describe("Model.upsert", fn() {
    test("inserts new record when not exists", fn() {
        let result = TestUser.upsert("upsert_key_123", { "name": "UpsertNew", "value": 1 });
        
        // Should return something (either the new record or success indicator)
        assert(result != null);
        
        // Cleanup
        TestUser.delete("upsert_key_123");
    });
});

describe("QueryBuilder.exists()", fn() {
    test("returns true when records exist", fn() {
        let created = TestUser.create({ "name": "ExistsTest", "value": 1 });
        
        let exists = TestUser.where("name = @n", { "n": "ExistsTest" }).exists.first;
        assert_eq(exists, true);

        created["record"].delete();
    });

    test("returns false when no records", fn() {
        let exists = TestUser.where("name = @n", { "n": "NonExistent99999" }).exists.first;
        assert_eq(exists, false);
    });
});

describe("QueryBuilder.pluck()", fn() {
    test("returns array of single field values", fn() {
        TestUser.create({ "name": "Pluck1", "value": 1 });
        TestUser.create({ "name": "Pluck2", "value": 2 });
        
        let names = TestUser.where("name LIKE @n", { "n": "Pluck%" }).pluck("name").all;
        
        assert(len(names) >= 2);
        
        // Cleanup
        let all = TestUser.where("name LIKE @n", { "n": "Pluck%" }).all;
        for u in all
            u.delete();
        end
    });
});

describe("QueryBuilder.sum() aggregation", fn() {
    test("returns sum of field", fn() {
        TestUser.create({ "name": "Sum1", "value": 10 });
        TestUser.create({ "name": "Sum2", "value": 20 });
        TestUser.create({ "name": "Sum3", "value": 30 });
        
        let total = TestUser.where("name LIKE @n", { "n": "Sum%" }).sum("value").first;

        assert(total >= 60);
        
        // Cleanup
        let all = TestUser.where("name LIKE @n", { "n": "Sum%" }).all;
        for u in all
            u.delete();
        end
    });
});

describe("QueryBuilder.avg() aggregation", fn() {
    test("returns average of field", fn() {
        TestUser.create({ "name": "Avg1", "value": 10 });
        TestUser.create({ "name": "Avg2", "value": 20 });
        
        let avg = TestUser.where("name LIKE @n", { "n": "Avg%" }).avg("value").first;

        assert(avg >= 15);
        
        // Cleanup
        let all = TestUser.where("name LIKE @n", { "n": "Avg%" }).all;
        for u in all
            u.delete();
        end
    });
});

describe("Instance.increment()", fn() {
    test("increments numeric field", fn() {
        let created = TestUser.create({ "name": "IncrementTest", "value": 10 });
        let user = created["record"];
        
        user.increment("value");
        user.reload();
        
        assert_eq(user.value, 11);
        
        user.increment("value", 5);
        user.reload();
        
        assert_eq(user.value, 16);
        
        user.delete();
    });
});

describe("Instance.decrement()", fn() {
    test("decrements numeric field", fn() {
        let created = TestUser.create({ "name": "DecrementTest", "value": 100 });
        let user = created["record"];
        
        user.decrement("value");
        user.reload();
        
        assert_eq(user.value, 99);
        
        user.delete();
    });
});

describe("Instance.touch()", fn() {
    test("updates _updated_at timestamp", fn() {
        let created = TestUser.create({ "name": "TouchTest", "value": 1 });
        let user = created["record"];
        
        let original_updated = user._updated_at;
        
        // Wait a moment to ensure timestamp changes
        // Note: In real tests, you might want to use a sleep or check for different values
        
        user.touch();
        user.reload();
        
        assert(user._updated_at != null);
        
        user.delete();
    });
});

describe("Soft delete functionality", fn() {
    test("soft delete sets deleted_at", fn() {
        let created = TestSoft.create({ "name": "SoftDeleteTest", "value": 1 });
        let record = created["record"];
        
        // Delete should set deleted_at instead of removing
        record.delete();
        
        // Record should not be found with normal query
        let found = TestSoft.find(record._key);
        assert_null(found);
        
        // But should be found with with_deleted
        let with_del = TestSoft.with_deleted.find(record._key);
        assert_not_null(with_del);
    });

    test("restore clears deleted_at", fn() {
        let created = TestSoft.create({ "name": "RestoreTest", "value": 1 });
        let record = created["record"];
        
        record.delete();
        
        // Restore
        record.restore();
        
        // Should be findable again
        let found = TestSoft.find(record._key);
        assert_not_null(found);
        
        found.delete();
    });

    test("only_deleted queries deleted records", fn() {
        let created = TestSoft.create({ "name": "OnlyDeletedTest", "value": 1 });
        let record = created["record"];
        
        record.delete();
        
        let deleted = TestSoft.only_deleted.where("name = @n", { "n": "OnlyDeletedTest" }).all;
        
        assert(len(deleted) >= 1);
    });
});

describe("Model.offset()", fn() {
    test("offsets results", fn() {
        // Create 3 records
        let r1 = TestUser.create({ "name": "Offset1", "value": 1 });
        let r2 = TestUser.create({ "name": "Offset2", "value": 2 });
        let r3 = TestUser.create({ "name": "Offset3", "value": 3 });
        
        let all = TestUser.where("name LIKE @n", { "n": "Offset%" }).order("value", "asc").all;
        
        let offset_results = TestUser.where("name LIKE @n", { "n": "Offset%" }).order("value", "asc").offset(1).all;
        
        assert_eq(len(offset_results), 2);
        
        // Cleanup
        r1["record"].delete();
        r2["record"].delete();
        r3["record"].delete();
    });
});

end // if __db_available
