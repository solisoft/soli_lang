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

    test("reduce on array", fn() {
        let arr = [1, 2, 3, 4];
        let sum = arr.reduce(fn(acc, x) { return acc + x; }, 0);
        assert_eq(sum, 10);
    });
});
