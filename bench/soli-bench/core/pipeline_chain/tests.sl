describe("pipeline_chain", fn() {
    test("piped_steps doubles 1..10", fn() {
        assert_eq(piped_steps, [2, 4, 6, 8, 10, 12, 14, 16, 18, 20]);
    });

    test("piped_sum filters > 5 and sums", fn() {
        // After doubling: 2,4,6,8,10,12,14,16,18,20 — filter > 5: 6,8,10,12,14,16,18,20
        // Sum: 6+8+10+12+14+16+18+20 = 104
        assert_eq(piped_sum, 104);
    });
});
