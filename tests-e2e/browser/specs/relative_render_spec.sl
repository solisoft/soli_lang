# A bare view name inside an action means the controller's own directory.
describe("render with a bare name", fn() {
    test("render(\"relative\") from pages#relative finds pages/relative", fn() {
        # A top-level relative view exists too: the controller's own one wins.
        visit("/relative")
        assert_text("Relative view")
        assert_no_text("Top-level relative view")
    })

    test("a name with no pages/ file falls back to the top-level view", fn() {
        visit("/relative-fallback")
        assert_text("Standalone view")
    })

    test("a name found nowhere says both places it looked", fn() {
        visit("/relative-missing")
        assert_text("Template 'nowhere' not found")
        assert_text("looked for 'pages/nowhere' first")
    })
})
