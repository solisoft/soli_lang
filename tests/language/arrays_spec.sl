// ============================================================================
// Arrays Test Suite
// ============================================================================

describe("Array Operations", fn() {
    test("array subtraction removes matching elements", fn() {
        let a = [1, 2, 3];
        let b = [1];
        let result = a - b;
        assert_eq(len(result), 2);
        assert_eq(result[0], 2);
        assert_eq(result[1], 3);
    });

    test("array concatenation with +", fn() {
        let a = [1, 2];
        let b = [3, 4];
        let result = a + b;
        assert_eq(len(result), 4);
        assert_eq(result[0], 1);
        assert_eq(result[1], 2);
        assert_eq(result[2], 3);
        assert_eq(result[3], 4);
    });

    test("array concatenation returns new array", fn() {
        let a = [1, 2];
        let b = [3, 4];
        let result = a + b;
        assert_eq(len(a), 2);
        assert_eq(len(b), 2);
    });

    test("array subtraction removes multiple matches", fn() {
        let a = [1, 2, 1, 3, 1];
        let b = [1];
        let result = a - b;
        assert_eq(len(result), 2);
        assert_eq(result[0], 2);
        assert_eq(result[1], 3);
    });

    test("array subtraction with no matches returns copy", fn() {
        let a = [1, 2, 3];
        let b = [99];
        let result = a - b;
        assert_eq(len(result), 3);
        assert_eq(result[0], 1);
        assert_eq(result[1], 2);
        assert_eq(result[2], 3);
    });

    test("array subtraction with empty array", fn() {
        let a = [1, 2, 3];
        let b = [];
        let result = a - b;
        assert_eq(len(result), 3);
        assert_eq(result[0], 1);
        assert_eq(result[1], 2);
        assert_eq(result[2], 3);
    });

    test("array subtraction with strings", fn() {
        let a = ["apple", "banana", "cherry"];
        let b = ["banana"];
        let result = a - b;
        assert_eq(len(result), 2);
        assert_eq(result[0], "apple");
        assert_eq(result[1], "cherry");
    });

    test("array subtraction with instances uses identity", fn() {
        class Person {
            name: String;
            fn new(n) { this.name = n; }
        }
        let p1 = Person.new({"name": "Alice"});
        let p2 = Person.new({"name": "Bob"});
        let arr = [p1, p2];
        let to_remove = [p1];
        let result = arr - to_remove;
        assert_eq(len(result), 1);
        assert_eq(result[0].name, "Bob");
    });

    test("array indexing", fn() {
        let arr = ["a", "b", "c"];
        assert_eq(arr[0], "a");
        assert_eq(arr[1], "b");
        assert_eq(arr[2], "c");
    });

    test("array index assignment", fn() {
        let arr = [1, 2, 3];
        arr[1] = 20;
        assert_eq(arr[1], 20);
    });

    test("array spread operator", fn() {
        let a = [1, 2];
        let b = [3, 4];
        let c = [...a, ...b];
        assert_eq(len(c), 4);
        assert_eq(c[0], 1);
        assert_eq(c[3], 4);
    });

    test("array of mixed types", fn() {
        let arr = [1, "two", true, null];
        assert_eq(arr[0], 1);
        assert_eq(arr[1], "two");
        assert_eq(arr[2], true);
        assert_null(arr[3]);
    });

    test("nested array indexing", fn() {
        let arr = [[1, 2], [3, 4], [5, 6]];
        assert_eq(arr[0][1], 2);
        assert_eq(arr[2][0], 5);
    });

    test("array with negative index", fn() {
        let arr = [10, 20, 30];
        assert_eq(arr[-1], 30);
        assert_eq(arr[-2], 20);
    });
});

describe("Array Methods", fn() {
    test("map on array", fn() {
        let arr = [1, 2, 3];
        let doubled = arr.map(fn(x) { return x * 2; });
        assert_eq(doubled[0], 2);
        assert_eq(doubled[1], 4);
        assert_eq(doubled[2], 6);
    });

    test("filter on array", fn() {
        let arr = [1, 2, 3, 4, 5];
        let evens = arr.filter(fn(x) { return x % 2 == 0; });
        assert_eq(len(evens), 2);
        assert_eq(evens[0], 2);
        assert_eq(evens[1], 4);
    });

    test("each on array", fn() {
        let arr = ["a", "b", "c"];
        let result = "";
        arr.each(fn(x) { result = result + x; });
        assert_eq(result, "abc");
    });

    test("each_with_index on array", fn() {
        let arr = ["a", "b", "c"];
        let result = "";
        arr.each_with_index(fn(x, i) { result = result + str(i) + x; });
        assert_eq(result, "0a1b2c");
    });

    test("each_with_index returns the array", fn() {
        let arr = [10, 20, 30];
        let returned = arr.each_with_index(fn(x, i) { x; });
        assert_eq(len(returned), 3);
        assert_eq(returned[0], 10);
    });

    test("index_of finds first matching element", fn() {
        let arr = ["a", "b", "c", "b"];
        assert_eq(arr.index_of("b"), 1);
        assert_eq(arr.index_of("a"), 0);
        assert_eq(arr.index_of("c"), 2);
    });

    test("index_of returns -1 when not found", fn() {
        let arr = [1, 2, 3];
        assert_eq(arr.index_of(99), -1);
    });

    test("index_of works on empty array", fn() {
        let arr = [];
        assert_eq(arr.index_of(1), -1);
    });

    test("reduce on array", fn() {
        let arr = [1, 2, 3, 4];
        let sum = arr.reduce(fn(acc, x) { return acc + x; }, 0);
        assert_eq(sum, 10);
    });

    test("slice with start and end", fn() {
        let arr = [1, 2, 3, 4, 5];
        let result = arr.slice(1, 3);
        assert_eq(len(result), 2);
        assert_eq(result[0], 2);
        assert_eq(result[1], 3);
    });

    test("slice with negative start", fn() {
        let arr = [1, 2, 3, 4, 5];
        let result = arr.slice(-2);
        assert_eq(len(result), 2);
        assert_eq(result[0], 4);
        assert_eq(result[1], 5);
    });

    test("slice with negative end", fn() {
        let arr = [1, 2, 3, 4, 5];
        let result = arr.slice(1, -1);
        assert_eq(len(result), 3);
        assert_eq(result[0], 2);
        assert_eq(result[1], 3);
        assert_eq(result[2], 4);
    });

    test("slice out of bounds returns empty", fn() {
        let arr = [1, 2, 3];
        let result = arr.slice(5);
        assert_eq(len(result), 0);
    });

    test("slice does not mutate original", fn() {
        let arr = [1, 2, 3, 4, 5];
        let original_len = len(arr);
        arr.slice(1, 3);
        assert_eq(len(arr), original_len);
        assert_eq(arr[0], 1);
        assert_eq(arr[1], 2);
        assert_eq(arr[2], 3);
    });

    test("slice with no arguments returns copy", fn() {
        let arr = [1, 2, 3];
        let result = arr.slice();
        assert_eq(len(result), 3);
        assert_eq(result[0], 1);
        assert_eq(result[1], 2);
        assert_eq(result[2], 3);
    });
});

describe("Array - Ruby-compat methods", fn() {
    test("size is alias for length", fn() {
        assert_eq([1, 2, 3].size, 3);
        assert_eq([].size, 0);
    });

    test("delete removes matching elements", fn() {
        assert_eq([1, 2, 3, 2].delete(2), [1, 3]);
    });

    test("delete returns null if not found", fn() {
        assert_null([1, 2].delete(99));
    });

    test("delete_at removes at index", fn() {
        assert_eq([1, 2, 3].delete_at(1), [1, 3]);
    });

    test("delete_at with negative index", fn() {
        assert_eq([1, 2, 3].delete_at(-1), [1, 2]);
    });

    test("delete_at returns null if out of bounds", fn() {
        assert_null([1].delete_at(5));
    });

    test("shift removes first element", fn() {
        assert_eq([1, 2, 3].shift, [2, 3]);
    });

    test("shift on empty returns null", fn() {
        assert_null([].shift);
    });

    test("unshift prepends elements", fn() {
        assert_eq([1, 2].unshift(0), [0, 1, 2]);
    });

    test("insert at index", fn() {
        assert_eq([1, 3].insert(1, 2), [1, 2, 3]);
    });

    test("insert multiple values", fn() {
        assert_eq([1, 4].insert(1, 2, 3), [1, 2, 3, 4]);
    });

    test("rotate with default count", fn() {
        assert_eq([1, 2, 3].rotate, [2, 3, 1]);
    });

    test("rotate with explicit count", fn() {
        assert_eq([1, 2, 3].rotate(2), [3, 1, 2]);
    });

    test("rotate with negative count", fn() {
        assert_eq([1, 2, 3].rotate(-1), [3, 1, 2]);
    });

    test("reject is opposite of filter", fn() {
        assert_eq([1, 2, 3, 4].reject(fn(x) x % 2 == 0), [1, 3]);
    });

    test("none? returns true when no matches", fn() {
        assert([1, 2].none?(fn(x) x > 10));
    });

    test("none? returns false when any match", fn() {
        assert_not([1, 2].none?(fn(x) x == 2));
    });

    test("one? returns true when exactly one match", fn() {
        assert([1, 2, 3].one?(fn(x) x == 2));
    });

    test("one? returns false when multiple match", fn() {
        assert_not([1, 2, 2].one?(fn(x) x == 2));
    });

    test("values_at returns selected indices", fn() {
        assert_eq([10, 20, 30, 40].values_at(0, 2), [10, 30]);
    });

    test("values_at with negative indices", fn() {
        assert_eq([10, 20, 30].values_at(0, -1), [10, 30]);
    });

    test("count with no args returns length", fn() {
        assert_eq([1, 2, 3].count, 3);
    });

    test("count with value", fn() {
        assert_eq([1, 2, 2, 3].count(2), 2);
    });

    test("count with block", fn() {
        assert_eq([1, 2, 3, 4].count(fn(x) x % 2 == 0), 2);
    });
});

describe("Array#dig - safe nested access", fn() {
    test("dig on simple array with integer index", fn() {
        let arr = [10, 20, 30];
        assert_eq(arr.dig(0), 10);
        assert_eq(arr.dig(2), 30);
    });

    test("dig with negative index", fn() {
        let arr = [10, 20, 30];
        assert_eq(arr.dig(-1), 30);
        assert_eq(arr.dig(-2), 20);
        assert_eq(arr.dig(-3), 10);
    });

    test("dig returns null for out of bounds", fn() {
        let arr = [1, 2, 3];
        assert_null(arr.dig(5));
        assert_null(arr.dig(-10));
    });

    test("dig into nested arrays", fn() {
        let arr = [[1, 2], [3, [4, 5]]];
        assert_eq(arr.dig(0, 1), 2);
        assert_eq(arr.dig(1, 1, 0), 4);
    });

    test("dig into nested hashes inside array", fn() {
        let arr = [{"user": {"name": "Alice"}}, {"user": {"name": "Bob"}}];
        assert_eq(arr.dig(0, "user", "name"), "Alice");
        assert_eq(arr.dig(1, "user", "name"), "Bob");
    });

    test("dig mixed array and hash path", fn() {
        let data = [{"items": [10, {"price": 99}]}];
        assert_eq(data.dig(0, "items", 1, "price"), 99);
    });

    test("dig stops and returns null on first miss", fn() {
        let arr = [[1, 2], [3, 4]];
        assert_null(arr.dig(0, 5));      # index 5 does not exist in [1,2]
        assert_null(arr.dig(9, 0));      # first key already misses
    });

    test("dig with empty path returns the array itself (wrapped)", fn() {
        # dig requires at least one argument (consistent with Hash#dig)
        # Calling with zero args is an arity error.
        let err = null;
        try {
            [1, 2].dig();
        } catch (e) {
            err = e;
        }
        assert(err != null);
    });
});

describe("Array#pluck - extract fields from arrays of hashes/arrays", fn() {
    test("pluck single field from array of hashes", fn() {
        let posts = [
            { "id": 1, "title": "Hello" },
            { "id": 2, "title": "World" },
            { "id": 3, "title": "Soli" }
        ];
        assert_eq(posts.pluck("title"), ["Hello", "World", "Soli"]);
        assert_eq(posts.pluck("id"), [1, 2, 3]);
    });

    test("pluck returns null for missing keys", fn() {
        let users = [
            { "name": "Alice" },
            { "name": "Bob", "email": "bob@example.com" }
        ];
        assert_eq(users.pluck("email"), [null, "bob@example.com"]);
    });

    test("pluck multiple fields returns array of arrays", fn() {
        let data = [
            { "name": "Alice", "age": 30 },
            { "name": "Bob", "age": 25 }
        ];
        let result = data.pluck("name", "age");
        assert_eq(len(result), 2);
        assert_eq(result[0], ["Alice", 30]);
        assert_eq(result[1], ["Bob", 25]);
    });

    test("pluck works with integer indices on arrays of arrays", fn() {
        let rows = [
            [10, "foo"],
            [20, "bar"],
            [30, "baz"]
        ];
        assert_eq(rows.pluck(0), [10, 20, 30]);
        assert_eq(rows.pluck(1), ["foo", "bar", "baz"]);
    });

    test("pluck multiple indices returns rows as arrays", fn() {
        let matrix = [
            [1, 2, 3],
            [4, 5, 6]
        ];
        assert_eq(matrix.pluck(0, 2), [[1, 3], [4, 6]]);
    });

    test("pluck on empty array returns empty array", fn() {
        assert_eq([].pluck("anything"), []);
    });

    test("pluck on non-hash/non-array elements returns nulls", fn() {
        let mixed = [ { "x": 1 }, "not a hash", 42, null ];
        assert_eq(mixed.pluck("x"), [1, null, null, null]);
    });
});

describe("Array#pick - first matching value(s) from the first element", fn() {
    test("pick single field returns value from first element", fn() {
        let posts = [
            { "id": 1, "title": "Hello" },
            { "id": 2, "title": "World" }
        ];
        assert_eq(posts.pick("title"), "Hello");
        assert_eq(posts.pick("id"), 1);
    });

    test("pick returns null on empty array", fn() {
        assert_null([].pick("title"));
    });

    test("pick returns null when first element lacks the key", fn() {
        let users = [
            { "name": "Alice" },
            { "name": "Bob", "email": "bob@x.com" }
        ];
        assert_null(users.pick("email"));  # first row has no email
    });

    test("pick multiple fields returns array of values from first element", fn() {
        let row = [
            { "name": "Alice", "age": 30, "active": true },
            { "name": "Bob", "age": 25 }
        ];
        assert_eq(row.pick("name", "age"), ["Alice", 30]);
    });

    test("pick with integer indices works on arrays of arrays", fn() {
        let rows = [
            [10, "foo"],
            [20, "bar"]
        ];
        assert_eq(rows.pick(1), "foo");
        assert_eq(rows.pick(0, 1), [10, "foo"]);
    });

    test("pick on non-hash first element returns null", fn() {
        assert_null([ "string", { "x": 1 } ].pick("x"));
    });
});

describe("Array - Set Operations", fn() {
    test("intersection returns shared elements in receiver order", fn() {
        assert_eq([1, 2, 3].intersection([2, 3, 4]), [2, 3]);
    });

    test("intersection deduplicates the result", fn() {
        assert_eq([1, 1, 2, 2, 3].intersection([1, 2]), [1, 2]);
    });

    test("intersection with no overlap returns empty array", fn() {
        assert_eq([1, 2, 3].intersection([4, 5, 6]), []);
    });

    test("intersection with empty receiver returns empty array", fn() {
        assert_eq([].intersection([1, 2, 3]), []);
    });

    test("intersection with empty other returns empty array", fn() {
        assert_eq([1, 2, 3].intersection([]), []);
    });

    test("intersection works with strings", fn() {
        assert_eq(["a", "b", "c"].intersection(["b", "c", "d"]), ["b", "c"]);
    });

    test("union returns all elements, receiver first, deduplicated", fn() {
        assert_eq([1, 2, 3].union([2, 3, 4]), [1, 2, 3, 4]);
    });

    test("union deduplicates within receiver and across", fn() {
        assert_eq([1, 1, 2].union([2, 3]), [1, 2, 3]);
    });

    test("union with empty other returns deduped receiver", fn() {
        assert_eq([1, 1, 2, 3].union([]), [1, 2, 3]);
    });

    test("union with empty receiver returns deduped other", fn() {
        assert_eq([].union([1, 2, 2, 3]), [1, 2, 3]);
    });

    test("union with two empty arrays returns empty array", fn() {
        assert_eq([].union([]), []);
    });

    test("difference returns receiver elements not in other", fn() {
        assert_eq([1, 2, 3].difference([2, 3]), [1]);
    });

    test("difference deduplicates the result", fn() {
        assert_eq([1, 1, 2, 2, 3].difference([3]), [1, 2]);
    });

    test("difference with no overlap returns deduped receiver", fn() {
        assert_eq([1, 1, 2].difference([3, 4]), [1, 2]);
    });

    test("difference where other contains all returns empty array", fn() {
        assert_eq([1, 2, 3].difference([1, 2, 3]), []);
    });

    test("difference with empty other returns deduped receiver", fn() {
        assert_eq([1, 1, 2, 3].difference([]), [1, 2, 3]);
    });

    test("difference with empty receiver returns empty array", fn() {
        assert_eq([].difference([1, 2]), []);
    });

    test("set operations leave originals unchanged", fn() {
        let a = [1, 2, 3];
        let b = [2, 3, 4];
        a.intersection(b);
        a.union(b);
        a.difference(b);
        assert_eq(a, [1, 2, 3]);
        assert_eq(b, [2, 3, 4]);
    });
});
