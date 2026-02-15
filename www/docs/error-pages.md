# Error Pages

Soli provides a comprehensive error handling system that displays detailed error information during development and clean, user-friendly error pages in production.

## Development vs Production Mode

### Development Mode (`--dev` or no flag)

When running in development mode, Soli displays detailed error pages that include:

- Full stack traces with source code context
- Interactive REPL for inspecting request state
- Request details (params, query, body, headers, session)
- Quick inspection buttons for common data

This helps developers quickly identify and fix bugs during development.

### Production Mode (`--no-dev`)

In production mode, error pages are minimal and user-friendly:

- No stack traces or internal details
- Clean, branded error pages
- Error ID for support reference
- Navigation options (Go Home, Go Back)

Custom error pages can override the defaults.

## Default Error Pages

Soli includes built-in error pages for common HTTP status codes:

| Status Code | Description |
|-------------|-------------|
| 400 | Bad Request |
| 403 | Forbidden |
| 404 | Not Found |
| 405 | Method Not Allowed |
| 500 | Internal Server Error |
| 502 | Bad Gateway |
| 503 | Service Unavailable |

## Custom Error Pages

You can create custom error pages for any HTTP status code by placing templates in `app/views/errors/`.

### Directory Structure

```
app/
  views/
    errors/
      400.html.slv    # Custom 400 Bad Request page
      403.html.slv    # Custom 403 Forbidden page
      404.html.slv    # Custom 404 Not Found page
      500.html.slv    # Custom 500 Internal Server Error page
      502.html.slv    # Custom 502 Bad Gateway page
      503.html.slv    # Custom 503 Service Unavailable page
```

### Template Variables

Custom error templates have access to these variables:

| Variable | Type | Description |
|----------|------|-------------|
| `status` | Number | The HTTP status code (e.g., 500) |
| `message` | String | The error message |
| `request_id` | String | Unique identifier for support reference |

### Example: Custom 404 Page

```erb
<div style="min-height: 100vh; display: flex; align-items: center; justify-content: center; background-color: #f8f9fa;">
    <div style="text-align: center;">
        <h1 style="font-size: 6rem; font-weight: bold; color: #343a40; margin-bottom: 1rem;"><%= status %></h1>
        <h2 style="font-size: 1.5rem; font-weight: 600; color: #6c757d; margin-bottom: 1rem;">Page Not Found</h2>
        <p style="color: #6c757d; margin-bottom: 2rem;"><%= message %></p>
        <div style="display: flex; gap: 0.75rem; justify-content: center; flex-wrap: wrap;">
            <a href="/" style="display: inline-block; padding: 0.75rem 1.5rem; background-color: #007bff; color: white; text-decoration: none; border-radius: 0.375rem;">Go Home</a>
            <button onclick="history.back()" style="padding: 0.75rem 1.5rem; border: 1px solid #dee2e6; background: white; border-radius: 0.375rem; cursor: pointer;">Go Back</button>
        </div>
        <p style="margin-top: 2rem; font-size: 0.75rem; color: #adb5bd;">Error ID: <%= request_id %></p>
    </div>
</div>
```

### Example: Custom 500 Page

```erb
<div style="min-height: 100vh; display: flex; align-items: center; justify-content: center; background-color: #fff5f5;">
    <div style="text-align: center; max-width: 400px;">
        <div style="margin-bottom: 1.5rem;">
            <svg style="width: 5rem; height: 5rem; margin: 0 auto; color: #e53e3e;" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
            </svg>
        </div>
        <h1 style="font-size: 3rem; font-weight: bold; color: #c53030; margin-bottom: 1rem;"><%= status %></h1>
        <h2 style="font-size: 1.25rem; font-weight: 600; color: #1a202c; margin-bottom: 1rem;">Something went wrong</h2>
        <p style="color: #4a5568; margin-bottom: 2rem;">We're sorry, but something unexpected happened. Our team has been notified.</p>
        <a href="/" style="display: inline-block; padding: 0.75rem 1.5rem; background-color: #c53030; color: white; text-decoration: none; border-radius: 0.375rem;">Return to Homepage</a>
        <p style="margin-top: 2rem; font-size: 0.75rem; color: #718096;">Reference ID: <%= request_id %></p>
    </div>
</div>
```

### Using Tailwind CSS

If your application already uses Tailwind CSS, you can use Tailwind classes:</```erb
<div class="min-h-screen flex items-center justify-center bg-gray-100">
    <div class="text-center">
        <h1 class="text-6xl font-bold text-gray-800 mb-4"><%= status %></h1>
        <h2 class="text-2xl font-semibold text-gray-600 mb-4">Page Not Found</h2>
        <p class="text-gray-500 mb-8"><%= message %></p>
        <a href="/" class="px-6 py-2 bg-blue-600 text-white rounded hover:bg-blue-700">Go Home</a>
        <p class="mt-8 text-xs text-gray-400">Error ID: <%= request_id %></p>
    </div>
</div>
```

### Example: Custom 500 Page

```erb
<div class="min-h-screen flex items-center justify-center bg-red-50">
    <div class="text-center max-w-md">
        <div class="mb-6">
            <svg class="w-20 h-20 mx-auto text-red-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                    d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
            </svg>
        </div>
        <h1 class="text-5xl font-bold text-red-600 mb-4"><%= status %></h1>
        <h2 class="text-2xl font-semibold text-gray-800 mb-4">Something went wrong</h2>
        <p class="text-gray-600 mb-8">We're sorry, but something unexpected happened. Our team has been notified.</p>
        <a href="/" class="inline-block px-6 py-3 bg-red-600 text-white rounded-lg hover:bg-red-700">
            Return to Homepage
        </a>
        <p class="mt-8 text-xs text-gray-500">Reference ID: <%= request_id %></p>
    </div>
</div>
```

### Important Notes

1. **No Layouts**: Error pages render without application layouts to avoid potential cascading errors.

2. **Optional**: Custom error pages are optional. If a template for a status code doesn't exist, Soli uses the default error page.

3. **Styling**: Custom error pages can use any styling approach (CSS, Tailwind, etc.).

4. **Template Features**: Error templates support all template features including conditionals, loops, and partials.

## Error Handling in Controllers

You can also handle errors explicitly in your controllers by returning appropriate responses:

```soli
fn show(req: Any) -> Any {
    let id = req["params"]["id"];
    let user = database.get_user(id);
    
    if user == null {
        return {
            "status": 404,
            "body": "User not found"
        };
    }
    
    render("users/show.html.slv", { "user": user })
}
```

## Production Deployment

When deploying to production, ensure you run the server in production mode:

```bash
# Using soli CLI
soli serve --no-dev

# Or set environment variable
SOLI_ENV=production soli serve
```

This ensures:
- Custom error pages are used
- No sensitive information is leaked
- Error IDs are generated for support reference

## Troubleshooting

### Error pages not showing custom templates

1. Verify templates are in `app/views/errors/` (not `app/views/layouts/errors/` or another location)
2. Ensure template files have the correct extension (`.html.slv` or `.slv`)
3. Check that the status code in the filename matches the HTTP status
4. Verify the application is running in production mode (`--no-dev`)

### Error pages still showing defaults

1. Check that the template file exists and is readable
2. Verify there are no syntax errors in the template
3. Ensure templates don't reference undefined variables
4. Check server logs for template loading errors

## Best Practices

1. **Keep it simple**: Error pages should be clean and focused on helping users

2. **Provide navigation**: Include links to help users return to working pages

3. **Log error IDs**: Store error IDs in your logs to correlate user reports with server errors

4. **Test error flows**: Regularly test your custom error pages to ensure they render correctly

5. **Consider localization**: If your application supports multiple languages, consider creating localized error pages
