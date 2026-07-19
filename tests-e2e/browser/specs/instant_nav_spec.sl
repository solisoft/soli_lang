# Instant-nav (src/serve/nav.js), exercised in a real browser.
#
# This is the framework's own frontend, ~587 lines of link interception, body
# swapping, head merging and history handling that until now had no test at
# all — jsdom cannot meaningfully run it, and an HTTP-level spec never loads it.
#
# The trick throughout: set a sentinel on `window`, then navigate. A real page
# load destroys the JavaScript context and the sentinel with it, so its survival
# is direct evidence that the navigation was a body swap rather than a reload.

describe("instant-nav", fn() {
    test("is injected into every page", fn() {
        visit("/")
        assert_selector("script[src^='/__soli/nav.js']")
    })

    test("navigating a link does not reload the page", fn() {
        visit("/")
        evaluate("window.__sentinel = 'alive'")

        click_link("About")
        assert_page_path("/about")

        # Survived the navigation, so the JavaScript context was never torn
        # down — the body was swapped in place.
        assert_eq(evaluate("window.__sentinel"), "alive")
    })

    test("swaps in the new page's content", fn() {
        visit("/")
        assert_text("Welcome to the browser fixture.")

        click_link("About")
        assert_text("somewhere to go")
        assert_no_text("Welcome to the browser fixture.")
    })

    test("merges the new page's title into the head", fn() {
        visit("/")
        assert_eq(page_title(), "Home")

        click_link("About")
        assert_page_path("/about")
        assert_eq(page_title(), "About")
    })

    test("pushes history, so back returns to the previous page", fn() {
        visit("/")
        click_link("About")
        assert_page_path("/about")

        evaluate("history.back()")
        assert_page_path("/")
        assert_text("Welcome to the browser fixture.")
    })

    test("forward returns again after going back", fn() {
        visit("/")
        click_link("About")
        assert_page_path("/about")

        evaluate("history.back()")
        assert_page_path("/")

        evaluate("history.forward()")
        assert_page_path("/about")
    })

    test("emits soli:load on a swapped navigation", fn() {
        visit("/")
        evaluate("window.__loads = 0; document.addEventListener('soli:load', function () { window.__loads++; })")

        click_link("About")
        assert_page_path("/about")

        assert_gt(evaluate("window.__loads"), 0)
    })

    test("emits soli:visit when a navigation starts", fn() {
        visit("/")
        evaluate("window.__visits = 0; document.addEventListener('soli:visit', function () { window.__visits++; })")

        click_link("About")
        assert_page_path("/about")

        assert_gt(evaluate("window.__visits"), 0)
    })

    test("data-no-nav opts a link out, forcing a real load", fn() {
        visit("/")
        evaluate("window.__sentinel = 'alive'")

        click("#nav-about-reload")
        assert_page_path("/about")

        # The opposite of the swap test: a real load builds a fresh context, so
        # the sentinel must be gone.
        assert_null(evaluate("window.__sentinel"))
    })

    test("script in the swapped-in page still runs", fn() {
        # A body swap that does not re-run inline scripts would silently break
        # every page whose content is built client-side.
        visit("/")
        click_link("Dynamic")
        assert_page_path("/dynamic")
        assert_selector("#from-script")
        assert_text("Rendered by JavaScript")
    })

    test("navigation leaves no JavaScript errors behind", fn() {
        visit("/")
        click_link("About")
        click_link("Home")
        click_link("Dynamic")
        assert_no_page_errors()
    })
})
