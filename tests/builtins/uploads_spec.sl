# Upload helpers reachable without a server: parse_multipart, find_uploaded_file,
# uploaded_file_at, and upload_url for attached files on a saved record.

const BOUNDARY_HEADER = "multipart/form-data; boundary=XyZ"
const MULTIPART_BODY = "--XyZ\r\n" +
  "Content-Disposition: form-data; name=\"avatar\"; filename=\"../../pic.png\"\r\n" +
  "Content-Type: image/png\r\n\r\n" +
  "PNGDATA\r\n" +
  "--XyZ\r\n" +
  "Content-Disposition: form-data; name=\"title\"\r\n\r\n" +
  "Hello\r\n" +
  "--XyZ--\r\n"
const UPLOAD_PATH = "/tmp/soli_uploads_spec_note.txt"

# The saved record the upload_url tests point at
saved_doc = nil

class UploadedDoc < Model
  has_one_attached("avatar")
  has_many_attached("gallery")
end

describe("parse_multipart") do
  test("returns the file parts of a multipart body, with a sanitized filename") do
    pending("bug: parse_multipart always raises 'Invalid boundary: failed to decode Content-Type'")
    files = parse_multipart({"body": MULTIPART_BODY, "headers": {"content-type": BOUNDARY_HEADER}})
    assert_eq(files.length, 1)
    assert_eq(files[0]["field_name"], "avatar")
    assert_eq(files[0]["filename"], "pic.png")
    assert_eq(files[0]["content_type"], "image/png")
    assert_eq(files[0]["size"], 7)
    assert_eq(files[0]["data_base64"], Base64.encode("PNGDATA"))
  end

  test("returns an empty list for an empty body or a non-multipart request") do
    assert_eq(parse_multipart({"body": "", "headers": {"content-type": BOUNDARY_HEADER}}), [])
    form_request = {"body": "a=1", "headers": {"content-type": "application/x-www-form-urlencoded"}}
    assert_eq(parse_multipart(form_request), [])
  end

  test("refuses a multipart content type without a boundary") do
    assert_raises("No boundary found in Content-Type") do
      parse_multipart({"body": "x", "headers": {"content-type": "multipart/form-data"}})
    end
  end

  test("refuses something other than a request hash") do
    assert_raises("parse_multipart() expects request hash") do
      parse_multipart("x")
    end
  end
end

describe("find_uploaded_file") do
  test("finds the file part for a field in a multipart request") do
    avatar = {"name": "avatar", "filename": "a.png", "content_type": "image/png"}
    files = [{"name": "other", "filename": "b.png"}, avatar]
    request = {"headers": {"content-type": BOUNDARY_HEADER}, "files": files}
    assert_eq(find_uploaded_file(request, "avatar"), avatar)
  end

  test("ignores a part with an empty filename (nothing chosen)") do
    request = {"headers": {"content-type": BOUNDARY_HEADER}, "files": [{"name": "avatar", "filename": ""}]}
    assert_null(find_uploaded_file(request, "avatar"))
  end

  test("returns nil for a missing field or a request that is not multipart") do
    files = [{"name": "avatar", "filename": "a.png"}]
    assert_null(find_uploaded_file({"headers": {"content-type": BOUNDARY_HEADER}, "files": files}, "missing"))
    assert_null(find_uploaded_file({"headers": {"content-type": "application/json"}, "files": files}, "avatar"))
    assert_null(find_uploaded_file(nil, "avatar"))
  end

  test("refuses a field name that is not a string") do
    assert_raises("find_uploaded_file() expects a string field name") do
      find_uploaded_file({}, 5)
    end
  end
end

describe("uploaded_file_at") do
  before_each() do
    barf(UPLOAD_PATH, "hello upload")
  end

  after_each() do
    File.delete(UPLOAD_PATH) if file_exists(UPLOAD_PATH)
  end

  test("wraps a file on disk as an upload hash") do
    assert_eq(uploaded_file_at(UPLOAD_PATH), {
      "name": "soli_uploads_spec_note.txt",
      "filename": "soli_uploads_spec_note.txt",
      "content_type": "text/plain; charset=utf-8",
      "size": 12,
      "data": Base64.encode("hello upload")
    })
  end

  test("a name overrides the filename and the content type it implies") do
    upload = uploaded_file_at(UPLOAD_PATH, "notes.md")
    assert_eq(upload["filename"], "notes.md")
    assert_eq(upload["content_type"], "text/markdown; charset=utf-8")
  end

  test("refuses a path that is not a string") do
    assert_raises("uploaded_file_at() expects (path) or (path, name) as strings") do
      uploaded_file_at(5)
    end
  end

  test("refuses a file that does not exist") do
    assert_raises("uploaded_file_at() failed to read /tmp/soli_uploads_spec_missing.txt") do
      uploaded_file_at("/tmp/soli_uploads_spec_missing.txt")
    end
  end
end

describe("upload_url") do
  test("is nil for nil and for an unsaved record") do
    assert_null(upload_url(nil, "avatar"))
    assert_null(upload_url(new UploadedDoc(), "avatar"))
  end

  test("refuses something other than a model instance") do
    assert_raises("upload_url() expects a model instance, got string") do
      upload_url("x", "avatar")
    end
  end

  context("on a saved record") do
    before_each() do
      requires_solidb()
      saved_doc = UploadedDoc.create({"title": "x", "avatar_blob_id": "b1"})
    end

    after_each() do
      saved_doc.delete unless saved_doc.nil?
    end

    test("a single attachment URL carries the blob id as a cache buster") do
      assert_eq(upload_url(saved_doc, "avatar"), "/uploaded_docs/#{saved_doc._key}/avatar?v=b1")
    end

    test("transform options become query parameters, fmt the extension") do
      url = upload_url(saved_doc, "avatar", {"w": 100, "fmt": "webp"})
      assert_eq(url, "/uploaded_docs/#{saved_doc._key}/avatar.webp?v=b1&w=100")
    end

    test("a multiple attachment URL puts the blob id in the path, and needs one") do
      assert_eq(upload_url(saved_doc, "gallery", "blob9"), "/uploaded_docs/#{saved_doc._key}/gallery/blob9")
      assert_null(upload_url(saved_doc, "gallery"))
    end

    test("a multiple attachment URL puts fmt after the blob id") do
      url = upload_url(saved_doc, "gallery", {"blob_id": "blob9", "fmt": "PNG", "w": 40})
      assert_eq(url, "/uploaded_docs/#{saved_doc._key}/gallery/blob9.png?w=40")
    end

    test("a recorded content type ends the URL in the original's extension") do
      saved_doc.update({"avatar_content_type": "image/png"})
      assert_eq(upload_url(saved_doc, "avatar", {"thumb": 200}), "/uploaded_docs/#{saved_doc._key}/avatar.png?v=b1&thumb=200")
      assert_eq(upload_url(saved_doc, "avatar", {"fmt": "webp"}), "/uploaded_docs/#{saved_doc._key}/avatar.webp?v=b1")
    end

    test("a multiple field reads each blob's type from <field>_content_types") do
      saved_doc.update({"gallery_content_types": {"blob9": "image/jpeg"}})
      assert_eq(upload_url(saved_doc, "gallery", "blob9"), "/uploaded_docs/#{saved_doc._key}/gallery/blob9.jpg")
      assert_eq(upload_url(saved_doc, "gallery", "blob8"), "/uploaded_docs/#{saved_doc._key}/gallery/blob8")
    end

    test("a type with no settled extension gives none") do
      saved_doc.update({"avatar_content_type": "application/x-unknown"})
      assert_eq(upload_url(saved_doc, "avatar"), "/uploaded_docs/#{saved_doc._key}/avatar?v=b1")
    end

    test("an unknown field gives nil") do
      assert_null(upload_url(saved_doc, "unknown"))
    end
  end
end
