// ============================================================================
// Multiline Strings Test Suite
// ============================================================================

describe("Multiline Strings", fn() {
    test("basic multiline string", fn() {
        let text = [[hello
world]];
        assert_contains(text, "hello");
        assert_contains(text, "world");
    });

    test("multiline string preserves newlines", fn() {
        let text = [[line1
line2]];
        assert_contains(text, "\n");
    });

    test("multiline string is raw (no escape processing)", fn() {
        let text = [[hello\nworld]];
        assert_contains(text, "\\n");
    });

    test("multiline string with multiple lines", fn() {
        let text = [[first
second
third]];
        assert_contains(text, "first");
        assert_contains(text, "second");
        assert_contains(text, "third");
    });

    test("multiline string with single bracket", fn() {
        let text = [[contains ] single bracket]];
        assert_contains(text, "]");
    });

    test("empty multiline string", fn() {
        let text = [[]];
        assert_eq(text, "");
    });

    test("multiline string with leading/trailing whitespace", fn() {
        let text = [[
    indented line
]];
        assert_contains(text, "indented line");
    });

    test("multiline string in hash value", fn() {
        let h = {
            "description" => [[This is a
multiline description.]]
        };
        assert_contains(h["description"], "This is a");
        assert_contains(h["description"], "multiline description");
    });
});
