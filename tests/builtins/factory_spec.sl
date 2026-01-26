// ============================================================================
// Factory Functions Test Suite
// ============================================================================

describe("Factory Functions", fn() {
    test("Factory.define() and Factory.create() work", fn() {
        let user_data = hash();
        user_data["name"] = "Test User";
        user_data["email"] = "test@example.com";
        Factory.define("user", user_data);

        let user = Factory.create("user");
        assert_eq(user["name"], "Test User");
        assert_eq(user["email"], "test@example.com");
    });

    test("Factory.create_with() allows overrides", fn() {
        let base = hash();
        base["name"] = "Default";
        base["active"] = true;
        Factory.define("item", base);

        let overrides = hash();
        overrides["name"] = "Custom";
        let item = Factory.create_with("item", overrides);
        assert_eq(item["name"], "Custom");
        assert_eq(item["active"], true);
    });

    test("Factory.create_list() creates multiple", fn() {
        let data = hash();
        data["type"] = "widget";
        Factory.define("widget", data);

        let widgets = Factory.create_list("widget", 3);
        assert_eq(len(widgets), 3);
    });

    test("Factory.sequence() generates incrementing numbers", fn() {
        let seq1 = Factory.sequence("counter");
        let seq2 = Factory.sequence("counter");
        assert_eq(seq2, seq1 + 1);
    });

    test("Factory can create different types", fn() {
        let type_a = hash();
        type_a["type"] = "A";
        Factory.define("type_a", type_a);

        let type_b = hash();
        type_b["type"] = "B";
        Factory.define("type_b", type_b);

        let a = Factory.create("type_a");
        let b = Factory.create("type_b");
        assert_eq(a["type"], "A");
        assert_eq(b["type"], "B");
    });

    test("Factory.create() with id", fn() {
        let data = hash();
        data["name"] = "Item";
        Factory.define("item_with_id", data);

        let item = Factory.create_with_id("item_with_id");
        assert(item["id"] > 0);
    });
});
