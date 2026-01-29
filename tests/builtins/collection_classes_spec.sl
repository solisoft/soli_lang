// ============================================================================
// Collection Classes Test Suite
// Tests for String, Array, and Hash wrapper classes with to_string() methods
// ============================================================================

describe("Collection Classes", fn() {
    describe("String Class", fn() {
        test("String literals are wrapped in String class", fn() {
            let s = "hello";
            assert_eq(s.to_string(), "hello");
        });

        test("String.to_string() returns wrapped value", fn() {
            let s = String.new("test");
            assert_eq(s.to_string(), "test");
        });

        test("String.length() returns string length", fn() {
            let s = String.new("hello");
            assert_eq(s.length(), 5);
        });

        test("String.upcase() returns uppercase string", fn() {
            let s = String.new("hello");
            let upper = s.upcase();
            assert_eq(upper.to_string(), "HELLO");
        });

        test("String.downcase() returns lowercase string", fn() {
            let s = String.new("HELLO");
            let lower = s.downcase();
            assert_eq(lower.to_string(), "hello");
        });

        test("String.trim() removes whitespace", fn() {
            let s = String.new("  hello  ");
            let trimmed = s.trim();
            assert_eq(trimmed.to_string(), "hello");
        });

        test("String.contains() checks for substring", fn() {
            let s = String.new("hello world");
            assert(s.contains("world"));
            assert_not(s.contains("foo"));
        });

        test("String.starts_with() checks prefix", fn() {
            let s = String.new("hello world");
            assert(s.starts_with("hello"));
            assert_not(s.starts_with("world"));
        });

        test("String.ends_with() checks suffix", fn() {
            let s = String.new("hello world");
            assert(s.ends_with("world"));
            assert_not(s.ends_with("hello"));
        });

        test("String.split() returns array of parts", fn() {
            let s = String.new("a,b,c");
            let parts = s.split(",");
            assert_eq(parts.length(), 3);
        });

        test("String.new() creates string from various types", fn() {
            assert_eq(String.new(42).to_string(), "42");
            assert_eq(String.new(3.14).to_string(), "3.14");
            assert_eq(String.new(true).to_string(), "true");
        });

        test("String.index_of() returns substring position", fn() {
            let s = String.new("hello world");
            assert_eq(s.index_of("world"), 6);
            assert_eq(s.index_of("hello"), 0);
            assert_eq(s.index_of("foo"), -1);
        });

        test("String.substring() extracts part of string", fn() {
            let s = String.new("hello world");
            assert_eq(s.substring(0, 5).to_string(), "hello");
            assert_eq(s.substring(6, 11).to_string(), "world");
        });

        test("String.replace() replaces substring", fn() {
            let s = String.new("hello world");
            let replaced = s.replace("world", "soli");
            assert_eq(replaced.to_string(), "hello soli");
        });

        test("String.lpad() left pads string", fn() {
            let s = String.new("hi");
            assert_eq(s.lpad(5).to_string(), "   hi");
            assert_eq(s.lpad(5, "*").to_string(), "***hi");
        });

        test("String.rpad() right pads string", fn() {
            let s = String.new("hi");
            assert_eq(s.rpad(5).to_string(), "hi   ");
            assert_eq(s.rpad(5, "*").to_string(), "hi***");
        });
    });

    describe("Array Class", fn() {
        test("Array literals are wrapped in Array class", fn() {
            let arr = [1, 2, 3];
            let repr = arr.to_string();
            assert(repr.contains("1"));
            assert(repr.contains("2"));
            assert(repr.contains("3"));
        });

        test("Array.to_string() returns formatted string", fn() {
            let arr = Array.new();
            arr.push(1);
            arr.push(2);
            arr.push(3);
            assert_eq(arr.to_string(), "[1, 2, 3]");
        });

        test("Array.length() returns array length", fn() {
            let arr = Array.new();
            arr.push(1);
            arr.push(2);
            assert_eq(arr.length(), 2);
        });

        test("Array.push() adds element", fn() {
            let arr = Array.new();
            arr.push(42);
            assert_eq(arr.get(0), 42);
        });

        test("Array.pop() removes and returns last element", fn() {
            let arr = Array.new();
            arr.push(1);
            arr.push(2);
            let popped = arr.pop();
            assert_eq(popped, 2);
            assert_eq(arr.length(), 1);
        });

        test("Array.get() returns element at index", fn() {
            let arr = Array.new();
            arr.push("first");
            arr.push("second");
            assert_eq(arr.get(0), "first");
            assert_eq(arr.get(1), "second");
        });

        test("Array.get() supports negative index", fn() {
            let arr = Array.new();
            arr.push(1);
            arr.push(2);
            arr.push(3);
            assert_eq(arr.get(-1), 3);
            assert_eq(arr.get(-2), 2);
        });

        test("Array.new() creates empty array", fn() {
            let arr = Array.new();
            assert_eq(arr.length(), 0);
        });

        test("Array.clear() removes all elements", fn() {
            let arr = Array.new();
            arr.push(1);
            arr.push(2);
            arr.push(3);
            arr.clear();
            assert_eq(arr.length(), 0);
        });
    });

    describe("Hash Class", fn() {
        test("Hash literals are wrapped in Hash class", fn() {
            let h = {"key": "value"};
            let repr = h.to_string();
            assert(repr.contains("key"));
            assert(repr.contains("value"));
        });

        test("Hash.to_string() returns formatted string", fn() {
            let h = Hash.new();
            h.set("a", 1);
            h.set("b", 2);
            let repr = h.to_string();
            assert(repr.contains("a"));
            assert(repr.contains("b"));
        });

        test("Hash.length() returns entry count", fn() {
            let h = Hash.new();
            h.set("a", 1);
            h.set("b", 2);
            assert_eq(h.length(), 2);
        });

        test("Hash.get() returns value by key", fn() {
            let h = Hash.new();
            h.set("name", "test");
            assert_eq(h.get("name"), "test");
        });

        test("Hash.set() sets value by key", fn() {
            let h = Hash.new();
            h.set("key", "value");
            assert_eq(h.get("key"), "value");
        });

        test("Hash.has_key() checks key existence", fn() {
            let h = Hash.new();
            h.set("exists", true);
            assert(h.has_key("exists"));
            assert_not(h.has_key("missing"));
        });

        test("Hash.keys() returns array of keys", fn() {
            let h = Hash.new();
            h.set("a", 1);
            h.set("b", 2);
            let k = h.keys();
            assert_eq(k.length(), 2);
        });

        test("Hash.values() returns array of values", fn() {
            let h = Hash.new();
            h.set("a", 1);
            h.set("b", 2);
            let v = h.values();
            assert_eq(v.length(), 2);
        });

        test("Hash.new() creates empty hash", fn() {
            let h = Hash.new();
            assert_eq(h.length(), 0);
        });

        test("Hash.delete() removes key and returns value", fn() {
            let h = Hash.new();
            h.set("a", 1);
            h.set("b", 2);
            let deleted = h.delete("a");
            assert_eq(deleted, 1);
            assert_eq(h.length(), 1);
            assert_not(h.has_key("a"));
        });

        test("Hash.merge() combines two hashes", fn() {
            let h1 = Hash.new();
            h1.set("a", 1);
            let h2 = Hash.new();
            h2.set("b", 2);
            let merged = h1.merge(h2);
            assert_eq(merged.length(), 2);
            assert_eq(merged.get("a"), 1);
            assert_eq(merged.get("b"), 2);
        });

        test("Hash.entries() returns key-value pairs", fn() {
            let h = Hash.new();
            h.set("a", 1);
            let e = h.entries();
            assert_eq(e.length(), 1);
        });

        test("Hash.clear() removes all entries", fn() {
            let h = Hash.new();
            h.set("a", 1);
            h.set("b", 2);
            h.clear();
            assert_eq(h.length(), 0);
        });
    });

    describe("REPL to_string() Integration", fn() {
        test("String literals show to_string() in REPL", fn() {
            let s = "hello";
            let result = s.to_string();
            assert_eq(result, "hello");
        });

        test("Array literals show to_string() in REPL", fn() {
            let arr = [1, 2, 3];
            let repr = arr.to_string();
            assert(repr == "[1, 2, 3]");
        });

        test("Hash literals show to_string() in REPL", fn() {
            let h = {"a": 1, "b": 2};
            let repr = h.to_string();
            assert(repr.contains("a"));
            assert(repr.contains("b"));
        });
    });

    describe("Global len() Function with Class Instances", fn() {
        test("len() works with String class instance", fn() {
            let s = String.new("hello");
            assert_eq(len(s), 5);
        });

        test("len() works with Array class instance", fn() {
            let arr = Array.new();
            arr.push(1);
            arr.push(2);
            assert_eq(len(arr), 2);
        });

        test("len() works with Hash class instance", fn() {
            let h = Hash.new();
            h.set("a", 1);
            h.set("b", 2);
            assert_eq(len(h), 2);
        });
    });

    describe("Base64 Class", fn() {
        test("Base64.encode() encodes string to base64", fn() {
            let encoded = Base64.encode("hello");
            assert_eq(encoded, "aGVsbG8=");
        });

        test("Base64.decode() decodes base64 to string", fn() {
            let decoded = Base64.decode("aGVsbG8=");
            assert_eq(decoded, "hello");
        });

        test("Base64 round-trip works", fn() {
            let original = "Hello, World! 123";
            let encoded = Base64.encode(original);
            let decoded = Base64.decode(encoded);
            assert_eq(decoded, original);
        });

        test("Base64.encode() handles empty string", fn() {
            let encoded = Base64.encode("");
            assert_eq(encoded, "");
        });

        test("Base64.encode() handles special characters", fn() {
            let encoded = Base64.encode("Hello\nWorld\t!");
            assert_eq(encoded, "SGVsbG8KV29ybGQhIQ==");
        });

        test("Base64.decode() handles URL-safe base64", fn() {
            let encoded = Base64.encode("foo/bar?query=value");
            let decoded = Base64.decode(encoded);
            assert_eq(decoded, "foo/bar?query=value");
        });
    });
});
