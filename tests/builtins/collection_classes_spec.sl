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

        test("String.length() returns string length", fn() {
            let s = "hello";
            assert_eq(s.length(), 5);
        });

        test("String.upcase() returns uppercase string", fn() {
            let s = "hello";
            let upper = s.upcase();
            assert_eq(upper.to_string(), "HELLO");
        });

        test("String.downcase() returns lowercase string", fn() {
            let s = "HELLO";
            let lower = s.downcase();
            assert_eq(lower.to_string(), "hello");
        });

        test("String.trim() removes whitespace", fn() {
            let s = "  hello  ";
            let trimmed = s.trim();
            assert_eq(trimmed.to_string(), "hello");
        });

        test("String.contains() checks for substring", fn() {
            let s = "hello world";
            assert(s.contains("world"));
            assert_not(s.contains("foo"));
        });

        test("String.starts_with() checks prefix", fn() {
            let s = "hello world";
            assert(s.starts_with("hello"));
            assert_not(s.starts_with("world"));
        });

        test("String.ends_with() checks suffix", fn() {
            let s = "hello world";
            assert(s.ends_with("world"));
            assert_not(s.ends_with("hello"));
        });

        test("String.split() returns array of parts", fn() {
            let s = "a,b,c";
            let parts = s.split(",");
            assert_eq(len(parts), 3);
        });

        test("String.index_of() returns substring position", fn() {
            let s = "hello world";
            assert_eq(s.index_of("world"), 6);
            assert_eq(s.index_of("hello"), 0);
            assert_eq(s.index_of("foo"), -1);
        });

        test("String.substring() extracts part of string", fn() {
            let s = "hello world";
            assert_eq(s.substring(0, 5).to_string(), "hello");
            assert_eq(s.substring(6, 11).to_string(), "world");
        });

        test("String.replace() replaces substring", fn() {
            let s = "hello world";
            let replaced = s.replace("world", "soli");
            assert_eq(replaced.to_string(), "hello soli");
        });

        test("String.lpad() left pads string", fn() {
            let s = "hi";
            assert_eq(s.lpad(5).to_string(), "   hi");
            assert_eq(s.lpad(5, "*").to_string(), "***hi");
        });

        test("String.rpad() right pads string", fn() {
            let s = "hi";
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
            let arr = [1, 2, 3];
            assert_eq(arr.to_string(), "[1, 2, 3]");
        });

        test("Array.length() returns array length", fn() {
            let arr = [1, 2];
            assert_eq(arr.length(), 2);
        });

        test("Array.push() adds element", fn() {
            let arr = [];
            arr.push(42);
            assert_eq(arr.get(0), 42);
        });

        test("Array.pop() removes and returns last element", fn() {
            let arr = [1, 2];
            let popped = arr.pop();
            assert_eq(popped, 2);
            assert_eq(arr.length(), 1);
        });

        test("Array.get() returns element at index", fn() {
            let arr = ["first", "second"];
            assert_eq(arr.get(0), "first");
            assert_eq(arr.get(1), "second");
        });

        test("Array.get() supports negative index", fn() {
            let arr = [1, 2, 3];
            assert_eq(arr.get(-1), 3);
            assert_eq(arr.get(-2), 2);
        });

        test("Array.clear() removes all elements", fn() {
            let arr = [1, 2, 3];
            arr.clear();
            assert_eq(arr.length(), 0);
        });

        test("Array.first() returns first element", fn() {
            let arr = [1, 2, 3];
            assert_eq(arr.first(), 1);
        });

        test("Array.last() returns last element", fn() {
            let arr = [1, 2, 3];
            assert_eq(arr.last(), 3);
        });

        test("Array.reverse() returns reversed array", fn() {
            let arr = [1, 2, 3];
            let rev = arr.reverse();
            assert_eq(rev.first(), 3);
            assert_eq(rev.last(), 1);
        });

        test("Array.uniq() removes duplicates", fn() {
            let arr = [1, 2, 2, 3, 3, 3];
            let unique = arr.uniq();
            assert_eq(unique.length(), 3);
        });

        test("Array.take() returns first n elements", fn() {
            let arr = [1, 2, 3, 4, 5];
            let taken = arr.take(3);
            assert_eq(taken.length(), 3);
            assert_eq(taken.last(), 3);
        });

        test("Array.drop() returns array without first n elements", fn() {
            let arr = [1, 2, 3, 4, 5];
            let dropped = arr.drop(2);
            assert_eq(dropped.length(), 3);
            assert_eq(dropped.first(), 3);
        });

        test("Array.sum() returns sum of elements", fn() {
            let arr = [1, 2, 3, 4, 5];
            assert_eq(arr.sum(), 15.0);
        });

        test("Array.min() returns minimum value", fn() {
            let arr = [3, 1, 4, 1, 5];
            assert_eq(arr.min(), 1);
        });

        test("Array.max() returns maximum value", fn() {
            let arr = [3, 1, 4, 1, 5];
            assert_eq(arr.max(), 5);
        });

        test("Array.empty?() checks if array is empty", fn() {
            let empty = [];
            let full = [1, 2, 3];
            assert(empty.empty?());
            assert_not(full.empty?());
        });

        test("Array.include?() checks if array contains element", fn() {
            let arr = [1, 2, 3];
            assert(arr.include?(2));
            assert_not(arr.include?(5));
        });

        test("Array.join() joins elements with delimiter", fn() {
            let arr = [1, 2, 3];
            assert_eq(arr.join("-"), "1-2-3");
        });

        test("Array.zip() pairs elements with another array", fn() {
            let a = [1, 2, 3];
            let b = [4, 5, 6];
            let zipped = a.zip(b);
            assert_eq(zipped.length(), 3);
        });

        test("Array.find() returns first matching element", fn() {
            let arr = [1, 2, 3, 4, 5];
            let found = arr.find(fn(x) { return x > 3; });
            assert_eq(found, 4);
        });

        test("Array.find() returns null when no match", fn() {
            let arr = [1, 2, 3];
            let found = arr.find(fn(x) { return x > 10; });
            assert_null(found);
        });

        test("Array.any?() checks if any element matches", fn() {
            let arr = [1, 2, 3, 4];
            assert(arr.any?(fn(x) { return x > 3; }));
            assert_not(arr.any?(fn(x) { return x > 10; }));
        });

        test("Array.all?() checks if all elements match", fn() {
            let arr = [2, 4, 6];
            assert(arr.all?(fn(x) { return x % 2 == 0; }));
            assert_not([1, 2, 3].all?(fn(x) { return x > 2; }));
        });

        test("Array.sort() sorts elements", fn() {
            let arr = [3, 1, 4, 1, 5];
            let sorted = arr.sort();
            assert_eq(sorted.get(0), 1);
            assert_eq(sorted.get(1), 1);
            assert_eq(sorted.get(4), 5);
        });

        test("Array.sort() sorts strings", fn() {
            let arr = ["banana", "apple", "cherry"];
            let sorted = arr.sort();
            assert_eq(sorted.get(0), "apple");
            assert_eq(sorted.get(2), "cherry");
        });

        test("Array.sort_by() sorts by key function", fn() {
            let arr = [{"name": "Charlie"}, {"name": "Alice"}, {"name": "Bob"}];
            let sorted = arr.sort_by("name");
            assert_eq(sorted.get(0).get("name"), "Alice");
            assert_eq(sorted.get(2).get("name"), "Charlie");
        });

        test("Array.compact() removes null values", fn() {
            let arr = [1, null, 2, null, 3];
            let compacted = arr.compact();
            assert_eq(compacted.length(), 3);
            assert_eq(compacted.get(0), 1);
            assert_eq(compacted.get(1), 2);
            assert_eq(compacted.get(2), 3);
        });

        test("Array.flatten() flattens nested arrays", fn() {
            let arr = [1, [2, 3], [4, [5, 6]]];
            let flat = arr.flatten();
            assert_eq(flat.length(), 6);
            assert_eq(flat.get(0), 1);
            assert_eq(flat.get(5), 6);
        });

        test("Array.sample() returns an element from the array", fn() {
            let arr = [1, 2, 3, 4, 5];
            let s = arr.sample();
            assert(arr.include?(s));
        });

        test("Array.shuffle() returns shuffled array with same elements", fn() {
            let arr = [1, 2, 3, 4, 5];
            let shuffled = arr.shuffle();
            assert_eq(shuffled.length(), 5);
            assert(shuffled.include?(1));
            assert(shuffled.include?(5));
        });
    });

    describe("Hash Class", fn() {
        test("Hash literals are wrapped in Hash class", fn() {
            let h = {"key": "value"};
            let repr = h.to_string();
            assert(repr.contains("key"));
            assert(repr.contains("value"));
        });

        test("Hash.to_string() formats hash correctly", fn() {
            let h = {"a" => 1, "b" => 2};
            let repr = h.to_string();
            assert(repr.contains("a => 1"));
            assert(repr.contains("b => 2"));
        });

        test("Hash.length() returns entry count", fn() {
            let h = {"a": 1, "b": 2};
            assert_eq(h.length(), 2);
        });

        test("Hash.get() returns value by key", fn() {
            let h = {"name": "test"};
            assert_eq(h.get("name"), "test");
        });

        test("Hash.get() returns null for missing key", fn() {
            let h = {"a": 1};
            assert_null(h.get("missing"));
        });

        test("Hash.set() sets value by key", fn() {
            let h = {};
            h.set("key", "value");
            assert_eq(h.get("key"), "value");
        });

        test("Hash.has_key() checks key existence", fn() {
            let h = {"exists": true};
            assert(h.has_key("exists"));
            assert_not(h.has_key("missing"));
        });

        test("Hash.keys() returns array of keys", fn() {
            let h = {"a": 1, "b": 2};
            let k = h.keys();
            assert_eq(k.length(), 2);
        });

        test("Hash.values() returns array of values", fn() {
            let h = {"a": 1, "b": 2};
            let v = h.values();
            assert_eq(v.length(), 2);
        });

        test("Hash.delete() removes key and returns value", fn() {
            let h = {"a": 1, "b": 2};
            let deleted = h.delete("a");
            assert_eq(deleted, 1);
            assert_eq(h.length(), 1);
            assert_not(h.has_key("a"));
        });

        test("Hash.merge() combines two hashes", fn() {
            let h1 = {"a": 1};
            let h2 = {"b": 2};
            let merged = h1.merge(h2);
            assert_eq(merged.length(), 2);
            assert_eq(merged.get("a"), 1);
            assert_eq(merged.get("b"), 2);
        });

        test("Hash.entries() returns key-value pairs", fn() {
            let h = {"a": 1};
            let e = h.entries();
            assert_eq(e.length(), 1);
        });

        test("Hash.clear() removes all entries", fn() {
            let h = {"a": 1, "b": 2};
            h.clear();
            assert_eq(h.length(), 0);
        });

        test("Hash.empty?() checks if hash is empty", fn() {
            assert({}.empty?());
            assert_not({"a": 1}.empty?());
        });

        test("Hash.each() iterates over entries", fn() {
            let h = {"a": 1, "b": 2};
            let keys = [];
            h.each(fn(k, v) { keys.push(k); });
            assert_eq(keys.length(), 2);
        });

        test("Hash.map() transforms entries", fn() {
            let h = {"a": 1, "b": 2};
            let result = h.map(fn(k, v) { return [k, v * 10]; });
            assert_eq(result.get("a"), 10);
            assert_eq(result.get("b"), 20);
        });

        test("Hash.filter() filters entries", fn() {
            let h = {"a": 1, "b": 2, "c": 3};
            let result = h.filter(fn(k, v) { return v > 1; });
            assert_eq(result.length(), 2);
            assert_not(result.has_key("a"));
        });

        test("Hash.fetch() retrieves value or raises error", fn() {
            let h = {"a": 1};
            assert_eq(h.fetch("a"), 1);
            assert_eq(h.fetch("missing", "default"), "default");
        });

        test("Hash.fetch() throws on missing key without default", fn() {
            let threw = false;
            try {
                let h = {"a": 1};
                h.fetch("missing");
            } catch (e) {
                threw = true;
            }
            assert(threw);
        });

        test("Hash.invert() swaps keys and values", fn() {
            let h = {"a": 1, "b": 2};
            let inverted = h.invert();
            assert_eq(inverted.get(1), "a");
            assert_eq(inverted.get(2), "b");
        });

        test("Hash.transform_values() transforms all values", fn() {
            let h = {"a": 1, "b": 2};
            let result = h.transform_values(fn(v) { return v * 10; });
            assert_eq(result.get("a"), 10);
            assert_eq(result.get("b"), 20);
        });

        test("Hash.transform_keys() transforms all keys", fn() {
            let h = {"hello": 1, "world": 2};
            let result = h.transform_keys(fn(k) { return k.upcase(); });
            assert_eq(result.get("HELLO"), 1);
            assert_eq(result.get("WORLD"), 2);
        });

        test("Hash.select() selects matching entries", fn() {
            let h = {"a": 1, "b": 2, "c": 3};
            let result = h.select(fn(k, v) { return v >= 2; });
            assert_eq(result.length(), 2);
            assert(result.has_key("b"));
            assert(result.has_key("c"));
        });

        test("Hash.reject() rejects matching entries", fn() {
            let h = {"a": 1, "b": 2, "c": 3};
            let result = h.reject(fn(k, v) { return v >= 2; });
            assert_eq(result.length(), 1);
            assert(result.has_key("a"));
        });

        test("Hash.slice() returns subset by keys", fn() {
            let h = {"a": 1, "b": 2, "c": 3};
            let result = h.slice(["a", "c"]);
            assert_eq(result.length(), 2);
            assert_eq(result.get("a"), 1);
            assert_eq(result.get("c"), 3);
        });

        test("Hash.except() returns hash without specified keys", fn() {
            let h = {"a": 1, "b": 2, "c": 3};
            let result = h.except(["b"]);
            assert_eq(result.length(), 2);
            assert_not(result.has_key("b"));
        });

        test("Hash.compact() removes null values", fn() {
            let h = {"a": 1, "b": null, "c": 3};
            let result = h.compact();
            assert_eq(result.length(), 2);
            assert_not(result.has_key("b"));
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

    describe("Global len() Function with Literals", fn() {
        test("len() works with string literal", fn() {
            let s = "hello";
            assert_eq(len(s), 5);
        });

        test("len() works with array literal", fn() {
            let arr = [1, 2];
            assert_eq(len(arr), 2);
        });

        test("len() works with hash literal", fn() {
            let h = {"a": 1, "b": 2};
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
            assert_eq(encoded, "SGVsbG8KV29ybGQJIQ==");
        });

        test("Base64.decode() handles URL-safe base64", fn() {
            let encoded = Base64.encode("foo/bar?query=value");
            let decoded = Base64.decode(encoded);
            assert_eq(decoded, "foo/bar?query=value");
        });
    });
});
