# Views

Views handle the presentation layer of your application.

## Template Syntax

SoliLang uses ERB-style templates. Tag types:

| Tag | Output | When to use |
|-----|--------|-------------|
| `<%= expr %>` | HTML-escaped | **Default.** Anything that came from user input, the database, params, etc. |
| `<%- expr %>` | Raw, unescaped | Trusted HTML you've already produced — partials, `Markdown.to_safe_html(...)` output, `partial(...)` results. |
| `<% stmt %>` | No output | Statements, control flow, `let` bindings. |
| `<%= yield %>` | Layout insertion | Only valid inside a layout — marks where rendered content is spliced in. |
| `<%= yield "name" %>` | Named content | Splices content captured by a `content_for "name"` block. Empty when nothing was captured. |
| `<% content_for "name" do %> … <% end %>` | Nothing (captured) | Push a named block from a view/partial into the layout — per-page `<head>` scripts, sidebars, etc. |
| `<%# comment %>` | Nothing (stripped) | Developer comments — never sent to the browser. Single-line and multi-line both work. |

> `<%== expr %>` was removed (SEC-023). It decoded HTML entities and emitted the result raw, which silently re-created `<script>` from `&lt;script&gt;` whenever a value had been round-tripped through escape-encoded storage. Use `<%= html_unescape(expr) %>` for entity-decoded but escaped output, or `<%- expr %>` for trusted raw HTML.

### Output Variables

```erb
<h1><%= title %></h1>
<p>Hello, <%= name %>!</p>
<p>Count: <%= count %></p>
```

```erb
<!-- Raw output: skip escaping (only for HTML you trust) -->
<article><%- rendered_markdown %></article>
<%- partial("shared/nav") %>
```

For user-generated Markdown, render with `Markdown.to_safe_html(...)` before using raw output.

Controller instance fields are exposed as bare locals, but you can also reference them with the same `@` prefix you used in the controller — in a view, `<%= @title %>` falls back to the `title` local, so both forms render the same value. This works through member access and method calls too (`<%= @user.name %>`, `<%= @comments.length %>`). An `@`-name with no matching local renders as empty (`nil`), like any other absent template local.

```erb
<h1><%= @title %></h1>   <!-- mirrors the controller -->
<h1><%= title %></h1>    <!-- bare local, identical result -->
```

### Control Flow

```erb
<% if user_logged_in %>
  <p>Welcome back!</p>
<% else %>
  <p>Please log in.</p>
<% end %>

<% for item in items %>
  <li><%= item.name %></li>
<% end %>

<% if count > 0 %>
  <p>You have <%= count %> items.</p>
<% end %>
```

### Helper Functions

Templates have access to several built-in helper functions:

```erb
<%= h(user_input) %>          <!-- HTML escape -->
<%= html_escape(content) %>   <!-- Same as h() -->
```

## Template Helper Functions

The following helper functions are automatically available in all templates.

### Utility Functions

#### range(start, end)

Creates an array of numbers for iteration.

```erb
<% for i in range(1, 5) %>
  <p>Item <%= i %></p>
<% end %>
```

#### public_path(path)

Returns a versioned public asset path with cache-busting hash.

```erb
<link rel="stylesheet" href="<%= public_path("css/app.css") %>">
<script src="<%= public_path("js/app.js") %>"></script>
```

Output: `/css/app.css?v=a1b2c3d4...`

### HTML Functions

#### html_escape(string) / h(string)

Escapes HTML special characters to prevent XSS attacks. Use `h()` for content embedded in element bodies (between tags).

```erb
<p><%= html_escape(user_input) %></p>
<p><%= h(user_input) %></p>
```

#### attr(string)

Escapes a string for safe interpolation inside an HTML attribute value (between quotes). Encodes `"`, `'`, `<`, `>`, and `&`. Use this — not `h()` — when interpolating into attributes, since attribute context allows unquoted/single-quoted values that `h()` does not cover.

```erb
<a title="<%= attr(post.title) %>">Read</a>
<input value="<%= attr(form_value) %>">
```

#### j(string)

JavaScript-string escape for embedding inside an inline `<script>` block. Escapes backslashes, single/double quotes, and `<`/`>`/`&` so a payload cannot break out of the string literal or close the surrounding `</script>` tag.

```erb
<script>
  const user = "<%= j(current_user.name) %>";
  const next = "<%= j(redirect_url) %>";
</script>
```

#### url(string)

Percent-encodes a string for safe use as a URL query-parameter value or path segment. Only unreserved characters (`A-Z`, `a-z`, `0-9`, `-`, `_`, `.`, `~`) are passed through; everything else is `%`-encoded.

```erb
<a href="/search?q=<%= url(query) %>">Search</a>
<a href="/users/<%= url(user.slug) %>">Profile</a>
```

> **Pick the helper that matches the output context.** `h()` is for element bodies, `attr()` for attribute values, `j()` for JS string literals, `url()` for URL query/path parts. Using the wrong one — e.g. `h()` inside a `<script>` — leaves XSS gaps.

#### html_unescape(string)

Unescapes HTML entities back to their original characters.

```erb
<%= html_unescape("&lt;p&gt;") %>  <!-- Output: <p> -->
```

#### strip_html(string)

Removes all HTML tags from a string.

```erb
<%= strip_html("<p>Hello <b>World</b></p>") %>  <!-- Output: Hello World -->
```

#### sanitize_html(string)

Removes dangerous HTML tags and attributes while preserving safe content.

```erb
<%= sanitize_html(user_content) %>
```

#### substring(string, start, end)

Extracts a portion of a string (useful for truncating content).

```erb
<%= substring(post["content"], 0, 100) %>...
```

### Request-Context Functions

Read fields off the current request directly — no need to plumb them through the view data hash. Available in every template (views, layouts, partials). They return `null` when called outside an active request (e.g. from a unit test).

| Helper              | Returns                                                              |
|---------------------|----------------------------------------------------------------------|
| `current_path()`    | Request pathname, e.g. `"/users"`. `null` outside a request.         |
| `current_method()`  | HTTP method, e.g. `"GET"`. `null` outside a request.                 |
| `current_path?(p)`  | `true` if the current path equals `p` exactly. Use for active links. |

**Active-link pattern:**

```erb
<nav>
  <a href="/users" class="<%= current_path?("/users") ? "active" : "" %>">Users</a>
  <a href="/posts" class="<%= current_path?("/posts") ? "active" : "" %>">Posts</a>
</nav>

<p>You are viewing <%= current_path() %> (<%= current_method() %>).</p>
```

For prefix matches (e.g. any path under `/users`), compose with `current_path().starts_with("/users")`.

### Hover Preload

Soli auto-injects a small `<script>` tag into every HTML response that listens for `mouseover` on links and adds a `<link rel="prefetch" as="document">` so the browser warms its document-prefetch cache before the user clicks. The script is served at `/__soli/prefetch.js` — an external file, not inline, so apps with strict CSP (no `unsafe-inline`) work out of the box. Browsers send these requests with a `Sec-Purpose: prefetch` header (older browsers `Purpose: prefetch`), so your backend can log or differentiate them.

**What you get for free:**

- Same-origin GET links only; cross-origin, `mailto:`, `tel:`, and in-page `#fragment` links are skipped.
- 65 ms hover debounce so fly-over hovers don't waste bandwidth.
- Skipped on `navigator.connection.saveData` or 2G networks.
- Each URL prefetched at most once per page load.
- Works on touch devices: `touchstart` triggers an immediate prefetch (strong intent).

**Opt out per link:**

```erb
<a href="/heavy-report" data-no-prefetch>Heavy Report</a>

<!-- Or on a container to cover everything inside -->
<section data-no-prefetch>
  <a href="/a">A</a>
  <a href="/b">B</a>
</section>
```

Also skipped: `<a data-method="post">` (Rails-style non-GET link helpers).

**Opt out globally:**

```bash
SOLI_PREFETCH=off soli serve .
```

`off`, `false`, `0`, and `no` all disable. Anything else (or unset) keeps it on.

**Caching defaults:** every HTML response from a controller — whether you call `render(...)` explicitly or let an OOP controller action auto-render its matching view — carries two headers automatically so the prefetch actually delivers instant navigation:

- **`ETag: W/"<16-hex>"`** — a content-derived *weak* validator (FNV-1a over the rendered body, computed after live-reload/prefetch script injection so it reflects the exact bytes on the wire). Weak, not strong, so it survives the content-encoding transforms a CDN applies (Cloudflare and friends strip strong ETags when they re-compress; weak ones pass through).
- **`Cache-Control: private, no-cache`** — the browser may cache, shared caches (CDN, reverse proxy) may not; the entry must be revalidated before reuse.

On the actual click, the browser sends `If-None-Match: W/"<etag>"`. If the rendered body would be identical, the framework short-circuits to **`304 Not Modified`** with just the validator headers — no body re-transmission. Result: the prefetched body is consumed as the navigation response, so the click feels instant.

**Behind a CDN (Cloudflare, etc.):** that `304` round-trip only works if the conditional `GET` reaches the origin. Some edge configurations don't relay it — a "Cache Everything" rule, edge revalidation, or HTML-transform features (Rocket Loader, Email Obfuscation, Mirage) that need the full body — so the click re-downloads the whole page and the prefetch is wasted. To make the feature robust regardless of edge config, Soli detects the `Sec-Purpose: prefetch` request and answers it with **`Cache-Control: private, max-age=30`** instead of `no-cache`. The prefetched HTML is then *fresh* in the browser's own (private) cache for a short window, so the click reuses it directly — **no conditional GET, so the CDN never gets a vote.** Normal navigations are unaffected: they don't carry the prefetch header, so they still get `private, no-cache`. Tune the window with `SOLI_PREFETCH_TTL` (seconds, clamped 1–300; default 30), or set it low if your pages change second-to-second.

> If your CDN rewrites `Cache-Control` for the browser (Cloudflare's **Browser Cache TTL** set to anything other than *Respect Existing Headers*), it can override this `max-age`. Leave Browser Cache TTL on *Respect Existing Headers* for the app hostname.

**Override per response** when the defaults don't fit. Set your own `Cache-Control` (and optionally `ETag`) in the response headers and the framework defaults step aside:

```soli
def downloads
  # One-shot download — never reuse; always re-fetch.
  return {
    "status": 200,
    "headers": {"Cache-Control": "no-store", "Content-Type": "text/csv"},
    "body": csv_bytes
  }
end
```

**Gotchas:**

- **`Cache-Control: no-store`** (explicit, in your response) disables the cache entirely — prefetch still fires but the browser re-fetches on click. Use for sensitive one-shot pages.
- **POST/PUT/DELETE responses** aren't cached regardless, so nothing special is needed there.
- Per-request **`Set-Cookie`** headers (flash messages, CSRF token rotation) can cause some browsers to ignore the cache entry even with good `Cache-Control`. In that case the prefetch still warms the TCP/TLS connection and any server-side caches, so the click is at least faster.

### Instant Navigation

On top of hover preloading, Soli auto-injects an instant-navigation script (served at `/__soli/nav.js`) into every HTML response. It intercepts same-origin link clicks, fetches the target page in the background, and **swaps `<body>` in place** — merging the new page's `<title>`, stylesheets, and `meta` tags — while managing the URL bar with `pushState`. The result is Turbo-Drive-style navigation: your CSS and JS stay loaded, Alpine and htmx stay booted, and clicks render near-instantly, all while the app remains plain server-rendered HTML.

When instant navigation is on, it **takes over hover prefetching**: a JavaScript `fetch()` can't consume `<link rel="prefetch">` entries (browser cache partitioning), so `nav.js` prefetches hovered links into its own in-memory cache with the same ergonomics (65 ms debounce, `touchstart`, `data-no-prefetch`, save-data/2G skip). Its prefetch requests carry `Purpose: prefetch`, so all the [Hover Preload](#hover-preload) caching machinery — `SOLI_PREFETCH_TTL`, the ETag/304 revalidation, the CDN notes — applies unchanged. `SOLI_PREFETCH=off` disables the hover warming without disabling click swapping.

**Which clicks are intercepted?** Only plain left-clicks on same-origin GET links. Everything else falls through to the browser:

- Modifier keys (`Cmd`/`Ctrl`/`Shift`/`Alt` — open-in-new-tab intent), middle/right clicks.
- Cross-origin links, `mailto:`/`tel:`/`javascript:`, `target="_blank"` (any target ≠ `_self`), `download` links.
- `<a data-method="post">` and friends (non-GET link helpers).
- Links carrying any `hx-*`/`data-hx-*` attribute or inside `[hx-boost]` — htmx owns those.
- Anything that already called `preventDefault()` (Alpine `@click.prevent`, your own handlers).
- Same-page `#fragment` links (native anchor scroll).

**Opt out:**

```erb
<!-- Per link, or per container -->
<a href="/legacy-page" data-no-nav>Legacy page</a>
<section data-no-nav> ... </section>

<!-- Per page (both the current page and any page navigated to) -->
<meta name="soli-nav" content="off">
```

```bash
# Globally — restores plain hover preloading (prefetch.js)
SOLI_NAV=off soli serve .
```

**Lifecycle events** fire on `document` so you can hook in:

| Event | Cancelable | When |
|-------|-----------|------|
| `soli:visit` | yes — cancel to force a full navigation | Before a visit starts; `detail.url` |
| `soli:before-render` | yes — cancel to force a full navigation | After fetch, before the swap; `detail.newDocument` |
| `soli:load` | no | After every swap — the re-initialization hook |

**`DOMContentLoaded` keeps working.** The event itself fires once per document and never again after a swap — but inline scripts re-executed by a visit routinely register `DOMContentLoaded`/`load` listeners, so the framework *replays* them: once the event has already fired, registering a listener for it invokes the listener immediately (the same semantics as jQuery's `.ready()`). Your existing init code — sliders, lightboxes, anything wrapped in `DOMContentLoaded` — works after swaps without changes. The same replay covers `alpine:init`/`alpine:initialized`: a page-specific bundle first executed by a swap that registers components with `document.addEventListener("alpine:init", () => Alpine.data(...))` works unchanged. For code in **external** scripts (which execute once per tab, not per visit), hook `soli:load` to re-initialize per navigation:

```html
<script>
  function initWidgets() { /* ... */ }
  document.addEventListener("soli:load", initWidgets);  // fires after every swap
  initWidgets();                                        // first full load
</script>
```

**Script semantics after a swap:** inline `<script>` tags in the new body re-execute on every visit (that's what page-specific init wants). External `<script src>` tags execute **once per URL** for the lifetime of the browser tab — so `alpine.min.js` and `htmx.min.js` in your layout never double-evaluate. Scripts run **sequentially in document order**, each external awaited before the next script executes — the same guarantee the parser gives on a full load, so an inline `tailwind.config = {...}` right after the Tailwind CDN script still finds `tailwind` defined. Only after the whole chain settles does the framework call `Alpine.initTree(document.body)` and `htmx.process(document.body)` — so Alpine components whose `x-data` scope is registered by a page-specific bundle initialize correctly, and `hx-*` attributes in the new body just work.

**Persistent elements (`data-soli-permanent`):** the body swap tears down the old DOM and builds the new page fresh — fine for server-rendered content, but it destroys live client-side widgets that aren't reflected in the server HTML (a map with its rendered tiles, a playing `<video>`, a rich-text editor with unsaved state). Tag such an element with `data-soli-permanent` and an `id`, and the framework lifts the **live** element out of the old body and grafts it over the matching placeholder in the new one — untouched, never reparsed — so the widget keeps running across the navigation with no teardown, no re-initialization, no flicker:

```erb
<div id="gather-map" data-soli-permanent></div>
```

The element persists only between pages that **both** declare it (same `id` + `data-soli-permanent`); navigate to a page without the matching placeholder and it's discarded with the old body. Any inline `<script>` *inside* a permanent element is carried over live and is **not** re-executed on the swap (it already ran). This is the right tool when re-initializing on `soli:load` would mean an expensive rebuild — prefer it over the re-init hook for maps and media players.

**History and scrolling:** back/forward are handled via `popstate` with a refetch — which the ETag machinery answers with a cheap `304`, served from the browser's HTTP cache. Scroll position is restored on back-navigation; forward visits scroll to the top (or to the `#fragment` target if the link had one).

**View Transitions (opt-in):** add the same meta tag Turbo uses and swaps animate with the [View Transition API](https://developer.mozilla.org/en-US/docs/Web/API/View_Transition_API) in supporting browsers:

```html
<meta name="view-transition" content="same-origin">
```

**Graceful degradation:** non-HTML responses (downloads, JSON), fetch failures, and redirects that leave the origin all fall back to a normal full navigation automatically. (Alpine `x-teleport` pages swap fine — the swap destroys the old Alpine tree, which runs each teleport's cleanup, before replacing the body and re-initializing.) Error pages (404/500) that render HTML are swapped in like any other page, with the URL updated to the final response URL.

### DateTime Functions

These functions help you work with dates and times in templates.

#### datetime_now()

Returns the current Unix timestamp (UTC).

```erb
<p>Current timestamp: <%= datetime_now() %></p>
```

#### datetime_format(timestamp, format)

Formats a Unix timestamp using strftime format specifiers.

**Parameters:**
- `timestamp` (Int|String) - Unix timestamp or date string
- `format` (String) - strftime format string

Common format specifiers:
- `%Y` - 4-digit year (2024)
- `%m` - 2-digit month (01-12)
- `%d` - 2-digit day (01-31)
- `%H` - 24-hour hour (00-23)
- `%M` - Minute (00-59)
- `%S` - Second (00-59)
- `%B` - Full month name (January)
- `%b` - Abbreviated month name (Jan)
- `%A` - Full weekday name (Monday)
- `%a` - Abbreviated weekday name (Mon)

```erb
<p>Created: <%= datetime_format(item["created_at"], "%Y-%m-%d") %></p>
<p>Date: <%= datetime_format(item["created_at"], "%B %d, %Y") %></p>
<p>Time: <%= datetime_format(item["created_at"], "%H:%M:%S") %></p>
<p>Today: <%= datetime_format(datetime_now(), "%A, %B %d, %Y") %></p>
```

#### datetime_parse(string)

Parses a date string to a Unix timestamp.

Supported formats:
- RFC 3339: `"2024-01-15T10:30:00Z"`
- ISO datetime: `"2024-01-15T10:30:00"` or `"2024-01-15 10:30:00"`
- ISO date: `"2024-01-15"`

```erb
<% let ts = datetime_parse("2024-01-15") %>
<p>Parsed: <%= datetime_format(ts, "%B %d, %Y") %></p>
```

Returns `null` if parsing fails.

#### datetime_add_days(timestamp, days)

Adds (or subtracts) days from a timestamp.

```erb
<% let tomorrow = datetime_add_days(datetime_now(), 1) %>
<p>Tomorrow: <%= datetime_format(tomorrow, "%Y-%m-%d") %></p>

<% let last_week = datetime_add_days(datetime_now(), -7) %>
<p>Last week: <%= datetime_format(last_week, "%Y-%m-%d") %></p>
```

#### datetime_add_hours(timestamp, hours)

Adds (or subtracts) hours from a timestamp.

```erb
<% let in_two_hours = datetime_add_hours(datetime_now(), 2) %>
<p>In 2 hours: <%= datetime_format(in_two_hours, "%H:%M") %></p>
```

#### datetime_diff(timestamp1, timestamp2)

Returns the difference between two timestamps in seconds.

```erb
<% let diff = datetime_diff(item["created_at"], datetime_now()) %>
<p>Age in seconds: <%= diff %></p>
```

#### time_ago(timestamp)

Returns a human-readable relative time string.

**Parameters:**
- `timestamp` (Int|String) - Unix timestamp or date string

```erb
<p>Updated: <%= time_ago(item["updated_at"]) %></p>
<p>Posted: <%= time_ago(post["created_at"]) %></p>
```

Output examples:
- "5 seconds ago"
- "2 minutes ago"
- "1 hour ago"
- "3 days ago"
- "2 weeks ago"
- "1 month ago"
- "2 years ago"

### Complete DateTime Example

```erb
<article>
  <h1><%= post["title"] %></h1>

  <div class="meta">
    <span>Published: <%= datetime_format(post["created_at"], "%B %d, %Y") %></span>
    <span>(<%= time_ago(post["created_at"]) %>)</span>
  </div>

  <% if post["updated_at"] != post["created_at"] %>
    <div class="updated">
      Last updated: <%= time_ago(post["updated_at"]) %>
    </div>
  <% end %>

  <div class="content">
    <%= post["content"] %>
  </div>

  <footer>
    <p>Copyright <%= datetime_format(datetime_now(), "%Y") %></p>
  </footer>
</article>
```

### Internationalization (I18n) Functions

These functions help you build multilingual applications.

#### locale()

Returns the current locale code.

```erb
<p>Current language: <%= locale() %></p>
```

#### set_locale(code)

Sets the current locale for translations and formatting.

```erb
<% set_locale("fr") %>
<p>Now using French: <%= locale() %></p>
```

#### t(key, params)

Translates a key using the current locale. Supports interpolation with parameters.

```erb
<h1><%= t("welcome.title") %></h1>
<p><%= t("welcome.greeting", {"name": user["name"]}) %></p>
```

Translation files are stored in `config/locales/`:

```yaml
# config/locales/en.yml
en:
 welcome:
  title: "Welcome"
  greeting: "Hello, %{name}!"

# config/locales/fr.yml
fr:
 welcome:
  title: "Bienvenue"
  greeting: "Bonjour, %{name}!"
```

#### l(timestamp, format)

Localizes a date/time according to the current locale.

**Parameters:**
- `timestamp` (Int) - Unix timestamp in seconds
- `format` (String) - Format name or strftime string

**Named formats:**
- `"short"` - Short date (e.g., "01/15/2024" or "15/01/2024")
- `"long"` - Long date (e.g., "January 15, 2024" or "15 janvier 2024")
- `"full"` - Full date with weekday (e.g., "Monday, January 15, 2024")
- `"time"` - Time only (e.g., "10:30" or "10h30")
- `"datetime"` - Date and time combined
- Or any strftime format string (e.g., "%d %B %Y")

```erb
<p>Date: <%= l(timestamp, "short") %></p>       <!-- en: 01/15/2024 | fr: 15/01/2024 -->
<p>Date: <%= l(timestamp, "long") %></p>        <!-- en: January 15, 2024 | fr: 15 janvier 2024 -->
<p>Date: <%= l(timestamp, "full") %></p>        <!-- en: Monday, January 15, 2024 | fr: lundi 15 janvier 2024 -->
<p>Time: <%= l(timestamp, "time") %></p>        <!-- en: 10:30 AM | fr: 10h30 -->
<p>Custom: <%= l(timestamp, "%d %b %Y") %></p>  <!-- en: 15 Jan 2024 | fr: 15 janv. 2024 -->
```

### Complete I18n Example

```erb
<% set_locale(user["preferred_locale"]) %>

<html lang="<%= locale() %>">
<head>
  <title><%= t("site.title") %></title>
</head>
<body>
  <h1><%= t("products.header") %></h1>

  <% for product in products %>
    <div class="product">
      <h2><%= product["name"] %></h2>
      <p class="price"><%= currency(product["price"]) %></p>
      <p class="stock"><%= t("products.in_stock", {"count": product["quantity"]}) %></p>
      <p class="updated"><%= t("products.last_updated") %>: <%= l(product["updated_at"], "long") %></p>
    </div>
  <% end %>

  <footer>
    <p><%= t("footer.copyright", {"year": datetime_format(datetime_now(), "%Y")}) %></p>
  </footer>
</body>
</html>
```

---

## Application Helpers

When you create a new application with `soli new`, a starter helper file is generated at `app/helpers/application_helper.sl`. These helpers complement the built-in functions and are automatically available in all templates.

### truncate(text, length, suffix)

Truncates text to a maximum length, appending a suffix (default: "...").

**Parameters:**
- `text` (String) - The text to truncate
- `length` (Int) - Maximum length before truncation
- `suffix` (String, optional) - Suffix to append (default: "...")

```erb
<p><%= truncate(post["content"], 100) %></p>
<!-- "This is a very long article that continues..." -->

<p><%= truncate(title, 50, " [more]") %></p>
<!-- "This is a long title that gets cut [more]" -->
```

### number_with_delimiter(number, delimiter)

Formats a number with thousands separators for display. The integer part is
grouped in threes; a leading sign and any fractional part are preserved.

**Parameters:**
- `number` (Int|Float|String) - The number to format
- `delimiter` (String, optional) - The separator inserted between groups (default `","`)

```erb
<p>Views: <%= number_with_delimiter(1234567) %></p>
<!-- "1,234,567" -->

<p>Population: <%= number_with_delimiter(population) %></p>

<!-- Override with a specific delimiter -->
<p>Custom: <%= number_with_delimiter(1234567, "'") %></p>
<!-- "1'234'567" -->
```

### currency(amount, symbol)

Formats a number as currency. Locale-aware by default - automatically uses the correct symbol, delimiter, and symbol position for the current locale.

**Parameters:**
- `amount` (Int|Float) - The amount to format
- `symbol` (String, optional) - Currency symbol (auto-detected from locale if not specified)

```erb
<p>Price: <%= currency(1000) %></p>
<!-- en: "$1,000" | fr: "1 000 €" | de: "1.000 €" | ja: "¥1,000" -->

<p>Total: <%= currency(order["total"]) %></p>

<!-- Override with specific symbol -->
<p>Pounds: <%= currency(1234, "£") %></p>
<!-- "£1,234" -->
```

**Supported locales:**
| Locale | Symbol | Format |
|--------|--------|--------|
| en (default) | $ | $1,234 |
| fr | € | 1 234 € |
| de, es, it, pt | € | 1.234 € |
| ja, zh | ¥ | ¥1,234 |
| ru | ₽ | 1 234 ₽ |

### pluralize(count, singular, plural)

Returns a pluralized string based on count.

**Parameters:**
- `count` (Int) - The count to check
- `singular` (String) - Singular form of the word
- `plural` (String, optional) - Plural form (default: singular + "s")

```erb
<p><%= pluralize(1, "item") %></p>
<!-- "1 item" -->

<p><%= pluralize(5, "item") %></p>
<!-- "5 items" -->

<p><%= pluralize(cart_count, "item", "items") %></p>

<p><%= pluralize(person_count, "person", "people") %></p>
<!-- "1 person" or "3 people" -->

<p><%= pluralize(comment_count, "comment") %> on this post</p>
<!-- "12 comments on this post" -->
```

### capitalize(text)

Capitalizes the first letter of a string.

**Parameters:**
- `text` (String) - The text to capitalize

```erb
<p><%= capitalize("hello world") %></p>
<!-- "Hello world" -->

<p><%= capitalize(user["status"]) %></p>
<!-- "Active" (if status was "active") -->
```

### link_to(text, url, css_class)

Generates an HTML anchor tag with proper escaping to prevent XSS attacks.

**Parameters:**
- `text` (String) - Link text (will be HTML escaped)
- `url` (String) - Link URL (will be HTML escaped)
- `css_class` (String, optional) - CSS class(es) to add

```erb
<%= link_to("Home", "/") %>
<!-- <a href="/">Home</a> -->

<%= link_to("View Profile", "/users/" + user["id"]) %>
<!-- <a href="/users/123">View Profile</a> -->

<%= link_to("Edit", "/posts/" + post["id"] + "/edit", "btn btn-primary") %>
<!-- <a href="/posts/456/edit" class="btn btn-primary">Edit</a> -->

<nav>
  <%= link_to("Dashboard", "/dashboard", "nav-link") %>
  <%= link_to("Settings", "/settings", "nav-link") %>
  <%= link_to("Logout", "/logout", "nav-link text-danger") %>
</nav>
```

### slugify(text)

Converts text to a URL-friendly slug by lowercasing, replacing spaces and special characters with hyphens.

**Parameters:**
- `text` (String) - The text to convert to a slug

```erb
<%= slugify("Hello World!") %>
<!-- "hello-world" -->

<%= slugify("My Blog Post Title") %>
<!-- "my-blog-post-title" -->

<%= slugify("Café & Restaurant") %>
<!-- "cafe-restaurant" -->

<a href="/posts/<%= slugify(post["title"]) %>">
  <%= post["title"] %>
</a>
<!-- <a href="/posts/my-awesome-post">My Awesome Post</a> -->
```

### paginate(pagination, options)

Renders a pagination navigation control from the result of `Model.paginate(...)` (or the inner `pagination` hash).

**Parameters:**
- `pagination` (Hash) — the pagination metadata (`{page, per, total, total_pages}`)
- `options` (Hash, optional)
  - `"path"`: base URL/path for links (falls back to current request path)
  - `"param"`: query parameter name (default `"page"`)
  - `"window"`: how many page numbers to show around the current page (default `2`)
  - `"class"`: extra CSS class(es) on the `<nav>`

Returns raw HTML (use `<%- %>`).

```erb
<%- paginate(@pagination, {"path": users_path()}) %>
```

Generates semantic `<nav class="pagination">` with Previous/Next and page links. Preserves other query parameters when possible.

### component(name, data)

See the full [Components](#components) section for the dedicated `component()` helper and recommended file layout.

### Complete Application Helpers Example

```erb
<div class="product-card">
  <h2><%= capitalize(product["name"]) %></h2>

  <p class="description">
    <%= truncate(product["description"], 150) %>
  </p>

  <div class="pricing">
    <span class="price"><%= currency(product["price"]) %></span>
    <span class="stock"><%= pluralize(product["stock"], "unit") %> available</span>
  </div>

  <div class="stats">
    <span><%= number_with_delimiter(product["views"]) %> views</span>
    <span><%= pluralize(product["review_count"], "review") %></span>
  </div>

  <div class="actions">
    <%= link_to("View Details", "/products/" + product["id"], "btn btn-secondary") %>
    <%= link_to("Add to Cart", "/cart/add/" + product["id"], "btn btn-primary") %>
  </div>
</div>
```

### Customizing Application Helpers

You can add your own helpers by editing `app/helpers/application_helper.sl`:

```soli
# app/helpers/application_helper.sl

# ... existing helpers ...

# Custom helper: Format a phone number
def format_phone(number)
  digits = replace(number, "[^0-9]", "")
  if len(digits) == 10
    return "(" + substring(digits, 0, 3) + ") " + substring(digits, 3, 6) + "-" + substring(digits, 6, 10)
  end
  number
end

# Custom helper: Generate a mailto link
def mail_to(email, text = null)
  if text == null
    text = email
  end
  "<a href=\"mailto:" + html_escape(email) + "\">" + html_escape(text) + "</a>"
end
```

---

## Layouts

Wrap views in a common layout:

```erb
<!-- app/views/layouts/application.html.slv -->
<!DOCTYPE html>
<html>
<head>
  <title><%= title %></title>
</head>
<body>
  <nav>
    <a href="/">Home</a>
    <a href="/about">About</a>
  </nav>

  <main>
    <%= yield %>
  </main>

  <footer>
    &copy; 2024 My App
  </footer>
</body>
</html>
```

Use layout with render:

```soli
def index
  render("home/index", {
    "title": "Welcome"
  }, "layouts/application")
end
```

## Named Content with `content_for`

A plain `<%= yield %>` gives the layout exactly one insertion point. When a
page needs to inject content *elsewhere* in the layout — a page-specific
`<script>` in the `<head>`, a sidebar, extra meta tags — capture it in the
view with `content_for` and read it back in the layout with a named `yield`:

```erb
<!-- app/views/reports/show.html.slv -->
<% content_for "head" do %>
  <script src="/js/chart.js"></script>
<% end %>

<h1><%= report.title %></h1>
```

```erb
<!-- app/views/layouts/application.html.slv -->
<html>
<head>
  <title><%= title %></title>
  <%= yield "head" %>
</head>
<body>
  <%= yield %>
</body>
</html>
```

Semantics:

- **Views and partials can capture.** A `content_for` block inside a partial
  registers into the same store as the view that rendered it.
- **Repeated captures append.** Two `content_for "head"` blocks concatenate
  in document order (Rails semantics).
- **Missing names render empty.** `<%= yield "head" %>` emits nothing when no
  view captured `"head"` — no guard needed, no error.
- **No double-escaping.** The captured fragment is already-rendered template
  output: interpolations inside the block were escaped at capture time, and
  the named `yield` splices the result raw — exactly like the main `yield`.
- **Names are string literals** (`"head"` or `'head'`), so the layout's
  insertion points are known at parse time.

`content_for("name")` also works as a read-form in the layout, equivalent to
`yield "name"`:

```erb
<%= content_for("head") %>
```

To wrap a section in markup only when something was captured, use the
`content_for?` predicate:

```erb
<% if content_for?("sidebar") %>
  <aside class="sidebar">
    <%= yield "sidebar" %>
  </aside>
<% end %>
```

`content_for?("name")` returns `true` only when a non-empty capture exists
for that name.

## Partials

Reuse template fragments:

```erb
<!-- app/views/partials/user_card.html.slv -->
<div class="user-card">
  <h3><%= user.name %></h3>
  <p><%= user.email %></p>
</div>
```

Include partials — use `partial(...)` as the short alias for `render_partial(...)`:

```erb
<%= partial("partials/user_card", {"user": current_user}) %>

<% for user in users %>
  <%= partial("partials/user_card", {"user": user}) %>
<% end %>

<!-- render_partial(...) still works and is identical -->
<%= render_partial("partials/user_card", {"user": current_user}) %>
```

### Inherited instance variables

Partials **and** components inherit the current controller's `@instance`
variables, just like the main view — so a partial can read `@current_user` or
`@posts` without threading them through the locals hash:

```erb
<%# controller: @current_user = User.find(id) %>
<%= partial("shared/avatar") %>   <%# the partial reads @current_user directly %>
```

An explicit local always wins over an inherited ivar of the same name, and
framework internals (`req`, `params`, `session`, `headers`) are never injected.

### The `locals` hash

Partials expose their entire context hash as a `locals` variable, mirroring
Rails' `local_assigns`. You don't need this for everyday partials — bare
identifiers work and read better:

```erb
<!-- Caller -->
<%= partial("user_card", {"user": current_user, "size": "lg"}) %>

<!-- Inside the partial -->
<div class="user-card user-card--<%= size %>">
  <h3><%= user.name %></h3>
</div>
```

Reach for `locals[...]` when a key would collide with a reserved word or a
builtin function. For example, `class` is reserved and bare `class` fails to
parse; `type` is a global builtin, so bare `type` resolves to the function
and auto-calls with zero args. In both cases, bracket access returns the
string the caller passed:

```erb
<!-- Caller -->
<%= partial("shared/icon", {"name": "bell", "class": "h-6 w-6"}) %>

<!-- Inside shared/_icon.html.slv -->
<svg class="<%= locals["class"] %>" data-icon="<%= name %>">…</svg>
```

Missing keys return `null`, so the usual `.nil?` pattern applies without
guards:

```erb
<% let css = locals["class"].nil? ? "h-5 w-5" : locals["class"] %>
```

`locals` is always defined, even when the partial is rendered without a
data hash — in that case it's an empty hash, so `locals[anything]` is safe
and returns `null`.

## Components

Soli ships a lightweight component system for reusable UI primitives. The `component()` helper gives you a dedicated convention that is clearer than scattering everything under `partials/`.

While `partial()` is great for one-off extracted fragments, `component()` signals that you're rendering a designed, named building block (cards, rows, badges, empty states, stat blocks, etc.).

### The `component()` helper

```erb
<%- component("stats_card", {
  "label": "Active Users",
  "value": 1243,
  "trend": "+18%"
}) %>
```

Resolution rules:

- If the name contains `/` or `.`, it is treated as a relative path from `app/views/`.
- Otherwise Soli looks for `components/<name>.html.slv`.

Data works exactly like partials: values are exposed as bare variables. Use `locals["key"]` only when the key collides with a builtin or reserved word. Components also inherit the controller's `@instance` variables (see [Inherited instance variables](#inherited-instance-variables)).

The function-call form passes only a data hash — it has no captured body, so a `<%= yield %>` inside the component needs an explicit `"content"` (or `"body"`) key. To pass block content into the default slot, use the [block form](#block-syntax-and-slots).

### Component files

Component files live under `app/views/components/`. They do **not** need a leading underscore (unlike partials).

```erb
<!-- app/views/components/stats_card.html.slv -->
<div class="stats-card">
  <div class="label"><%= label %></div>
  <div class="value"><%= number_with_delimiter(value) %></div>
  <% if trend %>
    <div class="trend"><%= trend %></div>
  <% end %>
</div>
```

### Subdirectories

Group related components naturally:

```
app/views/components/
├── card.html.slv
├── badge.html.slv
├── table/
│   ├── header.html.slv
│   └── row.html.slv
└── form/
    └── field.html.slv
```

```erb
<%= component("table/row", {"record": post}) %>
<%= component("form/field", {"name": "email", "type": "email"}) %>
```

### Block syntax and slots

To pass body content, use the block form — the block body is captured and
exposed to the component as the default slot. Props come from **named
arguments** (`title: "…"`) or an explicit **parenthesized hash**; a paren-less
`component "card", { … } do` does *not* pass the hash (`{` isn't a command
argument), so reach for named args or parentheses:

```erb
<%- component "card", title: "Important Notice" do %>
  <p>This is the default slot body.</p>
<%- end %>

<%# equivalent, with an explicit hash %>
<%- component("card", { "title": "Important Notice" }) do %>
  <p>This is the default slot body.</p>
<%- end %>
```

Inside the component template, use `<%= content %>` or `<%= yield %>` for the default slot:

```erb
<div class="card">
  <h3><%= title %></h3>
  <div class="body">
    <%= yield %>
  </div>
</div>
```

For named slots, use `content_for` inside the block and `yield "name"` (or `content_for "name"`) in the template. This works because component bodies participate in the content capture system:

```erb
<%- component "card", title: "With header" do %>
  <% content_for "header" do %>
    <strong>Header content</strong>
  <% end %>
  Body
<%- end %>
```

Or, more ergonomically, bind a **slot-builder** with `do |c|` and call
`c.slot("name")` — it desugars to the same `content_for` capture, so it's just
a clearer way to spell named slots (call `c.slot(...)` at the top level of the
block):

```erb
<%- component "card", title: "With header" do |c| %>
  <%- c.slot("header") do %>
    <strong>Header content</strong>
  <%- end %>
  Body
<%- end %>
```

Either form feeds the same named `yield` in the component template:

```erb
<div class="card">
  <header><%= yield "header" %></header>
  <div class="body"><%= content %></div>
</div>
```

Named slots are great for things like headers, footers, or sidebars within a component.

### Rendering a collection

Pass a `"collection"` to render the component once per item. Each item is bound
to a local named after the component (override with `"as"`), and you also get
`<as>_index` (0-based) and `<as>_counter` (1-based). Any other keys pass through
to every item:

```erb
<%- component("post_card", { "collection": posts }) %>

<%# custom local name + a shared prop for every item %>
<%- component("post_card", { "collection": posts, "as": "post", "compact": true }) %>
```

```erb
<!-- app/views/components/post_card.html.slv -->
<article class="post" data-n="<%= post_counter %>">
  <h2><%= h(post["title"]) %></h2>
</article>
```

### Components vs partials

| Situation                              | Recommended helper                  |
|----------------------------------------|-------------------------------------|
| One-off extracted fragment             | `partial("thing", ...)`             |
| Named, reusable UI primitive           | `component("thing", ...)`           |
| Design-system pieces (cards, rows...)  | `component("job_row", ...)`         |
| Internal shared layout helper          | `partial("shared/...", ...)`        |

Both ultimately call the same rendering machinery. The difference is intent and file organization.

### Best practices

- Keep components small and purely presentational.
- Start the file with a `#` comment listing the expected keys (Soli comments inside templates are `#`).
- Always use `h()` for user data.
- Choose descriptive names: `user_avatar`, `empty_state`, `job_row`, `stat_block`.
- Use subdirectories when you have more than a handful of components.
- Prefer `component()` over `partial()` for anything that feels like a reusable widget.

### Example: realistic admin component

```erb
<!-- app/views/components/job_row.html.slv -->
# Expected locals:
#   job: Hash with id, script, webhook_url, status, priority
#   status_class: optional extra class

<tr class="job-row <%= status_class || "" %>">
  <td class="font-mono"><%= job["id"] %></td>
  <td><%= h(job["script"] || job["webhook_url"]) %></td>
  <td><span class="status-pill"><%= job["status"] %></span></td>
  <td class="text-right"><%= job["priority"] %></td>
  <td>
    <% if job["status"] == "pending" %>
      <button
        class="btn-xs"
        hx-post="<%= some_action_path(job) %>"
        hx-target="closest tr"
      >
        Run now
      </button>
    <% end %>
  </td>
</tr>
```

Used from a queues view:

```erb
<% for job in @jobs %>
  <%- component("job_row", {
    "job": job,
    "status_class": job["status"] == "failed" ? "is-failed" : ""
  }) %>
<% end %>
```

## Passing Data

Controllers pass data to views:

```soli
def show
  render("posts/show", {
    "title": "My Post",
    "post": post,
    "comments": comments,
    "author": author
  })
end
```

Access in template:

```erb
<h1><%= post.title %></h1>
<div class="content"><%= post.content %></div>

<h2>Comments</h2>
<% for comment in comments %>
  <div class="comment">
    <strong><%= comment.author %></strong>
    <p><%= comment.text %></p>
  </div>
<% end %>
```

## View Best Practices

1. Keep views simple and focused on presentation
2. Use `component()` for named, reusable UI primitives (cards, rows, badges, etc.)
3. Use `partial()` for one-off fragments or internal helpers
4. Never put business logic in views
5. Use helper functions for complex formatting
6. Escape user-generated content with `h()`

## File Organization

```
app/views/
├── home/
│   ├── index.html.slv
│   └── show.html.slv
├── users/
│   ├── _form.html.slv
│   ├── edit.html.slv
│   └── show.html.slv
├── components/
│   ├── card.html.slv
│   ├── job_row.html.slv
│   └── table/
│       └── row.html.slv
├── partials/
│   ├── _header.html.slv
│   └── _footer.html.slv
└── layouts/
    ├── application.html.slv
    └── docs.html.slv
```
