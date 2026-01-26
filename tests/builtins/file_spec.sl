// ============================================================================
// File Functions Test Suite
// ============================================================================
// Tests for file I/O functions
// ============================================================================

describe("File Write Functions", fn() {
    test("barf() writes text file", fn() {
        let path = "/tmp/soli_test_output.txt";
        let content = "Hello, World!";

        let result = barf(path, content);
        assert(result);
    });

    test("barf() overwrites existing file", fn() {
        let path = "/tmp/soli_test_overwrite.txt";
        barf(path, "original");
        barf(path, "updated");

        let content = slurp(path);
        assert_eq(content, "updated");
    });

    test("barf() with binary data", fn() {
        let path = "/tmp/soli_test_binary.bin";
        let binary_data = [0, 1, 2, 255, 254, 253];

        let result = barf(path, binary_data);
        assert(result);
    });

    test("barf() creates directories if needed", fn() {
        let path = "/tmp/soli_test_subdir/nested/file.txt";
        let content = "Nested file";

        let result = barf(path, content);
        assert(result);
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

    test("slurp() returns null for nonexistent file", fn() {
        let path = "/tmp/soli_nonexistent_file_12345.txt";
        let content = slurp(path);
        assert_null(content);
    });
});

describe("File Existence", fn() {
    test("is_file() returns true for file", fn() {
        let path = "/tmp/soli_test_is_file.txt";
        barf(path, "test");

        let result = is_file(path);
        assert(result);
    });

    test("is_file() returns false for directory", fn() {
        let result = is_file("/tmp");
        assert_not(result);
    });

    test("is_file() returns false for nonexistent", fn() {
        let result = is_file("/tmp/nonexistent_12345.txt");
        assert_not(result);
    });

    test("is_dir() returns true for directory", fn() {
        let result = is_dir("/tmp");
        assert(result);
    });

    test("is_dir() returns false for file", fn() {
        let path = "/tmp/soli_test_is_dir.txt";
        barf(path, "test");

        let result = is_dir(path);
        assert_not(result);
    });
});

describe("File Properties", fn() {
    test("file_size() returns file size", fn() {
        let path = "/tmp/soli_test_size.txt";
        barf(path, "Hello");

        let size = file_size(path);
        assert_eq(size, 5);
    });

    test("file_size() returns 0 for empty file", fn() {
        let path = "/tmp/soli_test_empty_size.txt";
        barf(path, "");

        let size = file_size(path);
        assert_eq(size, 0);
    });

    test("file_mtime() returns modification time", fn() {
        let path = "/tmp/soli_test_mtime.txt";
        barf(path, "test");

        let mtime = file_mtime(path);
        assert(mtime > 0);
    });
});

describe("File Deletion", fn() {
    test("rm() deletes file", fn() {
        let path = "/tmp/soli_test_delete.txt";
        barf(path, "to be deleted");

        let result = rm(path);
        assert(result);
        assert_not(is_file(path));
    });

    test("rm() returns false for nonexistent file", fn() {
        let path = "/tmp/soli_nonexistent_12345.txt";
        let result = rm(path);
        assert_not(result);
    });
});

describe("Directory Functions", fn() {
    test("mkdir() creates directory", fn() {
        let path = "/tmp/soli_test_mkdir";

        let result = mkdir(path);
        assert(result);
        assert(is_dir(path));
    });

    test("mkdir() with parents", fn() {
        let path = "/tmp/soli_test_mkdir_parents/a/b/c";

        let result = mkdir(path, true);
        assert(result);
        assert(is_dir(path));
    });

    test("rmdir() removes empty directory", fn() {
        let path = "/tmp/soli_test_rmdir";
        mkdir(path);

        let result = rmdir(path);
        assert(result);
        assert_not(is_dir(path));
    });

    test("ls() lists directory contents", fn() {
        let path = "/tmp/soli_test_ls";
        mkdir(path);
        barf(path + "/file1.txt", "1");
        barf(path + "/file2.txt", "2");

        let files = ls(path);
        assert(len(files) >= 2);
    });
});

describe("File Copy/Move", fn() {
    test("cp() copies file", fn() {
        let src = "/tmp/soli_test_cp_src.txt";
        let dst = "/tmp/soli_test_cp_dst.txt";
        barf(src, "original");

        let result = cp(src, dst);
        assert(result);

        let content = slurp(dst);
        assert_eq(content, "original");
    });

    test("mv() moves file", fn() {
        let src = "/tmp/soli_test_mv_src.txt";
        let dst = "/tmp/soli_test_mv_dst.txt";
        barf(src, "moved content");

        let result = mv(src, dst);
        assert(result);
        assert_not(is_file(src));
        assert(is_file(dst));
    });
});

describe("File Path Functions", fn() {
    test("basename() returns file name", fn() {
        let result = basename("/path/to/file.txt");
        assert_eq(result, "file.txt");
    });

    test("dirname() returns directory", fn() {
        let result = dirname("/path/to/file.txt");
        assert_eq(result, "/path/to");
    });

    test("extname() returns extension", fn() {
        let result = extname("/path/to/file.txt");
        assert_eq(result, ".txt");
    });

    test("join_path() joins paths", fn() {
        let result = join_path("/path", "to", "file.txt");
        assert_contains(result, "file.txt");
    });
});

describe("File Permissions", fn() {
    test("chmod() changes permissions", fn() {
        let path = "/tmp/soli_test_chmod.txt";
        barf(path, "test");

        let result = chmod(path, 493);
        assert(result);
    });
});
