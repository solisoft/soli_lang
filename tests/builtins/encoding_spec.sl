# Encoding.decode / Encoding.encode and the charset argument of slurp() and
# File.read() — importing and exporting Latin-1 files.

# café in Latin-1: c=0x63 a=0x61 f=0x66 é=0xE9
CAFE_LATIN1 = [99, 97, 102, 233]
LATIN1_PATH = "/tmp/soli_encoding_spec_latin1.txt"

describe("Encoding.decode") do
  test("turns Latin-1 bytes into UTF-8") do
    assert_eq(Encoding.decode(CAFE_LATIN1, "latin1"), "café")
  end

  test("accepts the iso-8859-1 label") do
    assert_eq(Encoding.decode([233], "iso-8859-1"), "é")
  end

  test("decodes UTF-8 bytes") do
    assert_eq(Encoding.decode([195, 169], "utf-8"), "é")
  end

  test("replaces invalid UTF-8 with U+FFFD") do
    assert_eq(Encoding.decode([255], "utf-8"), "\u{FFFD}")
  end

  test("no bytes decode to an empty string") do
    assert_eq(Encoding.decode([], "latin1"), "")
  end

  test("raises on a value that is not a byte") do
    assert_raises("Encoding.decode(): byte value 256 out of range") do
      Encoding.decode([256], "latin1")
    end
  end

  test("raises on an unknown label") do
    assert_raises("unknown encoding: no-such-encoding") do
      Encoding.decode([65], "no-such-encoding")
    end
  end
end

describe("Encoding.encode") do
  test("turns a UTF-8 string into Latin-1 bytes") do
    assert_eq(Encoding.encode("café", "latin1"), CAFE_LATIN1)
  end

  test("windows-1252 has the euro sign") do
    assert_eq(Encoding.encode("€", "windows-1252"), [128])
  end

  test("a character the charset lacks becomes an HTML character reference") do
    assert_eq(Encoding.decode(Encoding.encode("日", "latin1"), "latin1"), "&#26085;")
  end

  test("an empty string encodes to no bytes") do
    assert_eq(Encoding.encode("", "latin1"), [])
  end

  test("raises on an unknown label") do
    assert_raises("unknown encoding: no-such-encoding") do
      Encoding.encode("A", "no-such-encoding")
    end
  end

  test("round-trips through windows-1252") do
    original = "Curaçao — déjà vu"
    assert_eq(Encoding.decode(Encoding.encode(original, "windows-1252"), "windows-1252"), original)
  end
end

describe("charset-aware file import") do
  after_each() do
    File.delete(LATIN1_PATH) if file_exists(LATIN1_PATH)
  end

  test("slurp(path, \"latin1\") reads a Latin-1 file as UTF-8") do
    barf(LATIN1_PATH, Encoding.encode("café", "latin1"))
    # The bytes on disk are Latin-1, not UTF-8.
    assert_eq(slurp(LATIN1_PATH, "binary"), CAFE_LATIN1)
    assert_eq(slurp(LATIN1_PATH, "latin1"), "café")
  end

  test("File.read(path, \"latin1\") decodes the same way") do
    barf(LATIN1_PATH, Encoding.encode("déjà", "latin1"))
    assert_eq(File.read(LATIN1_PATH, "latin1"), "déjà")
  end

  test("slurp raises on an unknown mode") do
    barf(LATIN1_PATH, "hello")
    assert_raises("slurp: unknown mode/encoding 'not-an-encoding'") do
      slurp(LATIN1_PATH, "not-an-encoding")
    end
  end

  test("File.read raises on an unknown encoding") do
    barf(LATIN1_PATH, "hello")
    assert_raises("unknown encoding: nope") do
      File.read(LATIN1_PATH, "nope")
    end
  end
end
