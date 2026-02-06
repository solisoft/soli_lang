# Views

Views handle the presentation layer of your application.

## Template Syntax

SoliLang uses ERB-style templates with `<%= %>` for output and `<% %>` for logic.

### Output Variables

```erb
<h1><%= title %></h1>
<p>Hello, <%= name %>!</p>
<p>Count: <%= count %></p>
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

Escapes HTML special characters to prevent XSS attacks.

```erb
<p><%= html_escape(user_input) %></p>
<p><%= h(user_input) %></p>
```

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

Formats a number with thousands separators. Locale-aware by default.

**Parameters:**
- `number` (Int|Float) - The number to format
- `delimiter` (String, optional) - The separator character (auto-detected from locale if not specified)

```erb
<p>Views: <%= number_with_delimiter(1234567) %></p>
<!-- en: "1,234,567" | fr: "1 234 567" | de: "1.234.567" -->

<p>Population: <%= number_with_delimiter(population) %></p>

<!-- Override with specific delimiter -->
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
// app/helpers/application_helper.sl

// ... existing helpers ...

// Custom helper: Format a phone number
fn format_phone(number) {
    let digits = replace(number, "[^0-9]", "")
    if len(digits) == 10 {
        return "(" + substring(digits, 0, 3) + ") " + substring(digits, 3, 6) + "-" + substring(digits, 6, 10)
    }
    number
}

// Custom helper: Generate a mailto link
fn mail_to(email, text = null) {
    if text == null {
        text = email
    }
    "<a href=\"mailto:" + html_escape(email) + "\">" + html_escape(text) + "</a>"
}
```

---

## Layouts

Wrap views in a common layout:

```erb
<!-- app/views/layouts/application.html.erb -->
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
fn index(req: Any) -> Any {
    render("home/index", {
        "title": "Welcome"
    }, "layouts/application")
}
```

## Partials

Reuse template fragments:

```erb
<!-- app/views/partials/user_card.html.erb -->
<div class="user-card">
    <h3><%= user.name %></h3>
    <p><%= user.email %></p>
</div>
```

Include partials:

```erb
<%= render_partial("partials/user_card", {"user": current_user}) %>

<% for user in users %>
    <%= render_partial("partials/user_card", {"user": user}) %>
<% end %>
```

## Passing Data

Controllers pass data to views:

```soli
fn show(req: Any) -> Any {
    render("posts/show", {
        "title": "My Post",
        "post": post,
        "comments": comments,
        "author": author
    })
}
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
2. Use partials for repeated elements
3. Never put business logic in views
4. Use helper functions for complex formatting
5. Escape user-generated content with `h()`

## File Organization

```
app/views/
├── home/
│   ├── index.html.erb
│   └── show.html.erb
├── users/
│   ├── _form.html.erb
│   ├── edit.html.erb
│   └── show.html.erb
├── partials/
│   ├── _header.html.erb
│   └── _footer.html.erb
└── layouts/
    ├── application.html.erb
    └── docs.html.erb
```
