# Browser Tests Without Node: Soli Drives Chrome Itself

Your request specs are green. Every route returns the right status, every template renders the right locals, the N+1 guard is clean. Then a user reports that the checkout button does nothing.

The button was never broken on the server. It was broken in the browser — and no HTTP-level test can tell you that, because an HTTP test never runs the page's JavaScript. It reads the bytes the server sent and stops there.

**`soli test --browser`** closes that gap. It drives a real headless Chrome over the Chrome DevTools protocol, spoken by the `soli` binary itself. No Node. No npm. No Playwright. Nothing added to your project at all — the only requirement is a Chromium-family browser, which your CI runner already has.

<figure style="margin:1.5rem auto;max-width:1024px;">
  <img src="/images/blog/browser-testing.svg" width="1024" height="576" alt="Two side-by-side test runs against the same route. The HTTP spec sees only the server's HTML, so an element built by JavaScript is missing and the assertion fails. The browser spec runs the page in a real headless Chrome, the element appears, and the assertion passes. Below, the soli binary speaks the DevTools protocol straight to Chrome, with node_modules, npm ci and playwright install crossed out." style="display:block;width:100%;height:auto;border-radius:12px;border:1px solid #30363d;background:#0b0d0f;">
  <figcaption style="text-align:center;color:#8b949e;font-size:0.875rem;margin-top:0.5rem;">Same route, same assertion. One spec never runs the script that builds half the page.</figcaption>
</figure>

## What a browser spec looks like

The same DSL you already write, with a browser behind it:

```soli
describe("checkout", fn() {
  test("a customer can apply a coupon", fn() {
    visit("/cart")
    fill_in("Coupon", "SAVE10")
    click_button("Apply")

    assert_text("Discount applied")
    assert_no_page_errors()
  })
})
```

Nothing here is a new paradigm. `visit` is `get` for a browser. `assert_text` is `assert_contains` for a page. If you have written a request spec in Soli, you can write this one.

## Why not just wrap Playwright?

Because Soli ships as [a single binary with no package manager](/docs/blog/no-build-no-dependency), and a test runner that quietly requires Node, a `package.json`, a lockfile and a 300 MB browser download would have thrown that away for one feature.

So the driver is Rust, and it speaks CDP directly:

| | Playwright wrapper | Soli's driver |
|---|---|---|
| Install | Node + npm + `playwright install` | nothing |
| Added to your repo | `package.json`, lockfile, `node_modules` | nothing |
| CI setup | Node toolchain, browser download, cache | one `setup-chrome` step |
| New Rust dependencies | — | **zero** (`tungstenite` was already there) |
| Browsers | Chromium, Firefox, WebKit | Chromium family |

That last row is a real trade, and worth being straight about: this is not a cross-browser testing tool. It drives Chrome, Chromium, Edge or Brave. If your bug is Safari-shaped, this will not find it.

What you get in exchange is that `soli test --browser` works on a fresh clone with nothing installed but a browser most machines already have.

## Real clicks, not `element.click()`

A lot of "browser testing" is really JavaScript testing wearing a costume: the helper calls `element.click()`, which fires a synthetic event on a node whether or not a human could have reached it.

Soli dispatches actual input events at the element's measured position:

```soli
click("#save")     # scrolls it into view, then presses and releases at its centre
```

The difference shows up exactly when it matters. A button covered by a modal overlay, a sticky footer sitting on top of a form, an element with `width: 0` — all of those are unclickable for a user, and all of them fail here. `element.click()` reports success on every one.

Fields resolve the way you think about them, not the way the markup happens to be written:

```soli
fill_in("#title", "Hello")        # CSS selector
fill_in("Full name", "Ada")       # the <label> text
fill_in("email", "a@b.c")         # the name or placeholder
```

## Assertions that wait

The single biggest source of flake in browser tests is asserting one millisecond before the thing arrives. The usual fix is a `sleep` sprinkled where it last helped.

Positive assertions in Soli wait on their own:

```soli
click("#save")
assert_text("Saved")              # polls until it appears, or fails at 10s
```

Negative assertions do **not** wait, deliberately. `assert_no_text("Error")` checking for ten seconds that something stays absent would add ten seconds to every passing test. Absence is checked once, now.

The one place you still have to think is the escape hatch, because `evaluate` reads the DOM as it is at that instant:

```soli
click("#increment")
wait_for_text("count=1")                                 # wait explicitly…
assert_eq(evaluate("document.title"), "Counter — 1")     # …then read
```

## Signing in still works

Browser specs share the cookie jar with the request helpers, in both directions. The sign-in you already have keeps working unchanged:

```soli
before_each(fn() {
  login("ada@example.com", "secret")     # a real POST /login
})

test("the dashboard greets the user", fn() {
  visit("/dashboard")                    # arrives already signed in
  assert_text("Welcome back, Ada")
})
```

And it flows the other way too: if a test signs in by *clicking the form*, a later `get()` and `signed_in()` both see it. Without that write-back the two halves of a spec would quietly disagree about who is logged in — which is the sort of bug that costs an afternoon.

## Opt-in, so your suite stays fast

Browser specs cost seconds where request specs cost milliseconds, and they need software the rest of your suite does not. So they are opt-in: a spec is a browser spec when a `browser` directory appears anywhere in its path.

```
tests/
  users_spec.sl              # always runs
  browser/
    checkout_spec.sl         # only with --browser
```

```bash
soli test                    # Skipping 1 browser spec(s) — add --browser to run them.
soli test --browser          # run everything
soli test --headed           # watch it happen in a window
```

A machine with no browser still runs the suite green. And `--browser` checks for a browser up front and fails naming what it looked for, rather than timing out thirty seconds later inside a worker.

Each test worker gets its own server **and** its own browser, so `--jobs 3` means three browsers working in parallel. The browser is launched once per worker and reused across tests — which is why a suite of thirty browser specs finishes in seconds rather than a minute.

## We pointed it at our own frontend first

Soli ships around 2,500 lines of JavaScript: the LiveView client, instant-nav, the dev bar. Until now that code had either no test at all, or a JSDOM unit test — and JSDOM cannot open a websocket or lay anything out, so it could not tell you whether any of it actually worked.

The first thing we did with the new driver was cover it. Those specs now run in CI on every push, against a real browser:

```soli
test("navigating a link does not reload the page", fn() {
  visit("/")
  evaluate("window.__sentinel = 'alive'")

  click_link("About")
  assert_page_path("/about")

  # A real page load destroys the JavaScript context. The sentinel surviving
  # is direct evidence that instant-nav swapped the body in place.
  assert_eq(evaluate("window.__sentinel"), "alive")
})
```

Writing them was worth it before a single line shipped. Two examples:

**They found an isolation bug in the feature itself.** The first dev-bar spec hid the bar, and every later spec started with it already hidden — `sessionStorage` survives navigation by design, so state leaked from one test into the next and results depended on test order. Browser specs now clear `sessionStorage` and `localStorage` between tests. (Cookies are deliberately left alone, matching the existing convention that specs manage sign-in themselves.)

**They proved they can actually fail.** A test that has never failed is a claim, not evidence. So we deleted the line in `nav.js` that merges the new page's `<title>` and re-ran: exactly one spec went red — "merges the new page's title into the head" — while the other 24 assertions stayed green. Precise enough to point at the regression, not just report that something broke.

## What this is not

- **Not cross-browser.** Chromium family only.
- **Not visual regression testing.** `screenshot(path)` writes a PNG; comparing them is your call.
- **Not as deep as Capybara.** Rails' system-test ecosystem has years of matchers, drivers and edge cases behind it. This covers the flows most apps actually need, honestly and without ceremony.
- **Not a replacement for request specs.** It is the second tier, not the first. Test at the HTTP layer by default — it is faster, simpler and more stable. Reach for a browser when the behaviour genuinely lives in the DOM.

## Try it

```bash
mkdir -p tests/browser
```

```soli
# tests/browser/smoke_spec.sl
describe("the app boots in a browser", fn() {
  test("the home page renders and throws nothing", fn() {
    visit("/")
    assert_no_page_errors()
  })
})
```

```bash
soli test --browser
```

That two-line spec is worth more than it looks: it will fail the moment any page starts throwing in the browser, which is the class of bug that otherwise reaches users first.

Full reference: [Browser Testing](/docs/testing-browser).
