// ============================================================================
// List Comprehensions Test Suite
// ============================================================================

describe("List Comprehensions", fn() {
    test("basic list comprehension", fn() {
        let squares = [x * x for x in range(1, 5)];
        assert_eq(len(squares), 4);
        assert_eq(squares[0], 1);
        assert_eq(squares[1], 4);
        assert_eq(squares[2], 9);
        assert_eq(squares[3], 16);
    });

    test("list comprehension with condition", fn() {
        let evens = [x for x in range(1, 10) if x % 2 == 0];
        assert_eq(len(evens), 4);
        assert_eq(evens[0], 2);
        assert_eq(evens[1], 4);
    });

    test("list comprehension with transformation", fn() {
        let words = ["hello", "world"];
        let upper = [w.upcase() for w in words];
        assert_eq(upper[0], "HELLO");
        assert_eq(upper[1], "WORLD");
    });

    test("list comprehension with multiple variables", fn() {
        let result = [];
        for (x in range(1, 3)) {
            for (y in range(1, 3)) {
                result.push([x, y]);
            }
        }
        assert_eq(len(result), 4);
    });

    test("list comprehension with complex expression", fn() {
        let result = [];
        for (x in range(0, 5)) {
            if (x % 2 == 0) {
                result.push(x * 2 + 1);
            }
        }
        assert_eq(result[0], 1);
        assert_eq(result[1], 5);
        assert_eq(result[2], 9);
    });

    test("list comprehension over array", fn() {
        let arr = [10, 20, 30];
        let doubled = [x * 2 for x in arr];
        assert_eq(doubled[0], 20);
        assert_eq(doubled[1], 40);
        assert_eq(doubled[2], 60);
    });

    test("list comprehension with nested expression", fn() {
        let data = [];
        let h1 = hash();
        h1["v"] = 1;
        data.push(h1);
        let h2 = hash();
        h2["v"] = 2;
        data.push(h2);
        let h3 = hash();
        h3["v"] = 3;
        data.push(h3);
        let sum = 0;
        for (item in data) {
            sum = sum + item["v"];
        }
        assert_eq(sum, 6);
    });
});

describe("Hash Comprehensions", fn() {
    test("hash comprehension from array of hashes", fn() {
        let users = [{"name": "Alice", "age": 30}, {"name": "Bob", "age": 25}];
        let ages = {u["name"]: u["age"] for u in users};
        assert_eq(ages["Alice"], 30);
        assert_eq(ages["Bob"], 25);
    });

    test("hash comprehension with computed keys", fn() {
        let items = [{"id": 1, "value": "a"}, {"id": 2, "value": "b"}];
        let mapped = {i["id"]: i["value"] for i in items};
        assert_eq(mapped[1], "a");
        assert_eq(mapped[2], "b");
    });

    test("hash comprehension with condition", fn() {
        let data = [{"name": "Alice", "active": true}, {"name": "Bob", "active": false}, {"name": "Carol", "active": true}];
        let active = {d["name"]: d["active"] for d in data if d["active"]};
        assert_eq(len(active), 2);
        assert(active["Alice"]);
        assert(active["Carol"]);
    });

    test("hash comprehension with string concatenation keys", fn() {
        let items = [{"prefix": "key", "value": 1}, {"prefix": "key", "value": 2}];
        let mapped = {i["prefix"] + "_" + str(i["value"]): i["value"] for i in items};
        assert_eq(mapped["key_1"], 1);
        assert_eq(mapped["key_2"], 2);
    });

    test("hash comprehension with math expression", fn() {
        let nums = [{"n": 1}, {"n": 2}, {"n": 3}];
        let squares = {d["n"]: d["n"] * d["n"] for d in nums};
        assert_eq(squares[1], 1);
        assert_eq(squares[2], 4);
        assert_eq(squares[3], 9);
    });
});
