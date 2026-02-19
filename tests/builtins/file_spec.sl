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

describe("File Class Methods", fn() {
    test("File.read() reads file contents", fn() {
        let path = "/tmp/soli_test_file_read.txt";
        barf(path, "file read test");
        let content = File.read(path);
        assert_eq(content, "file read test");
    });

    test("File.write() writes file contents", fn() {
        let path = "/tmp/soli_test_file_write.txt";
        File.write(path, "file write test");
        let content = slurp(path);
        assert_eq(content, "file write test");
    });

    test("File.delete() removes a file", fn() {
        let path = "/tmp/soli_test_file_delete.txt";
        barf(path, "delete me");
        assert(File.exists(path));
        File.delete(path);
        assert_not(File.exists(path));
    });

    test("File.size() returns file size in bytes", fn() {
        let path = "/tmp/soli_test_file_size.txt";
        barf(path, "hello");
        let size = File.size(path);
        assert_eq(size, 5);
    });

    test("File.append() appends content to file", fn() {
        let path = "/tmp/soli_test_file_append.txt";
        barf(path, "hello");
        File.append(path, " world");
        let content = slurp(path);
        assert_eq(content, "hello world");
    });

    test("File.lines() reads file as array of lines", fn() {
        let path = "/tmp/soli_test_file_lines.txt";
        barf(path, "line1\nline2\nline3");
        let lines = File.lines(path);
        assert_eq(len(lines), 3);
        assert_eq(lines[0], "line1");
        assert_eq(lines[2], "line3");
    });

    test("File.copy() copies a file", fn() {
        let src = "/tmp/soli_test_file_copy_src.txt";
        let dest = "/tmp/soli_test_file_copy_dest.txt";
        barf(src, "copy me");
        File.copy(src, dest);
        assert(File.exists(dest));
        assert_eq(slurp(dest), "copy me");
    });

    test("File.rename() renames a file", fn() {
        let old_path = "/tmp/soli_test_file_rename_old.txt";
        let new_path = "/tmp/soli_test_file_rename_new.txt";
        barf(old_path, "rename me");
        File.rename(old_path, new_path);
        assert_not(File.exists(old_path));
        assert(File.exists(new_path));
        assert_eq(slurp(new_path), "rename me");
    });
});

describe("slurp_json", fn() {
    test("slurp_json() reads and parses JSON file", fn() {
        let path = "/tmp/soli_test_slurp_json.json";
        barf(path, "{\"name\": \"test\", \"value\": 42}");
        let data = slurp_json(path);
        assert_eq(data["name"], "test");
        assert_eq(data["value"], 42);
    });
});
