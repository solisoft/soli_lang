# Custom Error Pages Example

This example demonstrates how to create custom error pages for production mode.

## Structure

```
custom_errors/
├── app/
│   ├── controllers/
│   │   └── home_controller.sl
│   └── views/
│       ├── errors/
│       │   ├── 404.html.erb      # Custom 404 page
│       │   └── 500.html.erb      # Custom 500 page
│       └── home/
│           └── index.html.erb
└── public/
    └── app.css
```

## Creating Custom Error Pages

### 1. Create error templates

Place your custom error templates in `app/views/errors/` following the status code naming convention:

- `app/views/errors/400.html.erb`
- `app/views/errors/403.html.erb`
- `app/views/errors/404.html.erb`
- `app/views/errors/500.html.erb`
- etc.

### 2. Template context variables

Error templates have access to these variables:

| Variable | Description |
|----------|-------------|
| `status` | The HTTP status code (e.g., 500) |
| `message` | The error message |
| `request_id` | Unique error ID for support reference |

### 3. Example template (404.html.erb)

```erb
<div class="min-h-screen flex items-center justify-center bg-gray-100">
    <div class="text-center">
        <h1 class="text-6xl font-bold text-gray-800 mb-4"><%= status %></h1>
        <h2 class="text-2xl font-semibold text-gray-600 mb-4">Page Not Found</h2>
        <p class="text-gray-500 mb-8"><%= message %></p>
        <a href="/" class="px-6 py-2 bg-blue-600 text-white rounded hover:bg-blue-700">
            Go Home
        </a>
        <p class="mt-8 text-xs text-gray-400">Error ID: <%= request_id %></p>
    </div>
</div>
```

### 4. Run in production mode

```bash
# Development mode (shows detailed error pages)
soli serve custom_errors

# Production mode (shows custom error pages)
soli serve --no-dev custom_errors
```

## Notes

- Error pages render WITHOUT layouts to avoid potential cascading errors
- If a custom template is not found, the default error page is used
- Error templates can use any template features (conditionals, loops, etc.)
