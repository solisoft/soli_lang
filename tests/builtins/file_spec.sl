# File I/O: the standalone slurp/barf/mkdir_p/slurp_json helpers and the File
# class. Every test works inside SPEC_DIR, created fresh and removed after it.

SPEC_DIR = "/tmp/soli_file_spec"

def spec_path(name)
  "#{SPEC_DIR}/#{name}"
end

describe("File I/O") do
  before_each() do
    System.run_sync(["rm", "-rf", SPEC_DIR])
    mkdir_p(SPEC_DIR)
  end

  after_each() do
    System.run_sync(["rm", "-rf", SPEC_DIR])
  end

  describe("barf") do
    test("writes a text file and returns nil") do
      path = spec_path("hello.txt")
      assert_null(barf(path, "Hello, World!"))
      assert_eq(slurp(path), "Hello, World!")
    end

    test("overwrites an existing file") do
      path = spec_path("overwrite.txt")
      barf(path, "original")
      barf(path, "updated")
      assert_eq(slurp(path), "updated")
    end

    test("writes a byte array as raw bytes") do
      path = spec_path("bytes.dat")
      barf(path, [104, 105])
      assert_eq(slurp(path), "hi")
      assert_eq(slurp(path, "binary"), [104, 105])
    end

    test("rejects a byte outside 0-255") do
      assert_raises("byte value 300 out of range") do
        barf(spec_path("bad.dat"), [300])
      end
    end

    test("fails when the parent directory does not exist") do
      assert_raises("No such file or directory") do
        barf(spec_path("missing/nested/file.txt"), "Nested file")
      end
    end
  end

  describe("mkdir_p") do
    test("creates every missing parent and returns true") do
      nested = spec_path("a/b/c")
      assert_eq(mkdir_p(nested), true)
      assert(File.is_dir(spec_path("a")))
      assert(File.is_dir(nested))
    end

    test("succeeds on a directory that already exists") do
      mkdir_p(spec_path("again"))
      assert_eq(mkdir_p(spec_path("again")), true)
    end

    test("lets barf write into the directory it made") do
      mkdir_p(spec_path("deep/dir"))
      barf(spec_path("deep/dir/note.txt"), "inside")
      assert_eq(slurp(spec_path("deep/dir/note.txt")), "inside")
    end
  end

  describe("slurp") do
    test("reads a text file") do
      barf(spec_path("read.txt"), "Test content")
      assert_eq(slurp(spec_path("read.txt")), "Test content")
    end

    test("returns an empty string for an empty file") do
      barf(spec_path("empty.txt"), "")
      assert_eq(slurp(spec_path("empty.txt")), "")
    end

    test("reads raw bytes in binary mode") do
      barf(spec_path("abc.txt"), "abc")
      assert_eq(slurp(spec_path("abc.txt"), "binary"), [97, 98, 99])
    end

    test("rejects an unknown mode") do
      barf(spec_path("mode.txt"), "x")
      assert_raises("unknown mode/encoding 'bogus'") do
        slurp(spec_path("mode.txt"), "bogus")
      end
    end

    test("raises for a missing file, naming it") do
      message = assert_raises("slurp failed to read") do
        slurp(spec_path("nonexistent.txt"))
      end
      assert_contains(message, "nonexistent.txt")
    end
  end

  describe("slurp_json") do
    test("reads and parses a JSON file") do
      barf(spec_path("data.json"), "{\"name\": \"test\", \"value\": 42}")
      assert_eq(slurp_json(spec_path("data.json")), {"name": "test", "value": 42})
    end

    test("raises on invalid JSON") do
      barf(spec_path("bad.json"), "{nope")
      assert_raises("slurp_json failed to parse") do
        slurp_json(spec_path("bad.json"))
      end
    end
  end

  describe("existence checks") do
    test("is_file is true for a file") do
      barf(spec_path("file.txt"), "test")
      assert(File.is_file(spec_path("file.txt")))
    end

    test("is_file is false for a directory") do
      assert_not(File.is_file(SPEC_DIR))
    end

    test("is_file is false for a missing path") do
      assert_not(File.is_file(spec_path("nonexistent.txt")))
    end

    test("is_dir is true for a directory") do
      # A directory this suite creates, not the platform's `/tmp`: on macOS
      # `/tmp` is a symlink to `private/tmp`, and `File` deliberately does not
      # follow symlinks (the jail policy).
      assert(File.is_dir(SPEC_DIR))
    end

    test("is_dir follows a symlinked path only through Trusted") do
      # `/tmp` is a real directory on Linux and a symlink on macOS; `Trusted`
      # follows symlinks, so it answers true either way.
      assert(Trusted.is_dir("/tmp"))
    end

    test("is_dir is false for a file") do
      barf(spec_path("not_dir.txt"), "test")
      assert_not(File.is_dir(spec_path("not_dir.txt")))
    end

    test("exists is true for a file and for a directory") do
      barf(spec_path("exists.txt"), "test")
      assert(File.exists(spec_path("exists.txt")))
      assert(File.exists(SPEC_DIR))
    end

    test("exists is false for a missing path") do
      assert_not(File.exists(spec_path("nonexistent.txt")))
    end

    test("file_exists mirrors File.exists") do
      barf(spec_path("standalone.txt"), "x")
      assert(file_exists(spec_path("standalone.txt")))
      assert_not(file_exists(spec_path("nonexistent.txt")))
    end
  end

  describe("File class") do
    test("read returns the file contents") do
      barf(spec_path("file_read.txt"), "file read test")
      assert_eq(File.read(spec_path("file_read.txt")), "file read test")
    end

    test("read raises for a missing file") do
      assert_raises("File.read() failed") do
        File.read(spec_path("nonexistent.txt"))
      end
    end

    test("write writes the contents and returns true") do
      assert_eq(File.write(spec_path("file_write.txt"), "file write test"), true)
      assert_eq(slurp(spec_path("file_write.txt")), "file write test")
    end

    test("write stringifies a non-string value") do
      File.write(spec_path("number.txt"), 42)
      assert_eq(slurp(spec_path("number.txt")), "42")
    end

    test("delete removes a file") do
      path = spec_path("delete.txt")
      barf(path, "delete me")
      File.delete(path)
      assert_not(File.exists(path))
    end

    test("delete raises for a missing file") do
      assert_raises("File.delete() failed") do
        File.delete(spec_path("nonexistent.txt"))
      end
    end

    test("size returns the size in bytes") do
      barf(spec_path("size.txt"), "hello")
      assert_eq(File.size(spec_path("size.txt")), 5)
      barf(spec_path("size_empty.txt"), "")
      assert_eq(File.size(spec_path("size_empty.txt")), 0)
    end

    test("append adds to an existing file") do
      path = spec_path("append.txt")
      barf(path, "hello")
      File.append(path, " world")
      assert_eq(slurp(path), "hello world")
    end

    test("append creates a missing file") do
      path = spec_path("append_new.txt")
      File.append(path, "first")
      assert_eq(slurp(path), "first")
    end

    test("lines splits the file on newlines") do
      barf(spec_path("lines.txt"), "line1\nline2\nline3")
      assert_eq(File.lines(spec_path("lines.txt")), ["line1", "line2", "line3"])
    end

    test("lines of an empty file is an empty array") do
      barf(spec_path("no_lines.txt"), "")
      assert_eq(File.lines(spec_path("no_lines.txt")), [])
    end

    test("copy duplicates a file and keeps the source") do
      source = spec_path("copy_src.txt")
      destination = spec_path("copy_dest.txt")
      barf(source, "copy me")
      File.copy(source, destination)
      assert_eq(slurp(destination), "copy me")
      assert(File.exists(source))
    end

    test("rename moves a file") do
      old_path = spec_path("rename_old.txt")
      new_path = spec_path("rename_new.txt")
      barf(old_path, "rename me")
      File.rename(old_path, new_path)
      assert_not(File.exists(old_path))
      assert_eq(slurp(new_path), "rename me")
    end

    test("glob lists the matching paths") do
      barf(spec_path("one.txt"), "1")
      barf(spec_path("two.txt"), "2")
      barf(spec_path("three.log"), "3")
      assert_eq(File.glob(spec_path("*.txt")).sort, [spec_path("one.txt"), spec_path("two.txt")])
    end
  end
end
