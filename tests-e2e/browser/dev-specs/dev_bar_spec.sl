# The dev bar (src/serve/dev_bar.rs), exercised in a real browser.
#
# The bar is roughly a thousand lines of inline JavaScript — panel toggles,
# a flame chart, a request list, a keybinding — that ships to every developer
# running `soli serve --dev` and had no test of any kind.
#
# The bar only exists under `--dev`, and test servers otherwise run in
# production mode so specs exercise the bytecode VM that serves production.
# That is why this file lives in `dev-specs/` rather than `specs/`: CI runs
# this directory in its own invocation with `SOLI_TEST_SERVER_DEV=1`, which
# puts `--dev` back for that server only. Running it from `specs/` asserts
# against a page with no bar.

describe("dev bar", fn() {
    test("is injected into dev-mode pages", fn() {
        visit("/")
        assert_selector("#__solidev_bar")
    })

    test("reports the route that served the page", fn() {
        visit("/")
        assert_text("pages#index")
    })

    test("reports the request method and status", fn() {
        visit("/about")
        assert_text("GET /about")
        assert_text("200")
    })

    test("the close button hides the bar and reveals the opener", fn() {
        visit("/")
        assert_selector("#__solidev_close")

        click("#__solidev_close")
        assert_eq(evaluate("document.getElementById('__solidev_bar').style.display"), "none")
        assert_ne(evaluate("document.getElementById('__solidev_show').style.display"), "none")
    })

    test("the opener brings it back", fn() {
        visit("/")
        click("#__solidev_close")
        assert_eq(evaluate("document.getElementById('__solidev_bar').style.display"), "none")

        click("#__solidev_show")
        assert_ne(evaluate("document.getElementById('__solidev_bar').style.display"), "none")
    })

    test("Alt+D toggles the bar", fn() {
        visit("/")
        assert_ne(evaluate("document.getElementById('__solidev_bar').style.display"), "none")

        press("Alt+d")
        assert_eq(evaluate("document.getElementById('__solidev_bar').style.display"), "none")

        press("Alt+d")
        assert_ne(evaluate("document.getElementById('__solidev_bar').style.display"), "none")
    })

    test("hidden state survives a navigation", fn() {
        # The bar remembers itself through sessionStorage; a developer who hid
        # it does not want it back on the next page.
        visit("/")
        press("Alt+d")
        assert_eq(evaluate("document.getElementById('__solidev_bar').style.display"), "none")

        visit("/about")
        assert_eq(evaluate("document.getElementById('__solidev_bar').style.display"), "none")

        # Leave it visible so the ordering of tests cannot matter.
        press("Alt+d")
    })

    test("pads the body so the bar never covers page content", fn() {
        visit("/")
        let padding = evaluate("parseInt(document.body.style.paddingBottom || '0', 10)")
        assert_gt(padding, 0)
    })

    test("renders without JavaScript errors", fn() {
        visit("/")
        assert_no_page_errors()
    })
})
