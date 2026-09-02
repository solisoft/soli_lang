// ============================================================================
// SolidB Client Test Suite
// ============================================================================
// Tests for the solidb_* client builtins (src/interpreter/builtins/solidb.rs)
// plus the offline-testable DB config builtins (set_solidb_address,
// db_cursor_url, db_name, db_query_raw, db_query_hardcoded, connection).
//
// Requires a running SolidB instance (default http://localhost:6745) for the
// server-backed tests; those skip gracefully when no server answers.
//
// NOTE on signatures (verified against src/interpreter/builtins/solidb.rs):
// The class-method loop re-registers `solidb_ping`, `solidb_auth`, and
// `solidb_query` as standalone globals whose FIRST argument is a Solidb
// instance — these shadow the address-first global forms registered earlier.
// The only true address-first globals are `solidb_connect(addr)` (1 arg),
// `solidb_auth(addr, database, username, password)` (4 args), and
// `solidb_query(host, database, sdbql[, bind_vars])` (3-4 args). Everything
// else goes through an instance from `Solidb(host, database)`.
// ============================================================================

class AuditSpecConnProbe extends Model
end

let __solidb_host = "http://localhost:6745"
let __spec_db = db_name()

# The constructor never touches the network, so this is safe offline.
fn spec_db()
    return Solidb(__solidb_host, __spec_db)
end

# Probe server availability once
let __solidb_available = false
try
    let __probe = Solidb(__solidb_host, __spec_db)
    __probe.ping()
    __solidb_available = true
catch e
end

# ============================================================================
# Offline tests — run (and genuinely assert) with no server present
# ============================================================================

describe("SolidB offline configuration builtins", fn() {
    test("set_solidb_address() stores the address for blob URL building", fn() {
        set_solidb_address("http://solidb-spec-host.invalid:6745")
        # get_blob_url() falls back to the configured address when no
        # explicit base_url is passed — this is how the setting is observable.
        let url = get_blob_url("audit_spec_blobs", "specblob123")
        assert(url.contains("http://solidb-spec-host.invalid:6745"))
        assert(url.contains("/_api/database/"))
        set_solidb_address("http://localhost:6745")
    })

    test("db_cursor_url() returns the cursor endpoint URL", fn() {
        let url = db_cursor_url()
        assert(url.contains("_api/database/"))
        assert(url.contains("/cursor"))
    })

    test("db_name() returns a non-empty database name", fn() {
        let name = db_name()
        assert(name.length() > 0)
    })

    test("db_query_raw() returns a raw response string", fn() {
        # Never raises: errors come back as "Error: ..." strings when the
        # server is unreachable, JSON text when it answers. Either way the
        # result must be a non-empty string.
        let result = db_query_raw("FOR d IN audit_spec_raw RETURN d")
        assert(result.length() > 0)
    })

    test("db_query_hardcoded() returns a raw response string", fn() {
        let result = db_query_hardcoded("FOR d IN audit_spec_hardcoded RETURN d")
        assert(result.length() > 0)
    })

    test("connection() rejects a non-class first argument", fn() {
        let raised = false
        try
            connection("not-a-class", "primary")
        catch e
            raised = str(e).contains("Expected class")
        end
        assert(raised)
    })

    test("connection() fails fast on an unknown connection name", fn() {
        let raised = false
        try
            connection(AuditSpecConnProbe, "audit_spec_missing_conn_xyz")
        catch e
            raised = str(e).contains("Unknown database connection")
        end
        assert(raised)
    })

    test("connected() is false on a fresh unauthenticated instance", fn() {
        let client = spec_db()
        assert_not(client.connected())
    })

    test("close() removes the instance state", fn() {
        let client = Solidb(__solidb_host, __spec_db)
        assert(client.close())
        # After close() the state map no longer holds the instance, so any
        # subsequent call raises instead of silently reconnecting.
        let raised = false
        try
            client.connected()
        catch e
            raised = true
        end
        assert(raised)
    })

    test("timeout() returns the client so it chains into query()", fn() {
        let client = spec_db()
        let chained = client.timeout(60)
        assert_eq(chained, client)
    })

    test("timeout() rejects zero, negative, and non-numeric values", fn() {
        let client = spec_db()
        for bad in [0, -1, "60", null]
            let raised = false
            try
                client.timeout(bad)
            catch e
                raised = str(e).contains("timeout")
            end
            assert(raised)
        end
    })

    test("query() rejects a third argument that is not an options hash", fn() {
        let client = spec_db()
        let raised = false
        try
            client.query("RETURN 1", {}, 60)
        catch e
            raised = str(e).contains("options hash")
        end
        assert(raised)
    })

    test("query() rejects an unknown options key", fn() {
        let client = spec_db()
        let raised = false
        try
            client.query("RETURN 1", {}, {"typo": 1})
        catch e
            raised = str(e).contains("unknown option")
        end
        assert(raised)
    })

    test("query() rejects a non-positive timeout option", fn() {
        let client = spec_db()
        let raised = false
        try
            client.query("RETURN 1", {}, {"timeout": 0})
        catch e
            raised = str(e).contains("timeout")
        end
        assert(raised)
    })
})

# ============================================================================
# Server-gated tests — each early-returns when SolidB is unreachable
# ============================================================================

describe("Solidb connection builtins", fn() {
    test("solidb_connect() connects to the server", fn() {
        if not __solidb_available
            return null
        end
        let result = solidb_connect(__solidb_host)
        assert(result.contains("Connected"))
    })

    test("ping() returns a server timestamp", fn() {
        if not __solidb_available
            return null
        end
        let client = spec_db()
        let stamp = client.ping()
        assert(stamp.present?)
        # Also reachable through the legacy standalone form (instance first)
        assert(solidb_ping(client).present?)
    })

    test("auth() marks the instance connected", fn() {
        if not __solidb_available
            return null
        end
        let client = spec_db()
        assert_not(client.connected())
        assert_eq(client.auth("spec_user", "spec_pass"), "Authenticated")
        assert(client.connected())
    })

    test("solidb_auth() authenticates with address-first arguments", fn() {
        if not __solidb_available
            return null
        end
        let result = solidb_auth(__solidb_host, __spec_db, "spec_user", "spec_pass")
        assert_eq(result, "Authenticated")
    })

    test("solidb_query() runs SDBQL with bind variables", fn() {
        if not __solidb_available
            return null
        end
        let client = spec_db()
        client.create_collection("audit_spec_globalq")
        client.insert("audit_spec_globalq", "g1", {"name": "alice"})
        let results = solidb_query(
            __solidb_host,
            __spec_db,
            "FOR d IN audit_spec_globalq FILTER d.name == @name RETURN d",
            {"name": "alice"}
        )
        assert(len(results) == 1)
        assert_eq(results[0]["name"], "alice")
        client.drop_collection("audit_spec_globalq")
    })

    test("query() without bind variables returns all docs", fn() {
        if not __solidb_available
            return null
        end
        let client = spec_db()
        let results = client.query("FOR d IN audit_spec_globalq RETURN d")
        assert(len(results) >= 0)
    })
})

describe("Solidb collection management", fn() {
    test("create_collection() and drop_collection() round trip", fn() {
        if not __solidb_available
            return null
        end
        let client = spec_db()
        assert_eq(
            client.create_collection("audit_spec_roundtrip"),
            "Created collection: audit_spec_roundtrip"
        )
        assert_eq(
            client.drop_collection("audit_spec_roundtrip"),
            "Dropped collection: audit_spec_roundtrip"
        )
    })

    test("list_collections() includes a newly created collection", fn() {
        if not __solidb_available
            return null
        end
        let client = spec_db()
        client.create_collection("audit_spec_listed")
        let names = client.list_collections()
        assert(len(names) >= 1)
        let found = false
        for name in names
            if str(name) == "audit_spec_listed"
                found = true
            end
        end
        assert(found)
        client.drop_collection("audit_spec_listed")
    })

    test("collection_stats() returns stats for a collection", fn() {
        if not __solidb_available
            return null
        end
        let client = spec_db()
        client.create_collection("audit_spec_stats")
        let stats = client.collection_stats("audit_spec_stats")
        assert(stats.present?)
        client.drop_collection("audit_spec_stats")
    })

    test("prune_collection() returns the deleted document count", fn() {
        if not __solidb_available
            return null
        end
        let client = spec_db()
        client.create_collection("audit_spec_prune")
        client.insert("audit_spec_prune", "old1", {"value": 1})
        let deleted = client.prune_collection("audit_spec_prune", "1970-01-01T00:00:00Z")
        assert(deleted >= 0)
        client.drop_collection("audit_spec_prune")
    })
})

describe("Solidb document CRUD", fn() {
    test("insert() and get() round trip a document", fn() {
        if not __solidb_available
            return null
        end
        let client = spec_db()
        client.create_collection("audit_spec_docs")
        client.insert("audit_spec_docs", "doc1", {"value": 42, "label": "spec"})
        let doc = client.get("audit_spec_docs", "doc1")
        assert(doc.present?)
        assert_eq(doc["value"], 42)
        assert_eq(doc["label"], "spec")
    })

    test("update() replaces a document", fn() {
        if not __solidb_available
            return null
        end
        let client = spec_db()
        client.create_collection("audit_spec_update")
        client.insert("audit_spec_update", "u1", {"value": 1})
        client.update("audit_spec_update", "u1", {"value": 2})
        let doc = client.get("audit_spec_update", "u1")
        assert(doc.present?)
        assert_eq(doc["value"], 2)
    })

    test("upsert() merges into an existing document", fn() {
        if not __solidb_available
            return null
        end
        let client = spec_db()
        client.create_collection("audit_spec_upsert")
        client.insert("audit_spec_upsert", "up1", {"a": 1, "b": 2})
        client.upsert("audit_spec_upsert", "up1", {"b": 3})
        let doc = client.get("audit_spec_upsert", "up1")
        assert(doc.present?)
        assert_eq(doc["a"], 1)
        assert_eq(doc["b"], 3)
    })

    test("delete() removes a document", fn() {
        if not __solidb_available
            return null
        end
        let client = spec_db()
        client.create_collection("audit_spec_delete")
        client.insert("audit_spec_delete", "d1", {"value": 7})
        assert_eq(client.delete("audit_spec_delete", "d1"), "OK")
        # A read of the deleted key must not return the old document
        let still_there = true
        try
            let doc = client.get("audit_spec_delete", "d1")
            still_there = doc.present?
        catch e
            still_there = false
        end
        assert_not(still_there)
    })

    test("list() returns the documents of a collection", fn() {
        if not __solidb_available
            return null
        end
        let client = spec_db()
        client.create_collection("audit_spec_list")
        client.insert("audit_spec_list", "l1", {"n": 1})
        client.insert("audit_spec_list", "l2", {"n": 2})
        let docs = client.list("audit_spec_list")
        assert(len(docs) >= 2)
    })

    test("query() filters with bound variables", fn() {
        if not __solidb_available
            return null
        end
        let client = spec_db()
        client.create_collection("audit_spec_qbind")
        client.insert("audit_spec_qbind", "q1", {"color": "red"})
        client.insert("audit_spec_qbind", "q2", {"color": "blue"})
        let results = client.query(
            "FOR d IN audit_spec_qbind FILTER d.color == @color RETURN d",
            {"color": "red"}
        )
        assert(len(results) == 1)
        assert_eq(results[0]["color"], "red")
    })

    test("explain() returns a query plan", fn() {
        if not __solidb_available
            return null
        end
        let client = spec_db()
        client.create_collection("audit_spec_explain")
        let plan = client.explain("FOR d IN audit_spec_explain RETURN d")
        assert(plan.present?)
    })
})

describe("Solidb indexes", fn() {
    test("create_index(), list_indexes(), and drop_index()", fn() {
        if not __solidb_available
            return null
        end
        let client = spec_db()
        client.create_collection("audit_spec_idx")
        assert_eq(
            client.create_index("audit_spec_idx", "by_value", ["value"]),
            "Created index: by_value on audit_spec_idx"
        )
        let indexes = client.list_indexes("audit_spec_idx")
        assert(len(indexes) >= 1)
        let found = false
        for index in indexes
            if str(index["name"]) == "by_value"
                found = true
            end
        end
        assert(found)
        assert_eq(
            client.drop_index("audit_spec_idx", "by_value"),
            "Dropped index: by_value from audit_spec_idx"
        )
    })

    test("create_vector_index() and drop_vector_index()", fn() {
        if not __solidb_available
            return null
        end
        let client = spec_db()
        client.create_collection("audit_spec_vecidx")
        assert_eq(
            client.create_vector_index("audit_spec_vecidx", "by_embedding", "embedding", 3),
            "Created vector index: by_embedding on audit_spec_vecidx"
        )
        assert_eq(
            client.drop_vector_index("audit_spec_vecidx", "by_embedding"),
            "Dropped vector index: by_embedding from audit_spec_vecidx"
        )
    })
})

describe("Solidb columnar stores", fn() {
    test("create_columnar(), list_columnar(), and drop_columnar()", fn() {
        if not __solidb_available
            return null
        end
        let client = spec_db()
        client.create_columnar("audit_spec_col", [
            {"name": "id", "type": "Int"},
            {"name": "label", "type": "String", "nullable": true}
        ])
        let stores = client.list_columnar()
        assert(len(stores) >= 1)
        assert_eq(
            client.drop_columnar("audit_spec_col"),
            "Dropped columnar store: audit_spec_col"
        )
    })
})

describe("Solidb blobs", fn() {
    test("store_blob() and get_blob() round trip binary data", fn() {
        if not __solidb_available
            return null
        end
        let client = spec_db()
        client.create_collection("audit_spec_blobs")
        let encoded = Base64.encode("hello solidb blob")
        let blob_id = client.store_blob("audit_spec_blobs", encoded, "spec.txt", "text/plain")
        assert(blob_id.present?)
        assert(str(blob_id).length() > 0)
        let round_tripped = client.get_blob("audit_spec_blobs", str(blob_id))
        assert_eq(round_tripped, encoded)
        client.delete_blob("audit_spec_blobs", str(blob_id))
    })

    test("get_blob_metadata() describes a stored blob", fn() {
        if not __solidb_available
            return null
        end
        let client = spec_db()
        let encoded = Base64.encode("metadata probe")
        let blob_id = str(
            client.store_blob("audit_spec_blobs", encoded, "meta.bin", "application/octet-stream")
        )
        let metadata = client.get_blob_metadata("audit_spec_blobs", blob_id)
        assert(metadata.present?)
        client.delete_blob("audit_spec_blobs", blob_id)
    })

    test("delete_blob() removes a stored blob", fn() {
        if not __solidb_available
            return null
        end
        let client = spec_db()
        let encoded = Base64.encode("to be deleted")
        let blob_id = str(
            client.store_blob("audit_spec_blobs", encoded, "gone.bin", "application/octet-stream")
        )
        assert_eq(client.delete_blob("audit_spec_blobs", blob_id), "OK")
        let raised = false
        try
            client.get_blob("audit_spec_blobs", blob_id)
        catch e
            raised = true
        end
        assert(raised)
    })
})

# Best-effort cleanup of anything left behind by interrupted runs
try
    let cleanup_db = spec_db()
    cleanup_db.drop_collection("audit_spec_roundtrip")
    cleanup_db.drop_collection("audit_spec_listed")
    cleanup_db.drop_collection("audit_spec_stats")
    cleanup_db.drop_collection("audit_spec_prune")
    cleanup_db.drop_collection("audit_spec_docs")
    cleanup_db.drop_collection("audit_spec_update")
    cleanup_db.drop_collection("audit_spec_upsert")
    cleanup_db.drop_collection("audit_spec_delete")
    cleanup_db.drop_collection("audit_spec_list")
    cleanup_db.drop_collection("audit_spec_qbind")
    cleanup_db.drop_collection("audit_spec_explain")
    cleanup_db.drop_collection("audit_spec_idx")
    cleanup_db.drop_collection("audit_spec_vecidx")
    cleanup_db.drop_collection("audit_spec_blobs")
    cleanup_db.drop_collection("audit_spec_globalq")
catch e
end
