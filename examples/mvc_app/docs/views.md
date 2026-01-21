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

```erb
<%= h(user_input) %>          <!-- HTML escape -->
<%= html_escape(content) %>   <!-- Same as h() -->
<%= json_encode(data) %>      <!-- Convert to JSON -->
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
