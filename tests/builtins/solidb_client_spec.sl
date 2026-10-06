# The raw SoliDB client: `Solidb(host, database)` instances and the
# db_* configuration builtins. Server-backed tests authenticate with the same
# credentials the models use and drop every collection they create.

class AuditSpecConnProbe < Model
end

created_collections = []

def solidb_host
  getenv("SOLIDB_HOST") || "http://localhost:6745"
end

def spec_username
  getenv("SOLIDB_USERNAME") || "spec_user"
end

def spec_password
  getenv("SOLIDB_PASSWORD") || "spec_pass"
end

# A client that has not called auth — the constructor never touches the network.
def bare_db
  Solidb(solidb_host(), db_name())
end

# A raw client on the ORM's database, with the credentials the models use.
def raw_db
  db = bare_db()
  username = getenv("SOLIDB_USERNAME")
  db.auth(username, getenv("SOLIDB_PASSWORD")) if username.present?
  db
end

def collection(name, collection_type = nil)
  raw_db().create_collection(name, collection_type)
  created_collections.push(name)
  name
end

describe("SoliDB configuration builtins") do
  after_each() do
    set_solidb_address(solidb_host())
  end

  test("set_solidb_address sets the host blob URLs are built on") do
    set_solidb_address("http://solidb-spec-host.invalid:6745")
    blob_url = get_blob_url("audit_spec_blobs", "specblob123")
    assert(blob_url.starts_with?("http://solidb-spec-host.invalid:6745/_api/database/"), blob_url)
    assert(blob_url.ends_with?("/document/audit_spec_blobs/specblob123"), blob_url)
  end

  test("db_cursor_url is the cursor endpoint of the test database") do
    assert_match(db_cursor_url(), "/_api/database/#{db_name()}/cursor$")
  end

  test("db_name is a plain database name") do
    assert_match(db_name(), "^[A-Za-z0-9_-]+$")
  end

  test("db_query_raw and db_query_hardcoded refuse a query that is not a string") do
    assert_raises("db_query_raw requires a query string") do
      db_query_raw(42)
    end
    assert_raises("db_query_hardcoded requires a query string") do
      db_query_hardcoded(nil)
    end
  end

  test("db_query_hardcoded answers the server's JSON or a legible error, never nothing") do
    assert_match(db_query_hardcoded("RETURN 1"), "^(\\{|Error: )")
  end

  test("connection refuses a first argument that is not a class") do
    assert_raises("Expected class as first argument, got string") do
      connection("not-a-class", "primary")
    end
  end

  test("connection fails fast on an unknown connection name") do
    assert_raises("Unknown database connection") do
      connection(AuditSpecConnProbe, "audit_spec_missing_conn_xyz")
    end
  end
end

describe("Solidb client without a server") do
  test("a fresh instance is not connected") do
    assert_eq(bare_db().connected(), false)
  end

  test("close drops the instance, so later calls raise") do
    client = bare_db()
    assert_eq(client.close(), true)
    assert_raises("Solidb instance not found") do
      client.connected()
    end
  end

  test("timeout returns the client so it chains") do
    client = bare_db()
    assert_eq(client.timeout(60), client)
    assert_eq(client.timeout(1.5), client)
  end

  test("timeout refuses zero, negative and non-numeric values") do
    client = bare_db()
    assert_raises("timeout() expects a positive number of seconds, got 0") do
      client.timeout(0)
    end
    assert_raises("timeout() expects a positive number of seconds, got -1") do
      client.timeout(-1)
    end
    assert_raises("timeout() expects a number of seconds, got string") do
      client.timeout("60")
    end
    assert_raises("timeout() expects a number of seconds, got null") do
      client.timeout(nil)
    end
  end

  test("query refuses a third argument that is not an options hash") do
    assert_raises("query() expects an options hash as the third argument, got int") do
      bare_db().query("RETURN 1", {}, 60)
    end
  end

  test("query refuses an unknown option") do
    assert_raises("query() unknown option 'typo'; expected timeout") do
      bare_db().query("RETURN 1", {}, {"typo": 1})
    end
  end

  test("query refuses a non-positive timeout option") do
    assert_raises("query() timeout expects a positive number of seconds, got 0") do
      bare_db().query("RETURN 1", {}, {"timeout": 0})
    end
  end
end

describe("Solidb client against a server") do
  before_each() do
    created_collections = []
    requires_solidb()
  end

  after_each() do
    created_collections.each do |name|
      raw_db().drop_collection(name) rescue nil
    end
  end

  describe("connecting") do
    test("db_query_raw returns the server's JSON") do
      assert_eq(HTTP.json_parse(db_query_raw("RETURN 1"))["result"], [1])
    end

    test("ping answers true, also through the instance-first global") do
      client = raw_db()
      assert_eq(client.ping(), true)
      assert_eq(solidb_ping(client), true)
    end

    test("auth attaches credentials and marks the instance connected") do
      client = bare_db()
      assert_eq(client.connected(), false)
      assert_eq(client.auth(spec_username(), spec_password()), "Authenticated")
      assert_eq(client.connected(), true)
    end

    test("auth only stores the credentials: a wrong password fails on the next call") do
      skip("SoliDB accepts anonymous requests here") if getenv("SOLIDB_USERNAME").blank?

      client = bare_db()
      assert_eq(client.auth(getenv("SOLIDB_USERNAME"), "wrong-password"), "Authenticated")
      assert_raises("401 Unauthorized") do
        client.query("RETURN 1")
      end
    end

    test("solidb_auth and solidb_query take the instance first") do
      client = bare_db()
      assert_eq(solidb_auth(client, spec_username(), spec_password()), "Authenticated")
      assert_eq(solidb_query(client, "RETURN 2"), [2])
    end

    test("the address-first one-shot globals work as documented") do
      pending("bug: the instance-first solidb_auth/solidb_query/solidb_ping globals shadow the address-first forms")
      assert_eq(solidb_auth(solidb_host(), db_name(), spec_username(), spec_password()), "Authenticated")
      assert_eq(solidb_query(solidb_host(), db_name(), "RETURN @n", {"n": 3}), [3])
    end

    test("solidb_connect pings the server") do
      skip("solidb_connect sends no credentials and this SoliDB requires them") if getenv("SOLIDB_USERNAME").present?

      assert_match(solidb_connect(solidb_host()), "^Connected")
    end
  end

  describe("queries") do
    test("query binds variables") do
      name = collection("client_spec_query")
      client = raw_db()
      client.insert(name, "red", {"color": "red"})
      client.insert(name, "blue", {"color": "blue"})
      keys = client.query("FOR d IN client_spec_query FILTER d.color == @color RETURN d._key", {"color": "red"})
      assert_eq(keys, ["red"])
    end

    test("query without bind variables returns every row") do
      name = collection("client_spec_all")
      client = raw_db()
      client.insert(name, "a", {"n": 1})
      client.insert(name, "b", {"n": 2})
      assert_eq(client.query("FOR d IN client_spec_all SORT d.n RETURN d.n"), [1, 2])
    end

    test("timeout chains into query and query takes a timeout option") do
      client = raw_db()
      assert_eq(client.timeout(30).query("RETURN 1"), [1])
      assert_eq(client.query("RETURN 2", {}, {"timeout": 30}), [2])
    end

    test("explain returns the plan without running the query") do
      name = collection("client_spec_explain")
      plan = raw_db().explain("FOR d IN client_spec_explain RETURN d")
      assert_eq(plan["collections"][0]["name"], name)
      assert_eq(plan["collections"][0]["access_type"], "full_scan")
    end
  end

  describe("collections") do
    test("create_collection and drop_collection report what they did") do
      client = raw_db()
      assert_eq(client.create_collection("client_spec_roundtrip"), "Created collection: client_spec_roundtrip")
      assert_eq(client.drop_collection("client_spec_roundtrip"), "Dropped collection: client_spec_roundtrip")
    end

    test("create_collection refuses an existing name") do
      name = collection("client_spec_twice")
      assert_raises("CollectionAlreadyExists") do
        raw_db().create_collection(name)
      end
    end

    test("drop_collection raises for a missing collection") do
      assert_raises("CollectionNotFound") do
        raw_db().drop_collection("client_spec_never_created")
      end
    end

    test("list_collections names each collection with its type") do
      collection("client_spec_listed")
      collection("client_spec_edges", "edge")
      types = {}
      raw_db().list_collections.each do |info|
        types[info["name"]] = info["type"]
      end
      assert_eq(types["client_spec_listed"], "document")
      assert_eq(types["client_spec_edges"], "edge")
    end

    test("collection_stats counts the documents") do
      name = collection("client_spec_stats")
      raw_db().insert(name, "one", {"n": 1})
      stats = raw_db().collection_stats(name)
      assert_eq(stats["collection"], name)
      assert_eq(stats["type"], "document")
      assert_eq(stats["document_count"], 1)
    end

    test("prune_collection returns how many rows it deleted") do
      name = collection("client_spec_prune", "timeseries")
      assert_eq(raw_db().prune_collection(name, "1970-01-01T00:00:00Z"), 0)
    end

    test("create_collection refuses the columnar type") do
      pending("bug: create_collection(name, \"columnar\") creates a collection instead of raising")
      assert_raises("create_columnar") do
        raw_db().create_collection("client_spec_columnar_type", "columnar")
      end
    end
  end

  describe("documents") do
    test("insert returns the stored document and get reads it back") do
      name = collection("client_spec_docs")
      client = raw_db()
      inserted = client.insert(name, "doc1", {"value": 42, "label": "spec"})
      assert_eq(inserted["_key"], "doc1")
      document = client.get(name, "doc1")
      assert_eq(document["value"], 42)
      assert_eq(document["label"], "spec")
    end

    test("insert with a nil key generates one") do
      name = collection("client_spec_autokey")
      client = raw_db()
      key = client.insert(name, nil, {"auto": true})["_key"]
      assert_match(key, "^[0-9a-f-]{36}$")
      assert_eq(client.get(name, key)["auto"], true)
    end

    test("get returns nil for a missing key") do
      pending("bug: Solidb#get raises DocumentNotFound instead of returning nil")
      name = collection("client_spec_get_missing")
      assert_null(raw_db().get(name, "nope"))
    end

    test("update patches the given fields of an existing document") do
      name = collection("client_spec_update")
      client = raw_db()
      client.insert(name, "u1", {"value": 1, "label": "kept"})
      client.update(name, "u1", {"value": 2})
      document = client.get(name, "u1")
      assert_eq(document["value"], 2)
      assert_eq(document["label"], "kept")
    end

    test("update raises for a missing document") do
      name = collection("client_spec_update_missing")
      assert_raises("Update failed: HTTP 404") do
        raw_db().update(name, "ghost", {"value": 1})
      end
    end

    test("upsert merges into an existing document") do
      name = collection("client_spec_upsert")
      client = raw_db()
      client.insert(name, "up1", {"a": 1, "b": 2})
      client.upsert(name, "up1", {"b": 3})
      document = client.get(name, "up1")
      assert_eq(document["a"], 1)
      assert_eq(document["b"], 3)
    end

    test("upsert inserts a missing document") do
      pending("bug: Solidb#upsert raises 'Update failed: HTTP 404' instead of inserting")
      name = collection("client_spec_upsert_new")
      client = raw_db()
      client.upsert(name, "fresh", {"n": 1})
      assert_eq(client.get(name, "fresh")["n"], 1)
    end

    test("delete removes a document") do
      name = collection("client_spec_delete")
      client = raw_db()
      client.insert(name, "d1", {"value": 7})
      assert_eq(client.delete(name, "d1"), "OK")
      assert_eq(client.query("FOR d IN client_spec_delete RETURN d"), [])
    end

    test("delete raises for a missing document") do
      name = collection("client_spec_delete_missing")
      assert_raises("Delete failed: HTTP 404") do
        raw_db().delete(name, "ghost")
      end
    end

    test("list returns the documents of a collection") do
      pending("bug: Solidb#list requests /collection/<name>/documents, which SoliDB answers 404")
      name = collection("client_spec_list")
      client = raw_db()
      client.insert(name, "l1", {"n": 1})
      client.insert(name, "l2", {"n": 2})
      assert_eq(client.list(name).map { |document| document["_key"] }.sort, ["l1", "l2"])
    end
  end

  describe("indexes") do
    test("create_index, list_indexes and drop_index") do
      name = collection("client_spec_idx")
      client = raw_db()
      assert_eq(client.create_index(name, "by_value", ["value"], {}), "Created index: by_value on client_spec_idx")
      index = client.list_indexes(name).find { |candidate| candidate["name"] == "by_value" }
      assert_eq(index["fields"], ["value"])
      assert_eq(index["unique"], false)
      assert_eq(client.drop_index(name, "by_value"), "Dropped index: by_value from client_spec_idx")
      assert_eq(client.list_indexes(name), [])
    end

    test("a unique index refuses a duplicate value") do
      name = collection("client_spec_unique")
      client = raw_db()
      client.create_index(name, "by_email", ["email"], {"unique": true})
      client.insert(name, "first", {"email": "a@example.com"})
      assert_raises("Unique constraint violated") do
        client.insert(name, "second", {"email": "a@example.com"})
      end
    end

    test("create_vector_index and drop_vector_index") do
      name = collection("client_spec_vecidx")
      client = raw_db()
      assert_eq(
        client.create_vector_index(name, "by_embedding", "embedding", 3, "cosine"),
        "Created vector index: by_embedding on client_spec_vecidx"
      )
      assert_eq(
        client.drop_vector_index(name, "by_embedding"),
        "Dropped vector index: by_embedding from client_spec_vecidx"
      )
    end

    test("index options are optional, as documented") do
      pending("bug: create_index needs 4 arguments and create_vector_index 5 — the options hash is not optional")
      name = collection("client_spec_idx_noopts")
      client = raw_db()
      assert_eq(client.create_index(name, "by_value", ["value"]), "Created index: by_value on client_spec_idx_noopts")
      assert_eq(
        client.create_vector_index(name, "by_embedding", "embedding", 3),
        "Created vector index: by_embedding on client_spec_idx_noopts"
      )
    end
  end

  describe("columnar stores") do
    after_each() do
      raw_db().drop_columnar("client_spec_col") rescue nil
    end

    test("create_columnar, list_columnar and drop_columnar") do
      client = raw_db()
      created = client.create_columnar("client_spec_col", [
        {"name": "id", "type": "Int"},
        {"name": "label", "type": "String", "nullable": true}
      ])
      assert_eq(created, {"columns": 2, "name": "client_spec_col", "status": "created"})
      store = client.list_columnar.find { |candidate| candidate["name"] == "client_spec_col" }
      assert_eq(store["columns"].map { |column| column["name"] }, ["id", "label"])
      assert_eq(client.drop_columnar("client_spec_col"), "Dropped columnar store: client_spec_col")
    end
  end

  describe("blobs") do
    test("store_blob and get_blob round-trip base64 data") do
      name = collection("client_spec_blobs", "blob")
      client = raw_db()
      encoded = Base64.encode("hello solidb blob")
      blob_id = client.store_blob(name, encoded, "spec.txt", "text/plain")
      assert_match(blob_id, "^[0-9a-f-]{36}$")
      assert_eq(client.get_blob(name, blob_id), encoded)
    end

    test("get_blob_metadata describes the blob without its body") do
      name = collection("client_spec_blob_meta", "blob")
      client = raw_db()
      blob_id = client.store_blob(name, Base64.encode("hello solidb blob"), "spec.txt", "text/plain")
      metadata = client.get_blob_metadata(name, blob_id)
      assert_eq(metadata["filename"], "spec.txt")
      assert_eq(metadata["content_type"], "text/plain")
      assert_eq(metadata["size"], 17)
    end

    test("delete_blob removes the blob") do
      name = collection("client_spec_blob_delete", "blob")
      client = raw_db()
      blob_id = client.store_blob(name, Base64.encode("to be deleted"), "gone.bin", "application/octet-stream")
      assert_eq(client.delete_blob(name, blob_id), "OK")
      assert_raises("Blob not found") do
        client.get_blob(name, blob_id)
      end
    end

    test("store_blob refuses a collection that is not a blob collection") do
      name = collection("client_spec_not_blobs")
      assert_raises("is not a blob collection") do
        raw_db().store_blob(name, Base64.encode("x"), "x.txt", "text/plain")
      end
    end
  end
end
