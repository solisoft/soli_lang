# ============================================================================
# Encoding / charset Test Suite
# ============================================================================
# Tests for Encoding.decode / Encoding.encode and the charset argument on
# slurp() / File.read() — importing and exporting Latin-1 files.
# ============================================================================

describe("Encoding class", fn() {
    test("decode() turns Latin-1 bytes into UTF-8", fn() {
        # café in Latin-1: c=0x63 a=0x61 f=0x66 é=0xE9 (233)
        let bytes = [99, 97, 102, 233];
        assert_eq(Encoding.decode(bytes, "latin1"), "café");
    });

    test("decode() accepts the iso-8859-1 label", fn() {
        assert_eq(Encoding.decode([233], "iso-8859-1"), "é");
    });

    test("encode() turns a UTF-8 string into Latin-1 bytes", fn() {
        assert_eq(Encoding.encode("café", "latin1"), [99, 97, 102, 233]);
    });

    test("encode/decode round-trips through windows-1252", fn() {
        let original = "Curaçao — déjà vu";
        let bytes = Encoding.encode(original, "windows-1252");
        assert_eq(Encoding.decode(bytes, "windows-1252"), original);
    });

    test("unknown encoding label raises", fn() {
        let caught = false;
        try {
            Encoding.decode([65], "no-such-encoding");
        } catch (e) {
            caught = true;
        }
        assert(caught);
    });
});

describe("Charset-aware file import/export", fn() {
    test("slurp(path, \"latin1\") imports a Latin-1 file as UTF-8", fn() {
        let path = "/tmp/soli_test_latin1.txt";
        barf(path, Encoding.encode("café", "latin1"));

        # The raw bytes on disk are Latin-1, not UTF-8.
        assert_eq(slurp(path, "binary"), [99, 97, 102, 233]);
        # Decoded via the charset argument, accents survive.
        assert_eq(slurp(path, "latin1"), "café");
    });

    test("File.read(path, \"latin1\") decodes the same way", fn() {
        let path = "/tmp/soli_test_latin1_fileread.txt";
        barf(path, Encoding.encode("déjà", "latin1"));
        assert_eq(File.read(path, "latin1"), "déjà");
    });

    test("slurp() with an unknown mode raises", fn() {
        let path = "/tmp/soli_test_unknown_mode.txt";
        barf(path, "hello");
        let caught = false;
        try {
            slurp(path, "not-an-encoding");
        } catch (e) {
            caught = true;
        }
        assert(caught);
    });
});
