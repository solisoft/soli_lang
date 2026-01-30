// ============================================================================
// File Upload Functions Test Suite
// ============================================================================
// Tests for multipart parsing and file upload functions
// ============================================================================

describe("SoliDB Address Configuration", fn() {
    test("set_solidb_address() configures global address", fn() {
        let result = set_solidb_address("http://localhost:8529");
        assert_null(result);
    });

    test("set_solidb_address() can be called multiple times", fn() {
        set_solidb_address("http://localhost:8529");
        let result = set_solidb_address("http://localhost:9999");
        assert_null(result);
    });
});

describe("Parse Multipart Basic", fn() {
    test("parse_multipart() returns empty array for non-multipart", fn() {
        let req = hash();
        req["body"] = "regular body";
        req["headers"] = hash();
        req["headers"]["content-type"] = "application/json";
        let result = parse_multipart(req);
        assert_eq(len(result), 0);
    });

    test("parse_multipart() returns empty for missing body", fn() {
        let req = hash();
        req["headers"] = hash();
        req["headers"]["content-type"] = "multipart/form-data; boundary=abc";
        let result = parse_multipart(req);
        assert_eq(len(result), 0);
    });

    test("parse_multipart() returns empty for empty body", fn() {
        let req = hash();
        req["body"] = "";
        req["headers"] = hash();
        req["headers"]["content-type"] = "multipart/form-data; boundary=abc";
        let result = parse_multipart(req);
        assert_eq(len(result), 0);
    });

    test("parse_multipart() handles invalid boundary gracefully", fn() {
        let req = hash();
        req["body"] = "--invalid_boundary\r\n\r\ndata\r\n--";
        req["headers"] = hash();
        req["headers"]["content-type"] = "multipart/form-data; boundary=";
        let result = parse_multipart(req);
        assert_eq(len(result), 0);
    });
});

describe("Parse Multipart File Extraction", fn() {
    test("parse_multipart() extracts file from multipart data", fn() {
        let body = "--boundary\r\nContent-Disposition: form-data; name=\"file\"; filename=\"test.txt\"\r\nContent-Type: text/plain\r\n\r\nHello World\r\n--boundary--";
        let req = hash();
        req["body"] = body;
        req["headers"] = hash();
        req["headers"]["content-type"] = "multipart/form-data; boundary=boundary";
        let result = parse_multipart(req);
        assert_eq(len(result), 1);
        assert_eq(result[0]["filename"], "test.txt");
        assert_eq(result[0]["content_type"], "text/plain");
        assert_eq(result[0]["field_name"], "file");
        assert_not_null(result[0]["data_base64"]);
        assert_not_null(result[0]["size"]);
    });

    test("parse_multipart() extracts multiple files", fn() {
        let body = "--boundary\r\nContent-Disposition: form-data; name=\"file1\"; filename=\"a.txt\"\r\nContent-Type: text/plain\r\n\r\nContent A\r\n--boundary\r\nContent-Disposition: form-data; name=\"file2\"; filename=\"b.txt\"\r\nContent-Type: text/plain\r\n\r\nContent B\r\n--boundary--";
        let req = hash();
        req["body"] = body;
        req["headers"] = hash();
        req["headers"]["content-type"] = "multipart/form-data; boundary=boundary";
        let result = parse_multipart(req);
        assert_eq(len(result), 2);
        assert_eq(result[0]["filename"], "a.txt");
        assert_eq(result[1]["filename"], "b.txt");
    });

    test("parse_multipart() handles case-insensitive content-type", fn() {
        let body = "--boundary\r\nContent-Disposition: form-data; name=\"doc\"; filename=\"doc.pdf\"\r\nContent-Type: application/pdf\r\n\r\nPDF DATA\r\n--boundary--";
        let req = hash();
        req["body"] = body;
        req["headers"] = hash();
        req["headers"]["Content-Type"] = "multipart/form-data; boundary=boundary";
        let result = parse_multipart(req);
        assert_eq(len(result), 1);
        assert_eq(result[0]["filename"], "doc.pdf");
    });

    test("parse_multipart() decodes base64 data correctly", fn() {
        let body = "--boundary\r\nContent-Disposition: form-data; name=\"file\"; filename=\"data.txt\"\r\nContent-Type: text/plain\r\n\r\nSGVsbG8gV29ybGQh\r\n--boundary--";
        let req = hash();
        req["body"] = body;
        req["headers"] = hash();
        req["headers"]["content-type"] = "multipart/form-data; boundary=boundary";
        let result = parse_multipart(req);
        assert_eq(len(result), 1);
        assert_eq(result[0]["filename"], "data.txt");
    });
});

describe("Parse Multipart Non-File Fields", fn() {
    test("parse_multipart() ignores non-file fields", fn() {
        let body = "--boundary\r\nContent-Disposition: form-data; name=\"text_field\"\r\n\r\ntext value\r\n--boundary\r\nContent-Disposition: form-data; name=\"file\"; filename=\"test.txt\"\r\nContent-Type: text/plain\r\n\r\nfile content\r\n--boundary--";
        let req = hash();
        req["body"] = body;
        req["headers"] = hash();
        req["headers"]["content-type"] = "multipart/form-data; boundary=boundary";
        let result = parse_multipart(req);
        assert_eq(len(result), 1);
        assert_eq(result[0]["field_name"], "file");
    });

    test("parse_multipart() handles mixed fields", fn() {
        let body = "--boundary\r\nContent-Disposition: form-data; name=\"name\"\r\n\r\nJohn\r\n--boundary\r\nContent-Disposition: form-data; name=\"avatar\"; filename=\"photo.jpg\"\r\nContent-Type: image/jpeg\r\n\r\nIMAGEDATA\r\n--boundary\r\nContent-Disposition: form-data; name=\"description\"\r\n\r\nMy description\r\n--boundary--";
        let req = hash();
        req["body"] = body;
        req["headers"] = hash();
        req["headers"]["content-type"] = "multipart/form-data; boundary=boundary";
        let result = parse_multipart(req);
        assert_eq(len(result), 1);
        assert_eq(result[0]["filename"], "photo.jpg");
        assert_eq(result[0]["field_name"], "avatar");
    });
});

describe("Parse Multipart Edge Cases", fn() {
    test("parse_multipart() handles empty filename", fn() {
        let body = "--boundary\r\nContent-Disposition: form-data; name=\"file\"; filename=\"\"\r\nContent-Type: text/plain\r\n\r\n\r\n--boundary--";
        let req = hash();
        req["body"] = body;
        req["headers"] = hash();
        req["headers"]["content-type"] = "multipart/form-data; boundary=boundary";
        let result = parse_multipart(req);
        assert_eq(len(result), 0);
    });

    test("parse_multipart() handles special characters in filename", fn() {
        let body = "--boundary\r\nContent-Disposition: form-data; name=\"file\"; filename=\"my document (1).txt\"\r\nContent-Type: text/plain\r\n\r\nContent\r\n--boundary--";
        let req = hash();
        req["body"] = body;
        req["headers"] = hash();
        req["headers"]["content-type"] = "multipart/form-data; boundary=boundary";
        let result = parse_multipart(req);
        assert_eq(len(result), 1);
        assert_contains(result[0]["filename"], "my document");
    });

    test("parse_multipart() handles different content types", fn() {
        let body = "--boundary\r\nContent-Disposition: form-data; name=\"image\"; filename=\"photo.png\"\r\nContent-Type: image/png\r\n\r\nPNG DATA\r\n--boundary--";
        let req = hash();
        req["body"] = body;
        req["headers"] = hash();
        req["headers"]["content-type"] = "multipart/form-data; boundary=boundary";
        let result = parse_multipart(req);
        assert_eq(len(result), 1);
        assert_eq(result[0]["content_type"], "image/png");
    });

    test("parse_multipart() handles binary data", fn() {
        let body = "--boundary\r\nContent-Disposition: form-data; name=\"file\"; filename=\"binary.bin\"\r\nContent-Type: application/octet-stream\r\n\r\ntest\r\n--boundary--";
        let req = hash();
        req["body"] = body;
        req["headers"] = hash();
        req["headers"]["content-type"] = "multipart/form-data; boundary=boundary";
        let result = parse_multipart(req);
        assert_eq(len(result), 1);
        assert_eq(result[0]["filename"], "binary.bin");
    });

    test("parse_multipart() calculates correct size", fn() {
        let body = "--boundary\r\nContent-Disposition: form-data; name=\"file\"; filename=\"test.txt\"\r\nContent-Type: text/plain\r\n\r\nHello World!\r\n--boundary--";
        let req = hash();
        req["body"] = body;
        req["headers"] = hash();
        req["headers"]["content-type"] = "multipart/form-data; boundary=boundary";
        let result = parse_multipart(req);
        assert_eq(len(result), 1);
        assert(result[0]["size"] > 0);
    });
});

describe("Get Blob URL", fn() {
    test("get_blob_url() generates correct URL with explicit base_url", fn() {
        let url = get_blob_url("images", "abc123", "http://localhost:8529");
        assert_contains(url, "images");
        assert_contains(url, "abc123");
        assert_contains(url, "http://localhost:8529");
    });

    test("get_blob_url() uses global address when base_url not provided", fn() {
        set_solidb_address("http://localhost:8529");
        let url = get_blob_url("files", "xyz789");
        assert_contains(url, "files");
        assert_contains(url, "xyz789");
        assert_contains(url, "http://localhost:8529");
    });

    test("get_blob_url() handles trailing slash in base_url", fn() {
        let url = get_blob_url("files", "xyz789", "http://localhost:8529/");
        assert_contains(url, "files");
        assert_contains(url, "xyz789");
    });

    test("get_blob_url() with custom expiration", fn() {
        let url = get_blob_url("docs", "doc001", "http://localhost:8529", 7200);
        assert_not_null(url);
    });

    test("get_blob_url() generates correct URL format", fn() {
        let url = get_blob_url("mycollection", "myblobid", "http://localhost:8529");
        assert_eq(url, "http://localhost:8529/_api/database/solidb/document/mycollection/myblobid");
    });
});

describe("Upload Error Handling", fn() {
    test("upload_to_solidb() returns error for missing field", fn() {
        set_solidb_address("http://localhost:8529");
        let req = hash();
        req["body"] = "--boundary\r\n--boundary--";
        req["headers"] = hash();
        req["headers"]["content-type"] = "multipart/form-data; boundary=boundary";
        let result = upload_to_solidb(req, "files", "missing_field");
        assert_not(result);
    });

    test("upload_all_to_solidb() returns empty array for no files", fn() {
        set_solidb_address("http://localhost:8529");
        let req = hash();
        req["body"] = "--boundary\r\nContent-Disposition: form-data; name=\"text_field\"\r\n\r\nvalue\r\n--boundary--";
        req["headers"] = hash();
        req["headers"]["content-type"] = "multipart/form-data; boundary=boundary";
        let result = upload_all_to_solidb(req, "files");
        assert_eq(len(result), 0);
    });

    test("upload_to_solidb() fails without configured address", fn() {
        let req = hash();
        req["body"] = "--boundary\r\n--boundary--";
        req["headers"] = hash();
        req["headers"]["content-type"] = "multipart/form-data; boundary=boundary";
        let result = upload_to_solidb(req, "files", "avatar");
        assert_not(result);
    });

    test("upload_all_to_solidb() fails without configured address", fn() {
        let req = hash();
        req["body"] = "--boundary\r\n--boundary--";
        req["headers"] = hash();
        req["headers"]["content-type"] = "multipart/form-data; boundary=boundary";
        let result = upload_all_to_solidb(req, "files");
        assert_not(result);
    });

    test("get_blob_url() fails without configured address", fn() {
        let result = get_blob_url("files", "blob123");
        assert_null(result);
    });
});

describe("Upload Integration Scenarios", fn() {
    test("upload flow: parse then upload specific file", fn() {
        let body = "--boundary\r\nContent-Disposition: form-data; name=\"avatar\"; filename=\"photo.jpg\"\r\nContent-Type: image/jpeg\r\n\r\nIMAGEDATA\r\n--boundary\r\nContent-Disposition: form-data; name=\"document\"; filename=\"resume.pdf\"\r\nContent-Type: application/pdf\r\n\r\nPDFDATA\r\n--boundary--";
        let req = hash();
        req["body"] = body;
        req["headers"] = hash();
        req["headers"]["content-type"] = "multipart/form-data; boundary=boundary";

        let files = parse_multipart(req);
        assert_eq(len(files), 2);
        assert_eq(files[0]["field_name"], "avatar");
        assert_eq(files[1]["field_name"], "document");
    });

    test("upload_all_to_solidb() processes multiple files", fn() {
        set_solidb_address("http://localhost:8529");
        let body = "--boundary\r\nContent-Disposition: form-data; name=\"file1\"; filename=\"a.txt\"\r\nContent-Type: text/plain\r\n\r\nContent A\r\n--boundary\r\nContent-Disposition: form-data; name=\"file2\"; filename=\"b.txt\"\r\nContent-Type: text/plain\r\n\r\nContent B\r\n--boundary--";
        let req = hash();
        req["body"] = body;
        req["headers"] = hash();
        req["headers"]["content-type"] = "multipart/form-data; boundary=boundary";

        let results = upload_all_to_solidb(req, "test_collection");
        assert_eq(len(results), 2);
    });
});
