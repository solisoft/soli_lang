// ============================================================================
// File Functions Test Suite
// ============================================================================
// Tests for file I/O functions
// ============================================================================

describe("File Write Functions", fn() {
    test("barf() writes text file", fn() {
        let path = "/tmp/soli_test_output.txt";
        let content = "Hello, World!";

        barf(path, content);  // barf returns nil, just check it doesn't throw
        let read_back = slurp(path);
        assert_eq(read_back, content);
    });

    test("barf() overwrites existing file", fn() {
        let path = "/tmp/soli_test_overwrite.txt";
        barf(path, "original");
        barf(path, "updated");

        let content = slurp(path);
        assert_eq(content, "updated");
    });

    test("barf() fails when directory does not exist", fn() {
        let path = "/tmp/soli_test_subdir_nonexistent/nested/file.txt";
        let content = "Nested file";

        let error_caught = false;
        try {
            barf(path, content);
        } catch (e) {
            error_caught = true;
        }
        assert(error_caught);
    });
});

describe("File Read Functions", fn() {
    test("slurp() reads text file", fn() {
        let path = "/tmp/soli_test_read.txt";
        barf(path, "Test content");

        let content = slurp(path);
        assert_eq(content, "Test content");
    });

    test("slurp() returns empty string for empty file", fn() {
        let path = "/tmp/soli_test_empty.txt";
        barf(path, "");

        let content = slurp(path);
        assert_eq(content, "");
    });

    test("slurp() reads JSON file", fn() {
        let path = "/tmp/soli_test_json.json";
        let json = "{\"name\": \"test\", \"value\": 42}";
        barf(path, json);

        let content = slurp(path);
        assert_contains(content, "name");
    });

    test("slurp() throws error for nonexistent file", fn() {
        let path = "/tmp/soli_nonexistent_file_12345.txt";
        let error_caught = false;
        try {
            let content = slurp(path);
        } catch (e) {
            error_caught = true;
        }
        assert(error_caught);
    });
});

describe("File Existence", fn() {
    test("is_file() returns true for file", fn() {
        let path = "/tmp/soli_test_is_file.txt";
        barf(path, "test");

        let result = File.is_file(path);
        assert(result);
    });

    test("is_file() returns false for directory", fn() {
        let result = File.is_file("/tmp");
        assert_not(result);
    });

    test("is_file() returns false for nonexistent", fn() {
        let result = File.is_file("/tmp/soli_nonexistent_12345.txt");
        assert_not(result);
    });

    test("is_dir() returns true for directory", fn() {
        let result = File.is_dir("/tmp");
        assert(result);
    });

    test("is_dir() returns false for file", fn() {
        let path = "/tmp/soli_test_is_dir.txt";
        barf(path, "test");

        let result = File.is_dir(path);
        assert_not(result);
    });

    test("exists() returns true for existing file", fn() {
        let path = "/tmp/soli_test_exists.txt";
        barf(path, "test");

        let result = File.exists(path);
        assert(result);
    });

    test("exists() returns false for nonexistent", fn() {
        let result = File.exists("/tmp/soli_nonexistent_12345.txt");
        assert_not(result);
    });
});
