# Controllers

Controllers handle HTTP requests and return responses. SoliLang supports OOP-style controllers with class inheritance, before/after action hooks, and request context injection.

## Creating a Controller

Create a file in `app/controllers/` with a `_controller.sl` suffix:

```soli
// app/controllers/users_controller.sl
class UsersController extends Controller {
    fn index(req: Any) -> Any {
        return render("users/index", {
            "title": "Users",
            "users": []
        });
    }

    fn show(req: Any) -> Any {
        let user_id = req.params["id"];
        return render("users/show", {
            "title": "User Details",
            "user_id": user_id
        });
    }
}
```

## OOP Controller Architecture

### Class-Based Controllers

Controllers are classes that extend the base `Controller` class:

```soli
class PostsController extends Controller {
    // Actions go here
    fn index(req: Any) -> Any { /* ... */ }
    fn show(req: Any) -> Any { /* ... */ }
}
```

### Static Configuration Block

Configure controllers using a `static { ... }` block:

```soli
class ApplicationController extends Controller {
    static {
        // Set the layout for all actions
        this.layout = "application";

        // Before action that runs for all actions
        this.before_action = fn(req) {
            let user_id = req.session["user_id"];
            if user_id != null {
                req["current_user"] = User.find(user_id);
            }
            return req;
        };
    }
}
```

### Controller Actions

Each public function in a controller is an action:

```soli
class PostsController extends Controller {
    fn index(req: Any) -> Any { /* List posts */ }
    fn show(req: Any) -> Any { /* Show single post */ }
    fn new(req: Any) -> Any { /* Show new post form */ }
    fn create(req: Any) -> Any { /* Create new post */ }
    fn edit(req: Any) -> Any { /* Show edit form */ }
    fn update(req: Any) -> Any { /* Update post */ }
    fn delete(req: Any) -> Any { /* Delete post */ }

    # Private helper methods (not exposed as actions)
    fn _validate_post(req: Any) -> Bool {
        return req.params["title"] != null;
    }
}
```

**Note:** Methods starting with `_` are private and not exposed as routes.

## Controller Inheritance

Create an `ApplicationController` with shared configuration:

```soli
// app/controllers/application_controller.sl
class ApplicationController extends Controller {
    static {
        this.layout = "application";

        # Run for all actions
        this.before_action = fn(req) {
            # Authentication check
            let user_id = req.session["user_id"];
            if user_id == null {
                return redirect("/login");
            }
            req["current_user"] = User.find(user_id);
            return req;
        };
    }

    # Shared helper method available to all subclasses
    fn _current_user(req: Any) -> Any {
        return req["current_user"];
    }
}
```

Subclasses inherit the configuration and can override it:

```soli
// app/controllers/posts_controller.sl
class PostsController extends ApplicationController {
    static {
        # Override layout for this controller
        this.layout = "posts";

        # Run before_action only for specific actions
        this.before_action(:show, :edit, :update, :delete) = fn(req) {
            let post = Post.find(req.params["id"]);
            if post == null {
                return error(404, "Post not found");
            }
            req["post"] = post;
            return req;
        };
    }

    fn index(req: Any) -> Any {
        # Can use inherited _current_user helper
        let user = this._current_user(req);
        let posts = Post.all();
        return render("posts/index", {
            "posts": posts,
            "user": user
        });
    }

    fn show(req: Any) -> Any {
        # req["post"] is set by before_action
        return render("posts/show", { "post": req["post"] });
    }
}
```

## Before/After Action Hooks

### Before Actions

Run code before an action executes. Can filter to specific actions:

```soli
class PostsController extends Controller {
    static {
        # Run for all actions
        this.before_action = fn(req) {
            println("Before any action: " + req.path);
            return req;
        };

        # Run only for specific actions
        this.before_action(:show, :edit, :delete) = fn(req) {
            let post = Post.find(req.params["id"]);
            if post == null {
                return error(404, "Post not found");
            }
            req["post"] = post;
            return req;
        };
    }
}
```

**Short-circuiting:** Return a response from a before action to skip the action:

```soli
this.before_action = fn(req) {
    if req.session["user_id"] == null {
        return redirect("/login");
    }
    return req;  # Continue to action
};
```

### After Actions

Run code after an action executes:

```soli
class PostsController extends Controller {
    static {
        this.after_action = fn(req, response) {
            # Log the action
            println("Completed: " + req.path);
            return response;  # Return modified or original response
        };
    }
}
```

Filter after actions to specific actions:

```soli
this.after_action(:create, :update) = fn(req, response) {
    # Log changes after create/update
    println("Data modified");
    return response;
};
```

## Request Object

Access request data through the `req` parameter:

```soli
fn create(req: Any) -> Any {
    # Path parameters
    let id = req.params["id"];

    # Query string parameters
    let page = req.query["page"];

    # Form data
    let name = req.form["name"];

    # JSON body (if Content-Type is application/json)
    let data = req.json;

    # HTTP headers
    let auth = req.headers["Authorization"];

    # HTTP method
    let method = req.method;

    # Original path
    let path = req.path;

    # Session data
    let user_id = req.session["user_id"];

    # Store data for after_action or views
    req["my_data"] = some_value;
}
```

### Request Context in Controllers

The request object is automatically injected into your controller:

```soli
class PostsController extends Controller {
    fn show(req: Any) -> Any {
        # Access params directly
        let id = req.params["id"];

        # Or access via this (after injection)
        let post = this.get_controller_field(req, "post");

        return render("posts/show", { "post": post });
    }
}
```

## Returning Responses

### Render a Template

```soli
fn index(req: Any) -> Any {
    return render("home/index", {
        "title": "Welcome",
        "message": "Hello!"
    });
}
```

### Render with Custom Layout

Set the layout in your controller:

```soli
class PostsController extends Controller {
    static {
        this.layout = "posts";  # Uses layouts/posts.html.erb
    }

    fn show(req: Any) -> Any {
        return render("posts/show", { "post": req["post"] });
    }

    # Skip layout for specific action
    fn json_only(req: Any) -> Any {
        return render_json({ "data": "value" }, layout: false);
    }
}
```

### Redirect

```soli
fn create(req: Any) -> Any {
    # Process form data...

    # Redirect to another page
    return redirect("/users");
}

fn update(req: Any) -> Any {
    # After update, redirect to show page
    let user_id = req.params["id"];
    return redirect("/users/" + user_id);
}
```

### JSON Response

```soli
fn api_users(req: Any) -> Any {
    return render_json({
        "users": [
            {"id": 1, "name": "Alice"},
            {"id": 2, "name": "Bob"}
        ]
    });
}
```

### Plain Text

```soli
fn ping(req: Any) -> Any {
    return render_text("pong");
}
```

### Error Response

```soli
fn show(req: Any) -> Any {
    let id = req.params["id"];
    if id == "" {
        return error(400, "Missing ID");
    }
    let user = find_user(id);
    if user == null {
        return error(404, "User not found");
    }
    return render("users/show", {"user": user});
}
```

## Controller Context

Controllers have access to context through `this`:

```soli
class PostsController extends Controller {
    static {
        this.layout = "posts";
        this.before_action = fn(req) {
            # Store data on request for later use
            req["post"] = Post.find(req.params["id"]);
            return req;
        };
    }

    fn show(req: Any) -> Any {
        # Access the post set by before_action
        let post = req["post"];

        return render("posts/show", { "post": post });
    }

    # Access request parameters
    fn _get_id(req: Any) -> String {
        return req.params["id"];
    }
}
```

## Strong Parameters

Validate and sanitize input:

```soli
fn create(req: Any) -> Any {
    let params = req.form;
    let clean_params = {
        "name": params["name"] ?? "",
        "email": params["email"] ?? "",
        "age": int(params["age"] ?? "0")
    };
}
```

## Routing to Controller Actions

Routes use `controller#action` syntax:

```soli
# config/routes.sl
get("/", "home#index");
get("/users", "users#index");
get("/users/:id", "users#show");
post("/users", "users#create");
```

The router automatically:
1. Instantiates a new controller instance per request
2. Injects the request context
3. Runs before_action hooks
4. Calls the action method
5. Runs after_action hooks
6. Returns the response

## File Naming Convention

| File | Class | Route Prefix |
|------|-------|--------------|
| `home_controller.sl` | `HomeController` | `home#` |
| `users_controller.sl` | `UsersController` | `users#` |
| `posts_controller.sl` | `PostsController` | `posts#` |
| `admin/users_controller.sl` | `Admin::UsersController` | `admin/users#` |

## Best Practices

1. **Keep controllers thin, models fat** - Business logic belongs in models
2. **Use before_action for authentication** - Common pattern for access control
3. **Validate parameters before processing** - Use strong parameters pattern
4. **Return appropriate HTTP status codes** - 200, 201, 400, 401, 404, 500
5. **Use redirects after successful POST requests** - Prevent form resubmission
6. **Use private helper methods** - Methods starting with `_` are not exposed
7. **Create ApplicationController** - Base class for shared configuration
8. **Use layouts consistently** - Set default layout in ApplicationController

## Testing Controllers

See the [Testing Guide](/docs/testing) for comprehensive information on testing controllers with both HTTP integration tests and direct action calls.
