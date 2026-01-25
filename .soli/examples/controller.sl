// ============================================================================
// PostsController - Example CRUD Controller for AI/LLM Code Generation
// ============================================================================
//
// AI AGENT GUIDE:
// ---------------
// This file demonstrates the standard Soli MVC controller conventions.
// Use this as a template when generating new controllers.
//
// KEY CONVENTIONS:
// 1. Class name: PascalCase ending with "Controller"
// 2. File name: snake_case ending with "_controller.sl"
// 3. Base class: All controllers extend "Controller"
// 4. Method signature: fn method_name(req: Any) -> Any
// 5. Response types: render() for HTML, dict for JSON/redirect
//
// TEMPLATE FOR AI GENERATION:
// ---------------------------
// class {Resource}Controller extends Controller {
//     static {
//         this.layout = "application";
//         this.before_action = fn(req) { /* auth check */ };
//     }
//     
//     fn index(req: Any) -> Any { /* list resources */ }
//     fn show(req: Any) -> Any { /* show single resource */ }
//     fn new(req: Any) -> Any { /* show creation form */ }
//     fn create(req: Any) -> Any { /* handle creation */ }
//     fn edit(req: Any) -> Any { /* show edit form */ }
//     fn update(req: Any) -> Any { /* handle update */ }
//     fn destroy(req: Any) -> Any { /* handle deletion */ }
// }
//
// ROUTE MAPPINGS:
// ---------------
// GET    /posts              → index
// GET    /posts/:id          → show
// GET    /posts/new          → new (form)
// POST   /posts              → create
// GET    /posts/:id/edit     → edit (form)
// PUT    /posts/:id          → update
// DELETE /posts/:id          → destroy
//
// ============================================================================

class PostsController extends Controller {
    // STATIC BLOCK - Controller-wide configuration
    // ------------------------------------------------
    // - Layout: Which layout template to use (optional, defaults to "application")
    // - before_action: Callback before each action (e.g., authentication)
    // - after_action: Callback after each action (e.g., logging)
    static {
        this.layout = "application";
        
        // Run _authenticate before show, edit, update, destroy actions
        this.before_action = fn(req) {
            let action = req["action"];
            if (action == "show" || action == "edit" || action == "update" || action == "destroy") {
                return _authenticate(req);
            }
            return {"continue": true, "request": req};
        };
    }

    // INDEX ACTION - List all posts
    // Usage: GET /posts
    // ------------------------------------------------
    fn index(req: Any) -> Any {
        // In real app: fetch from database using Post.all()
        let posts = [
            {"id": 1, "title": "First Post", "content": "Hello World"},
            {"id": 2, "title": "Second Post", "content": "Soli MVC is great!"}
        ];
        
        // render(template_path, data_hash) → HTML response
        // template_path: "controller_name/action_name" (without .sl)
        // data_hash: variables passed to template, accessed as @variable_name
        return render("posts/index", {
            "posts": posts,
            "title": "All Posts"
        });
    }

    // SHOW ACTION - Display single post
    // Usage: GET /posts/:id
    // ------------------------------------------------
    fn show(req: Any) -> Any {
        // Access path parameters via req["params"]["param_name"]
        let id = req["params"]["id"];
        
        // In real app: Post.find(id)
        let post = {"id": id, "title": "Post " + id, "content": "Content here"};
        
        if (post == null) {
            // Return 404 response
            return {
                "status": 404,
                "body": "Post not found"
            };
        }
        
        return render("posts/show", {
            "post": post,
            "title": post["title"]
        });
    }

    // NEW ACTION - Show creation form
    // Usage: GET /posts/new
    // ------------------------------------------------
    fn new(req: Any) -> Any {
        return render("posts/new", {
            "post": {"title": "", "content": ""},
            "title": "New Post"
        });
    }

    // CREATE ACTION - Handle form submission
    // Usage: POST /posts
    // ------------------------------------------------
    fn create(req: Any) -> Any {
        // Access JSON body via req["json"]
        let data = req["json"];
        
        // In real app: validate and save to database
        // let result = Post.create(data);
        let new_id = 3;  // Generated ID
        
        // Redirect after successful creation
        // Return redirect URL to redirect client
        return {
            "status": 302,
            "headers": {"Location": "/posts/" + new_id}
        };
    }

    // EDIT ACTION - Show edit form
    // Usage: GET /posts/:id/edit
    // ------------------------------------------------
    fn edit(req: Any) -> Any {
        let id = req["params"]["id"];
        
        // In real app: Post.find(id)
        let post = {"id": id, "title": "Post " + id, "content": "Content here"};
        
        if (post == null) {
            return {"status": 404, "body": "Post not found"};
        }
        
        return render("posts/edit", {
            "post": post,
            "title": "Edit " + post["title"]
        });
    }

    // UPDATE ACTION - Handle edit form submission
    // Usage: PUT /posts/:id
    // ------------------------------------------------
    fn update(req: Any) -> Any {
        let id = req["params"]["id"];
        let data = req["json"];
        
        // In real app: Post.update(id, data)
        let success = true;
        
        if (success) {
            return {
                "status": 302,
                "headers": {"Location": "/posts/" + id}
            };
        }
        
        // Return errors
        return {
            "status": 422,
            "body": json_stringify({"errors": {"title": "Title is required"}})
        };
    }

    // DESTROY ACTION - Delete a post
    // Usage: DELETE /posts/:id
    // ------------------------------------------------
    fn destroy(req: Any) -> Any {
        let id = req["params"]["id"];
        
        // In real app: Post.destroy(id)
        Post.destroy(id);
        
        return {
            "status": 302,
            "headers": {"Location": "/posts"}
        };
    }

    // PRIVATE METHODS - Helper functions (prefixed with _)
    // ------------------------------------------------
    // Private methods are implementation details not exposed as actions.
    // They can be called from other methods in the class.
    
    fn _authenticate(req: Any) -> Any {
        // Example: Check session for authentication
        let authenticated = session_get("authenticated");
        
        if (authenticated != true) {
            // Return redirect to login
            return {
                "continue": false,
                "response": {
                    "status": 302,
                    "headers": {"Location": "/users/login"}
                }
            };
        }
        
        return {"continue": true, "request": req};
    }

    fn _build_post_params(req: Any) -> Any {
        // Extract and sanitize post parameters from request
        let data = req["json"];
        return {
            "title": data["title"],
            "content": data["content"],
            "author_id": session_get("user_id")
        };
    }
}
