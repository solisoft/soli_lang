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
    return render("home/index", {
        "title": "Welcome"
    }, "layouts/application");
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
    return render("posts/show", {
        "title": "My Post",
        "post": post,
        "comments": comments,
        "author": author
    });
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
