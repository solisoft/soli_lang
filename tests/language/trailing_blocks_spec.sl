// ============================================================================
// Trailing Block Syntax Test Suite
// ============================================================================
// Trailing blocks allow passing a lambda to a method without parentheses:
//   obj.method |params| body end
// Instead of:
//   obj.method(|params| body)
//
// For methods that take additional arguments plus a block:
//   obj.method(arg) |params| body end
// Instead of:
//   obj.method(arg, |params| body)

describe("Trailing Block - Array Methods", fn() {
    test("map with trailing block", fn() {
        let result = [1, 2, 3].map |x| x * 2 end;
        assert_eq(result[0], 2);
        assert_eq(result[1], 4);
        assert_eq(result[2], 6);
    });

    test("filter with trailing block", fn() {
        let result = [1, 2, 3, 4, 5].filter |x| x % 2 == 0 end;
        assert_eq(len(result), 2);
        assert_eq(result[0], 2);
        assert_eq(result[1], 4);
    });

    test("each with trailing block", fn() {
        let sum = 0;
        let arr = [1, 2, 3];
        arr.each |x| sum = sum + x end;
        assert_eq(sum, 6);
    });

    test("find with trailing block", fn() {
        let result = [10, 20, 30].find |x| x > 15 end;
        assert_eq(result, 20);
    });

    test("any? with trailing block", fn() {
        let result = [1, 2, 3].any? |x| x > 2 end;
        assert_eq(result, true);
    });

    test("all? with trailing block", fn() {
        let result = [2, 4, 6].all? |x| x % 2 == 0 end;
        assert_eq(result, true);
    });

    test("sort with trailing block", fn() {
        let result = [3, 1, 2].sort |a, b| a - b end;
        assert_eq(result[0], 1);
        assert_eq(result[1], 2);
        assert_eq(result[2], 3);
    });
});

describe("Trailing Block - Int Methods", fn() {
    test("times with trailing block", fn() {
        let count = 0;
        3.times |i| count = count + i end;
        assert_eq(count, 3);  # 0 + 1 + 2
    });

    test("upto with trailing block after parens", fn() {
        let sum = 0;
        1.upto(3) |i| sum = sum + i end;
        assert_eq(sum, 6);  # 1 + 2 + 3
    });

    test("downto with trailing block after parens", fn() {
        let result = [];
        3.downto(1) |i| result.push(i) end;
        assert_eq(result[0], 3);
        assert_eq(result[1], 2);
        assert_eq(result[2], 1);
    });
});

describe("Trailing Block - Hash Methods", fn() {
    test("each with trailing block on hash", fn() {
        let h = {"a" => 1, "b" => 2};
        let keys = [];
        h.each |pair| keys.push(pair[0]) end;
        assert_eq(len(keys), 2);
    });

    test("map on hash with trailing block", fn() {
        let h = {"a" => 1, "b" => 2};
        let result = h.map |k, v| [k, v * 10] end;
        assert_eq(result["a"], 10);
        assert_eq(result["b"], 20);
    });

    test("filter on hash with trailing block", fn() {
        let h = {"a" => 1, "b" => 2, "c" => 3};
        let result = h.filter |pair| pair[1] > 1 end;
        assert_eq(len(result), 2);
    });
});

describe("Trailing Block - Assignment & Chaining", fn() {
    test("assign trailing block result to variable", fn() {
        let doubled = [1, 2, 3].map |x| x * 2 end;
        assert_eq(doubled[0], 2);
        assert_eq(doubled[2], 6);
    });

    test("trailing block on variable", fn() {
        let items = [10, 20, 30];
        let result = items.map |x| x + 1 end;
        assert_eq(result[0], 11);
        assert_eq(result[1], 21);
        assert_eq(result[2], 31);
    });

    test("multi-statement trailing block body", fn() {
        let result = [1, 2, 3].map |x|
            let doubled = x * 2;
            doubled + 1
        end;
        assert_eq(result[0], 3);
        assert_eq(result[1], 5);
        assert_eq(result[2], 7);
    });
});

describe("Trailing Block - Equivalent to Parenthesized Form", fn() {
    test("map: trailing block equals parenthesized", fn() {
        let a = [1, 2, 3].map |x| x * 3 end;
        let b = [1, 2, 3].map(|x| x * 3);
        assert_eq(a[0], b[0]);
        assert_eq(a[1], b[1]);
        assert_eq(a[2], b[2]);
    });

    test("filter: trailing block equals parenthesized", fn() {
        let a = [1, 2, 3, 4].filter |x| x > 2 end;
        let b = [1, 2, 3, 4].filter(|x| x > 2);
        assert_eq(len(a), len(b));
        assert_eq(a[0], b[0]);
    });

    test("times: trailing block equals parenthesized", fn() {
        let a = 0;
        let b = 0;
        3.times |i| a = a + i end;
        3.times(|i| b = b + i);
        assert_eq(a, b);
    });
});
