# has_one_attached / has_many_attached registration, and the disk side of
# store_attachment / read_attachment / delete_attachment plus content_type_for.
# The disk service writes under ./storage/attachments; after_each removes the
# spec's collection and any directory it left empty.

ATTACHMENT_COLLECTION = "attachments_dsl_spec"
DISK_CONFIG = {"collection": ATTACHMENT_COLLECTION}
HELLO_BASE64 = "aGVsbG8="

class AttachedUser < Model
  has_one_attached("avatar")
end

class AttachedAlbum < Model
  has_many_attached("photos", {"service": "s3", "max_size": 1000000})
end

class AttachedContract < Model
  has_one_attached("scan", {"content_types": ["application/pdf"], "encrypted": true})
end

class AttachedReport < Model
  has_one_attached("summary", {"content_types": ["application/pdf"], "max_size": 5})
  has_many_attached("scans")
end

describe("has_*_attached DSL") do
  test("has_one_attached registers a single disk uploader") do
    config = model_uploader_config(AttachedUser, "avatar")
    assert_eq(config["name"], "avatar")
    assert_eq(config["multiple"], false)
    assert_eq(config["service"], "disk")
    assert_eq(config["collection"], "attached_user_avatars")
    assert_gt(config["max_size"], 0)
    assert_contains(config["content_types"], "image/png")
  end

  test("has_many_attached can target s3 with its own max_size") do
    config = model_uploader_config(AttachedAlbum, "photos")
    assert_eq(config["multiple"], true)
    assert_eq(config["service"], "s3")
    assert_eq(config["max_size"], 1000000)
  end

  test("content_types replaces the default allowlist") do
    config = model_uploader_config(AttachedReport, "summary")
    assert_eq(config["content_types"], ["application/pdf"])
    assert_eq(config["max_size"], 5)
  end

  test("the class can be named by a string") do
    assert_eq(model_uploader_config("AttachedUser", "avatar")["name"], "avatar")
  end

  test("an undeclared field has no config") do
    assert_null(model_uploader_config(AttachedUser, "nothing"))
  end

  test("encrypted is off unless asked for") do
    assert_eq(model_uploader_config(AttachedUser, "avatar")["encrypted"], false)
    assert_eq(model_uploader_config(AttachedContract, "scan")["encrypted"], true)
  end

  test("model_uploader_fields lists every declared field in order") do
    assert_eq(model_uploader_fields(AttachedReport), ["summary", "scans"])
  end
end

describe("disk attachments") do
  after_each() do
    System.run_sync(["rm", "-rf", "storage/attachments/#{ATTACHMENT_COLLECTION}"])
    # rmdir only removes a directory that is empty, so a real ./storage stays.
    System.run_sync(["rmdir", "-p", "storage/attachments"])
  end

  describe("round trip") do
    test("store returns an id that read_attachment answers with the file") do
      file = {"filename": "hi.txt", "content_type": "text/plain", "data": HELLO_BASE64}
      id = store_attachment(DISK_CONFIG, file)
      stored = read_attachment(DISK_CONFIG, id)
      assert_eq(stored, {"filename": "hi.txt", "content_type": "text/plain", "size": 5, "data": HELLO_BASE64})
    end

    test("a file without name or type is stored as an octet-stream named file") do
      id = store_attachment(DISK_CONFIG, {"data": "aGk="})
      stored = read_attachment(DISK_CONFIG, id)
      assert_eq(stored["filename"], "file")
      assert_eq(stored["content_type"], "application/octet-stream")
      assert_eq(stored["size"], 2)
    end

    test("two stores of the same file get distinct ids") do
      file = {"filename": "hi.txt", "data": HELLO_BASE64}
      assert_ne(store_attachment(DISK_CONFIG, file), store_attachment(DISK_CONFIG, file))
    end

    test("delete_attachment removes it, and a read then raises") do
      id = store_attachment(DISK_CONFIG, {"filename": "hi.txt", "data": HELLO_BASE64})
      assert_eq(delete_attachment(DISK_CONFIG, id), true)
      assert_raises("attachment not found") do
        read_attachment(DISK_CONFIG, id)
      end
    end

    test("deleting an id that was never stored still answers true") do
      assert_eq(delete_attachment(DISK_CONFIG, "never-stored"), true)
    end
  end

  describe("streaming") do
    test("the head reads the metadata alone, nil for an unknown id") do
      id = store_attachment(DISK_CONFIG, {"filename": "hi.txt", "content_type": "text/plain", "data": HELLO_BASE64})
      assert_eq(__soli_attachment_head(DISK_CONFIG, id), {"filename": "hi.txt", "content_type": "text/plain", "size": 5})
      assert_null(__soli_attachment_head(DISK_CONFIG, "never-stored"))
    end

    test("the blob response describes the download and leaves the body to the server") do
      id = store_attachment(DISK_CONFIG, {"filename": "hi.txt", "data": HELLO_BASE64})
      req = {"method": "GET", "headers": {"range": "bytes=0-1"}}
      response = __soli_attachment_blob_response(DISK_CONFIG, id, req, {"Content-Type": "text/plain"})
      assert_eq(response["status"], 200)
      assert_eq(response["body"], "")
      assert_eq(response["headers"]["Content-Type"], "text/plain")
      assert_eq(response["headers"]["ETag"], "\"#{id}\"")
      assert(response["headers"]["x-soli-blob-stream"].present?)
    end

    test("a matching If-None-Match is a 304 without a download") do
      id = store_attachment(DISK_CONFIG, {"data": HELLO_BASE64})
      req = {"method": "GET", "headers": {"if-none-match": "\"#{id}\""}}
      response = __soli_attachment_blob_response(DISK_CONFIG, id, req, {"Content-Type": "text/plain"})
      assert_eq(response["status"], 304)
    end
  end

  describe("encrypted") do
    test("an encrypted store reads back as the file it was given") do
      skip("SOLI_ENCRYPTION_KEY is not set") unless hasenv("SOLI_ENCRYPTION_KEY")
      config = {"collection": ATTACHMENT_COLLECTION, "encrypted": true}
      id = store_attachment(config, {"filename": "hi.txt", "content_type": "text/plain", "data": HELLO_BASE64})
      assert_eq(read_attachment(config, id), {"filename": "hi.txt", "content_type": "text/plain", "size": 5, "data": HELLO_BASE64})
    end

    test("the bytes on disk are sealed, not the file") do
      skip("SOLI_ENCRYPTION_KEY is not set") unless hasenv("SOLI_ENCRYPTION_KEY")
      id = store_attachment({"collection": ATTACHMENT_COLLECTION, "encrypted": true}, {"data": HELLO_BASE64})
      raw = File.read("storage/attachments/#{ATTACHMENT_COLLECTION}/#{id}/data", "latin1")
      assert(raw.starts_with("SBE"))
      assert(!raw.includes?("hello"))
    end

    test("a sealed blob does not open under another collection") do
      skip("SOLI_ENCRYPTION_KEY is not set") unless hasenv("SOLI_ENCRYPTION_KEY")
      id = store_attachment({"collection": ATTACHMENT_COLLECTION, "encrypted": true}, {"data": HELLO_BASE64})
      System.run_sync(["mkdir", "-p", "storage/attachments/#{ATTACHMENT_COLLECTION}_moved"])
      System.run_sync(["cp", "-r", "storage/attachments/#{ATTACHMENT_COLLECTION}/#{id}", "storage/attachments/#{ATTACHMENT_COLLECTION}_moved/#{id}"])
      assert_raises("does not open") do
        read_attachment({"collection": "#{ATTACHMENT_COLLECTION}_moved", "encrypted": true}, id)
      end
      System.run_sync(["rm", "-rf", "storage/attachments/#{ATTACHMENT_COLLECTION}_moved"])
    end

    test("a file stored before encryption was turned on still reads") do
      id = store_attachment(DISK_CONFIG, {"data": HELLO_BASE64})
      assert_eq(read_attachment({"collection": ATTACHMENT_COLLECTION, "encrypted": true}, id)["data"], HELLO_BASE64)
    end

    test("direct_upload refuses an encrypted config") do
      assert_raises("cannot encrypt") do
        direct_upload({"service": "s3", "encrypted": true}, {"filename": "a.pdf", "size": 3})
      end
    end
  end

  describe("refusals") do
    test("reading an unknown id raises") do
      assert_raises("attachment not found") do
        read_attachment(DISK_CONFIG, "never-stored")
      end
    end

    test("a traversal id never reaches outside the collection") do
      assert_raises("attachment not found") do
        read_attachment({"collection": ".."}, "..")
      end
    end

    test("the solidb service points at solidb_store_blob") do
      assert_raises("use solidb_store_blob") do
        store_attachment({"service": "solidb"}, {"data": HELLO_BASE64})
      end
    end

    test("an unknown service is named in the error") do
      assert_raises("unknown attachment service \"ftp\"") do
        store_attachment({"service": "ftp"}, {"data": HELLO_BASE64})
      end
    end

    test("a file without data raises") do
      assert_raises("file is missing data") do
        store_attachment(DISK_CONFIG, {"filename": "a.txt"})
      end
    end

    test("data that is not base64 raises") do
      assert_raises("file data is not base64") do
        store_attachment(DISK_CONFIG, {"data": "!!!"})
      end
    end

    test("the config must be a hash") do
      assert_raises("expects a config hash") do
        store_attachment("disk", {"data": HELLO_BASE64})
      end
    end

    test("reading from an unknown service answers nil") do
      assert_null(read_attachment({"service": "weird"}, "x"))
    end

    test("deleting from an unknown service answers false") do
      assert_eq(delete_attachment({"service": "weird"}, "x"), false)
    end
  end
end

describe("content_type_for") do
  test("maps known extensions, case-insensitively") do
    assert_eq(content_type_for("a/b/c.png"), "image/png")
    assert_eq(content_type_for("report.PDF"), "application/pdf")
    assert_eq(content_type_for("data.json"), "application/json")
    assert_eq(content_type_for("page.html"), "text/html")
  end

  test("an unknown or missing extension is an octet-stream") do
    assert_eq(content_type_for("mystery.zzz"), "application/octet-stream")
    assert_eq(content_type_for("noextension"), "application/octet-stream")
  end

  test("a non-string name raises") do
    assert_raises("expects a string name") do
      content_type_for(42)
    end
  end
end
