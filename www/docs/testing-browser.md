# Browser Testing

Soli's request specs test what the server sends. Browser specs test what the
user gets: a real headless Chrome loads the page, runs its JavaScript, and you
drive it with the same kind of helpers you already use for HTTP.

```soli
describe("checkout", fn() {
  test("a customer can place an order", fn() {
    visit("/cart")
    fill_in("Coupon", "SAVE10")
    click_button("Apply")

    assert_text("Discount applied")
    assert_no_page_errors()
  })
})
```

No Node, no npm, no Playwright, no browser download. Soli speaks the Chrome
DevTools protocol from the binary itself — the only requirement is a
Chromium-family browser on the machine.

## Running browser specs

Browser specs are opt-in, because they need a browser and cost seconds rather
than milliseconds:

```bash
soli test --browser              # run everything, browser specs included
soli test --headed               # same, but watch it happen in a window
```

A spec counts as a browser spec when a directory called `browser` appears
anywhere in its path:

```
tests/
  users_spec.sl              # always runs
  browser/
    checkout_spec.sl         # only with --browser
```

Plain `soli test` sets those aside and says so:

```
Skipping 2 browser spec(s) — add --browser to run them.
```

That is the point of the split: a suite with no browser installed still runs
green, and nobody pays for a browser they did not ask for.

### Choosing a browser

Soli looks for `google-chrome`, `google-chrome-stable`, `chromium`,
`chromium-browser`, `microsoft-edge` and `brave-browser` on `PATH` (and the
usual `/Applications` paths on macOS). To use something else:

```bash
SOLI_CHROME_PATH=/opt/chrome/chrome soli test --browser
```

If nothing is found, `--browser` fails immediately with what it looked for —
rather than thirty seconds later on the first `visit()`.

## Navigating

```soli
visit("/posts")                  # relative to this worker's test server
visit("https://example.com")     # absolute URLs work too

page_path()                      # "/posts?page=2"
page_url()                       # "http://127.0.0.1:41731/posts?page=2"
page_title()                     # the <title>
page_text()                      # visible text, as the user sees it
page_html()                      # full markup
```

`visit` returns once the document has finished loading, so any script the page
runs on boot has already run.

## Interacting

```soli
click("#save")                   # CSS selector
click_link("Edit")               # a link by its text
click_button("Save")             # a button by its label or value

fill_in("#title", "Hello")       # by selector
fill_in("Full name", "Ada")      # by label text
fill_in("email", "a@b.c")        # by name or placeholder

select_option("#role", "admin")  # by value or visible text
check("#agree")
uncheck("#agree")
choose("#plan_pro")
press("Enter")
press("Alt+d")                   # chords: Alt, Ctrl, Shift, Meta/Cmd
```

Selectors are resolved leniently: a CSS selector first, then a matching
`<label>`, then a field's `name` or `placeholder`. So you can write what you
see on the page rather than what the markup happens to call it.

Clicks are **real input events** dispatched at the element's position, not
`element.click()`. An element covered by an overlay is not clickable in a
browser, and it is not clickable here either — which is the behaviour you want
a test to have.

## Asserting

```soli
assert_text("Saved")             # visible text contains this
assert_no_text("Error")
assert_selector("#toast")        # element is present
assert_no_selector(".error")
assert_page_path("/posts/1")
assert_no_page_errors()          # no uncaught exceptions or console.error
```

**Positive assertions wait.** `assert_text("Saved")` polls until the text
appears or the timeout expires, so a spec does not have to guess how long a
round trip takes. Negative assertions (`assert_no_text`, `assert_no_selector`)
check immediately — waiting for something to *stay* absent would only slow
every passing test down.

Override the wait per call:

```soli
assert_text("Report ready", {"timeout": 30})   # seconds
wait_for("#chart", {"timeout": 30})
```

The default is 10 seconds.

### Waiting explicitly

```soli
wait_for("#toast")               # until the element exists
wait_for_text("Saved")           # until the text appears
```

You need these when the next thing you do is not an assertion — `evaluate`
reads the DOM as it is *right now* and does not wait:

```soli
click("#increment")
wait_for_text("count=1")                                    # wait first…
assert_eq(evaluate("document.title"), "Counter — 1")        # …then read
```

## Escape hatches

```soli
evaluate("window.appVersion")            # any expression, value comes back
evaluate("localStorage.getItem('tok')")
screenshot("/tmp/checkout.png")          # PNG of the current view
page_errors()                            # array of captured JS errors
```

`evaluate` preserves JavaScript's types: a string stays a string even when it
looks like a number, so `evaluate("el.textContent")` on `<span>0</span>` gives
you `"0"`, not `0`.

## Signing in

The browser shares the request helpers' cookie jar, so the sign-in you already
have keeps working:

```soli
before_each(fn() {
  login("ada@example.com", "secret")     # a real POST /login
})

test("the dashboard greets the user", fn() {
  visit("/dashboard")                     # arrives already signed in
  assert_text("Welcome back, Ada")
})
```

Cookies flow both ways: a sign-in performed *in the browser* is visible to a
later `get()` and to `signed_in()`.

> `as_user(id)` with a single argument only sets a thread-local and does **not**
> authenticate a real request. Use the two-argument form, `as_user(7, {"role":
> "admin"})`, which writes the session store and sets the cookie — that one
> carries into the browser.

## What resets between tests

Each test starts with a clean page-error list and empty `sessionStorage` and
`localStorage`. Without that, a panel one test collapsed would stay collapsed
for the rest of the suite and results would depend on test order.

Cookies are **not** cleared automatically, matching the existing convention for
request specs: sign out explicitly when a test needs a guest.

```soli
before_each(fn() {
  logout()
})
```

The browser itself is reused for the whole worker rather than relaunched per
test, which is why the reset exists at all — and why a suite of thirty browser
specs takes seconds rather than a minute.

## In CI

```yaml
- uses: browser-actions/setup-chrome@v1
  with:
    chrome-version: stable

- run: soli test tests/browser --browser --no-coverage
```

Browser specs parallelise like any other: each test worker gets its own server
*and* its own browser, so `--jobs 3` means three browsers.

## Troubleshooting

**"Browser helpers need a browser: run this spec with `soli test --browser`."**
The spec called `visit()` without the flag. Either add it, or move the spec out
of a `browser/` directory if it does not need one.

**A click fails with "found nothing matching".** The helper waited the full
timeout and never saw the element. Check it is actually rendered (`page_html()`)
and not hidden — a zero-size element has no position to click.

**Flaky-looking failures.** Almost always a missing wait around `evaluate`.
Assertions wait; raw `evaluate` does not.

**Errors from a page you expected to be clean.** `page_errors()` shows what was
captured. Note that `assert_no_page_errors()` covers uncaught exceptions and
`console.error`, not warnings.
