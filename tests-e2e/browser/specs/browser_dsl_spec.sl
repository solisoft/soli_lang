# The browser DSL itself, driven against the fixture app.
#
# Run with: soli test tests-e2e/browser/specs --browser

describe("visiting pages", fn() {
    test("renders the server's HTML", fn() {
        visit("/")
        assert_text("Welcome to the browser fixture.")
        assert_selector("#page-title")
    })

    test("reports where the browser is", fn() {
        visit("/about")
        assert_page_path("/about")
        assert_eq(page_path(), "/about")
    })

    test("reads the document title from the layout", fn() {
        visit("/about")
        assert_eq(page_title(), "About")
    })

    test("page_text sees rendered text, not markup", fn() {
        visit("/about")
        assert_contains(page_text(), "somewhere to go")
        assert_not(page_text().includes?("<h1"))
    })

    test("page_html exposes the markup", fn() {
        visit("/about")
        assert_contains(page_html(), "<h1")
    })
})

describe("executing JavaScript", fn() {
    test("sees content that only exists after script runs", fn() {
        # The whole reason this feature exists: an HTTP-level test cannot see
        # #from-script at all, because the server never sent it.
        visit("/dynamic")
        assert_selector("#from-script")
        assert_text("Rendered by JavaScript")
    })

    test("evaluate returns values from the page", fn() {
        visit("/dynamic")
        assert_eq(evaluate("document.getElementById('static').textContent"),
                  "Rendered on the server.")
    })

    test("evaluate round-trips non-string values", fn() {
        visit("/")
        assert_eq(evaluate("2 + 3"), 5)
        assert_eq(evaluate("[1, 2, 3].length"), 3)
        assert_eq(evaluate("true"), true)
    })
})

describe("clicking", fn() {
    test("a real click runs the page's handler", fn() {
        visit("/")
        click("#counter")
        assert_text("Clicked 1")
    })

    test("clicking twice accumulates", fn() {
        visit("/")
        click("#counter")
        click("#counter")
        assert_text("Clicked 2")
    })

    test("links can be clicked by their text", fn() {
        visit("/")
        click_link("About")
        assert_page_path("/about")
    })

    test("navigating away and back works", fn() {
        visit("/")
        click_link("About")
        assert_page_path("/about")
        click_link("Home")
        assert_page_path("/")
        assert_text("Welcome to the browser fixture.")
    })
})

describe("forms", fn() {
    test("fills fields, submits, and the server receives the values", fn() {
        visit("/form")
        fill_in("#name", "Ada Lovelace")
        select_option("#role", "admin")
        check("#subscribe")
        click_button("Save")

        assert_text("Received name=Ada Lovelace role=admin subscribed=true")
    })

    test("fields can be addressed by their label", fn() {
        visit("/form")
        fill_in("Full name", "Grace Hopper")
        click_button("Save")
        assert_text("Received name=Grace Hopper")
    })

    test("an unchecked box submits nothing", fn() {
        visit("/form")
        fill_in("#name", "Alan")
        click_button("Save")
        assert_text("Received name=Alan role= subscribed=false")
    })

    test("a select can be chosen by its visible text", fn() {
        visit("/form")
        fill_in("#name", "Edsger")
        select_option("#role", "Editor")
        click_button("Save")
        assert_text("Received name=Edsger role=editor subscribed=false")
    })

    test("uncheck reverses check", fn() {
        visit("/form")
        fill_in("#name", "Barbara")
        check("#subscribe")
        uncheck("#subscribe")
        click_button("Save")
        assert_text("Received name=Barbara role= subscribed=false")
    })
})

describe("waiting", fn() {
    test("assertions wait for content that arrives late", fn() {
        # #late appears 400ms after load. Asserting without waiting would be a
        # race, so the assertions wait by default.
        visit("/slow")
        assert_selector("#late")
        assert_text("Arrived late")
    })

    test("wait_for blocks until the element exists", fn() {
        visit("/slow")
        wait_for("#late")
        assert_eq(evaluate("document.getElementById('late').textContent"),
                  "Arrived late")
    })

    test("wait_for_text blocks until the text appears", fn() {
        visit("/slow")
        wait_for_text("Arrived late")
        assert_selector("#late")
    })

    test("absence is checked immediately, not waited out", fn() {
        visit("/")
        assert_no_selector("#late")
        assert_no_text("Arrived late")
    })
})

describe("page errors", fn() {
    test("a clean page reports none", fn() {
        visit("/")
        assert_no_page_errors()
    })

    test("an uncaught exception is captured", fn() {
        visit("/broken")
        wait_for_text("Broken")

        let errors = page_errors()
        assert_gt(errors.length(), 0)
        assert_contains(str(errors), "fixture failure")
    })

    test("errors do not leak into the next test", fn() {
        # Guards the per-test reset: without it, the previous test's exception
        # would fail this one.
        visit("/")
        assert_no_page_errors()
    })
})

describe("screenshots", fn() {
    test("writes a PNG to the given path", fn() {
        visit("/")
        let path = "/tmp/soli_browser_spec_shot.png"
        screenshot(path)
        assert(File.exists(path))
        assert_gt(File.size(path), 100)
        File.delete(path)
    })
})
