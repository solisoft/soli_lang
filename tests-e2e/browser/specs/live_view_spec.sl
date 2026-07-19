# LiveView's client (src/live/client.js), exercised in a real browser.
#
# 904 lines of websocket transport, line-splice diffing, DOM morphing and
# reconnection logic. Its only test until now ran the morph functions inside
# JSDOM — which cannot open a websocket, cannot lay anything out, and so cannot
# tell whether the thing works end to end. Here the server really renders, the
# patch really crosses a socket, and the DOM really morphs.

describe("liveview", fn() {
    test("connects and renders the component", fn() {
        visit("/live")
        # The component only appears once the socket has connected and the
        # server has pushed its first render.
        wait_for("#count")
        assert_eq(evaluate("document.getElementById('count').textContent"), "count=0")
    })

    test("a click round-trips through the server and patches the DOM", fn() {
        visit("/live")
        wait_for("#count")

        click("#inc")
        wait_for_text("count=1")
        assert_eq(evaluate("document.getElementById('count').textContent"), "count=1")
    })

    test("state accumulates across events", fn() {
        visit("/live")
        wait_for("#count")

        click("#inc")
        wait_for_text("count=1")
        click("#inc")
        wait_for_text("count=2")
        click("#inc")
        wait_for_text("count=3")

        assert_eq(evaluate("document.getElementById('count').textContent"), "count=3")
    })

    test("events can decrease state as well", fn() {
        visit("/live")
        wait_for("#count")

        click("#inc")
        wait_for_text("count=1")
        click("#dec")
        # `evaluate` reads the DOM as it is right now, so the round trip has to
        # be waited for first — unlike `assert_text`, which waits for you.
        wait_for_text("count=0")

        assert_eq(evaluate("document.getElementById('count').textContent"), "count=0")
    })

    test("the patch morphs nodes instead of replacing them", fn() {
        # The distinguishing property of a morph: the same DOM node survives.
        # Replacing the subtree would give a different node and lose focus,
        # caret and any client-side widget state inside it.
        visit("/live")
        wait_for("#count")
        evaluate("window.__node = document.getElementById('count')")

        click("#inc")
        wait_for_text("count=1")

        assert_eq(evaluate("window.__node === document.getElementById('count')"), true)
    })

    test("soli-ignore islands are left alone by patches", fn() {
        visit("/live")
        wait_for("#island")
        evaluate("document.getElementById('island').textContent = 'client-owned'")

        click("#inc")
        wait_for_text("count=1")

        # The server's render still says "untouched"; the morph must not have
        # reverted the client's edit.
        assert_eq(evaluate("document.getElementById('island').textContent"),
                  "client-owned")
    })

    test("a change event carries the field's value to the server", fn() {
        visit("/live")
        wait_for("#echo")

        fill_in("#echo", "hello")
        wait_for_text("typed=hello")
        assert_eq(evaluate("document.getElementById('typed').textContent"), "typed=hello")
    })

    test("the focused field keeps focus across a patch", fn() {
        # This is what the morph exists for: a patch that lands while the user
        # is typing must not steal the caret.
        visit("/live")
        wait_for("#echo")

        fill_in("#echo", "abc")
        wait_for_text("typed=abc")

        assert_eq(evaluate("document.activeElement.id"), "echo")
    })

    test("connecting and patching leaves no JavaScript errors", fn() {
        visit("/live")
        wait_for("#count")
        click("#inc")
        wait_for_text("count=1")
        assert_no_page_errors()
    })
})
