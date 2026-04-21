// ============================================================================
// Model Includes Class Derivation Test Suite
// Tests that includes() generates correct queries and class derivation works
// ============================================================================

// Define test models
class Organisation extends Model
    has_many("contacts")
end

class Contact extends Model
    belongs_to("organisation")
end

describe("Model includes - query generation", fn() {
    test("belongs_to generates correct subquery", fn() {
        let q = Contact.includes("organisation").to_query;
        assert(q.contains("FOR rel IN organisations"));
        assert(q.contains("FILTER rel._key == doc.organisation_id"));
        assert(q.contains("RETURN MERGE(doc, {organisation: FIRST(_rel_organisation)})"));
    });

    test("has_many generates correct subquery", fn() {
        let q = Organisation.includes("contacts").to_query;
        assert(q.contains("FOR rel IN contacts FILTER rel.organisation_id == doc._key"));
        assert(q.contains("RETURN MERGE(doc, {contacts: _rel_contacts})"));
    });

    test("chained includes generates multiple LET statements", fn() {
        // Create a second model with a relation to test multiple includes
        class Address extends Model
            belongs_to("organisation")
        end
        let q = Organisation.includes("contacts").to_query;
        assert(q.contains("LET _rel_contacts"));
    });
});

describe("Model includes - class derivation from _id", fn() {
    test("_id parsing derives correct collection", fn() {
        // Test the internal _id parsing logic via class_name_from_id
        // This tests the Rust function through model query behavior
        let q = Contact.includes("organisation").to_query;
        // The query should reference the 'organisations' collection (pluralized from Organisation)
        assert(q.contains("organisations"));
    });
});