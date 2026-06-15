describe("match_classify", fn() {
    test("zero",   fn() { assert_eq(classify(0),    "zero"); });
    test("small",  fn() { assert_eq(classify(5),    "positive-small"); });
    test("large",  fn() { assert_eq(classify(42),   "positive-large"); });
    test("neg",    fn() { assert_eq(classify(-3),   "negative"); });
    test("string", fn() { assert_eq(classify("hi"), "other"); });
    test("hash",   fn() { assert_eq(classify({}),   "other"); });
});
