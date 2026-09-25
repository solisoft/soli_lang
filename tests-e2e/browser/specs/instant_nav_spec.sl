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

# Morph mode: a layout that says <meta name="soli-nav" content="morph"> is
# patched into the next page instead of replaced, so what did not change stays
# the same DOM node. A property set on a node is the evidence: a swap builds a
# new node and the property is gone.
describe("instant-nav morph", fn() {
    test("keeps unchanged nodes and changes the rest", fn() {
        visit("/morph/a")
        evaluate("document.getElementById('sidebar').__kept = 'yes'")

        click_link("Morph B")
        assert_page_path("/morph/b")
        assert_text("The second morphing page.")
        assert_no_text("The first morphing page.")
        assert_eq(page_title(), "Morph B")
        assert_eq(evaluate("document.getElementById('sidebar').__kept"), "yes")
    })

    test("keeps typed input, open details and scroll position", fn() {
        visit("/morph/a")
        evaluate("document.getElementById('sidebar-search').value = 'draft'")
        evaluate("document.getElementById('sidebar-more').open = true")
        evaluate("document.getElementById('sidebar').scrollTop = 60")

        click_link("Morph B")
        assert_page_path("/morph/b")
        assert_eq(evaluate("document.getElementById('sidebar-search').value"), "draft")
        assert_eq(evaluate("document.getElementById('sidebar-more').open"), true)
        assert_eq(evaluate("document.getElementById('sidebar').scrollTop"), 60)
    })

    test("runs the new page's scripts but not the unchanged layout script again", fn() {
        visit("/morph/a")
        assert_eq(evaluate("window.__layoutRuns"), 1)

        click_link("Morph B")
        assert_text("B script ran")
        assert_eq(evaluate("window.__layoutRuns"), 1)
    })

    test("a page without the meta is still swapped in", fn() {
        visit("/morph/a")
        evaluate("window.__sentinel = 'alive'")

        click_link("About")
        assert_page_path("/about")
        assert_text("somewhere to go")
        assert_eq(evaluate("window.__sentinel"), "alive")
    })

    test("back and forth leaves no JavaScript errors behind", fn() {
        visit("/morph/a")
        click_link("Morph B")
        assert_page_path("/morph/b")
        click_link("Morph A")
        assert_page_path("/morph/a")
        evaluate("history.back()")
        assert_page_path("/morph/b")
        assert_no_page_errors()
    })
})
