// Custom Error Pages Example
// 
// This example demonstrates how to create custom error pages for production mode.
// 
// Custom error templates should be placed in:
//   app/views/errors/{status_code}.html.erb
// 
// Available context variables:
//   - status  : The HTTP status code (e.g., 500)
//   - message : The error message
//   - request_id : Unique error ID for support reference
// 
// Error pages render WITHOUT layouts to avoid potential cascading errors.

fn index(req: Any) -> Any {
    return render("home/index.html.erb", { "title": "Welcome" });
}

fn not_found(req: Any) -> Any {
    // This will use app/views/errors/404.html.erb in production
    return { "status": 404, "body": "Custom 404 page" };
}

fn cause_error(req: Any) -> Any {
    // This will use app/views/errors/500.html.erb in production
    let x = null;
    return x["nonexistent"]; // This will cause a runtime error
}
