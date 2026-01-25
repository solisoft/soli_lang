// ============================================================================
// Routes Configuration for AI/LLM Code Generation
// ============================================================================
//
// AI AGENT GUIDE:
// ---------------
// This file documents routing conventions for the Soli MVC framework.
//
// BASIC ROUTE SYNTAX:
// HTTP_METHOD('/path', 'controller#action')
//
// HTTP METHODS:
// - get(path, controller#action)    → GET requests
// - post(path, controller#action)   → POST requests
// - put(path, controller#action)    → PUT requests
// - delete(path, controller#action) → DELETE requests
// - patch(path, controller#action)  → PATCH requests
//
// PARAMETERS:
// - Path parameters captured with :param_name syntax
// - Accessed in controller via req["params"]["param_name"]
//
// SCOPED MIDDLEWARE:
// - middleware("middleware_name", -> { routes })
// - Middleware only applies to routes inside the block
//
// SPECIAL ROUTES:
// - router_websocket(path, controller#action) → WebSocket upgrade
// - router_live(path, ControllerLive)         → LiveView component
//
// RESOURCES:
// - resources('name', null) → Generate RESTful routes for a resource
//
// ============================================================================

// ============================================================================
// BASIC ROUTES
// ============================================================================
//
// SIMPLE ROUTE:
// get("/path", "controller#action")
// - Maps GET /path to controller's action method
// - Controller file: app/controllers/controller_controller.sl
// - Controller class: ControllerController
// - Action method: fn action(req: Any) -> Any
//
// EXAMPLE:
// get("/", "home#index") 
// → Maps GET / to HomeController.index method
// → File: app/controllers/home_controller.sl
// → Method: fn index(req: Any) -> Any
//
// ============================================================================

// Root path
get("/", "home#index");

// Public routes
get("/health", "home#health");
get("/up", "home#up");

// ============================================================================
// ROUTES WITH PARAMETERS
// ============================================================================
//
// PARAMETERIZED ROUTE:
// get("/path/:param_name", "controller#action")
// - Captured parameters stored in req["params"]
// - Multiple parameters: /posts/:author/:slug
//
// EXAMPLE:
// get("/posts/:id", "posts#show")
// → Maps GET /posts/123 to PostsController.show
// → Controller accesses id via req["params"]["id"]
//
// ============================================================================

// Post routes with ID parameter
get("/posts/:id", "posts#show");
get("/posts/:id/edit", "posts#edit");
put("/posts/:id", "posts#update");
delete("/posts/:posts_id", "posts#destroy");

// Multiple parameters
get("/posts/:author/:slug", "posts#show_by_slug");

// ============================================================================
// CRUD ROUTE PATTERNS
// ============================================================================
//
// STANDARD CRUD ROUTES:
// GET    /posts              → index      (list all)
// GET    /posts/:id          → show       (view one)
// GET    /posts/new          → new        (show creation form)
// POST   /posts              → create     (handle creation)
// GET    /posts/:id/edit     → edit       (show edit form)
// PUT    /posts/:id          → update     (handle update)
// DELETE /posts/:id          → destroy    (handle deletion)
//
// ============================================================================

// Manual CRUD routes
get("/posts", "posts#index");
get("/posts/new", "posts#new");
post("/posts", "posts#create");

// ============================================================================
// RESOURCES MACRO
// ============================================================================
//
// SYNTAX:
// resources('resource_name', null)
// → Automatically generates all RESTful routes for a resource
//
// EQUIVALENT TO:
// get("/resource_name", "resource_name#index")
// get("/resource_name/new", "resource_name#new")
// post("/resource_name", "resource_name#create")
// get("/resource_name/:id", "resource_name#show")
// get("/resource_name/:id/edit", "resource_name#edit")
// put("/resource_name/:id", "resource_name#update")
// delete("/resource_name/:id", "resource_name#destroy")
//
// ============================================================================

// Generate RESTful routes for users resource
resources("users", null);

// ============================================================================
// SCOPED MIDDLEWARE
// ============================================================================
//
// SYNTAX:
// middleware("middleware_name", -> {
//     route1;
//     route2;
// })
//
// MIDDLEWARE REFERENCE:
// - Middleware function defined in app/middleware/middleware_name.sl
// - Reference by function name (without .sl extension)
// - Middleware must be marked with // scope_only: true
//
// EXAMPLE:
// middleware("authenticate", -> {
//     get("/dashboard", "dashboard#index");
//     get("/profile", "users#profile");
// });
//
// ============================================================================

// Authentication scope
middleware("authenticate", -> {
    get("/users/profile", "users#profile");
    get("/users/settings", "users#settings");
    post("/users/logout", "users#logout");
});

// Admin scope (nested middleware)
middleware("authenticate", -> {
    middleware("require_admin", -> {
        get("/admin", "admin#index");
        get("/admin/users", "admin#users");
        post("/admin/users", "admin#create_user");
    });
});

// ============================================================================
// API ROUTES
// ============================================================================
//
// API ROUTE PATTERNS:
// - Return JSON responses using {"status": N, "body": json_stringify(data)}
// - No render() calls (no views)
// - API versioning via path: /api/v1/resource
//
// ============================================================================

// API v1 routes
middleware("authenticate", -> {
    get("/api/v1/users", "api#users");
    get("/api/v1/users/:id", "api#user_show");
    post("/api/v1/users", "api#create_user");
    put("/api/v1/users/:id", "api#update_user");
    delete("/api/v1/users/:id", "api#delete_user");
});

// ============================================================================
// WEBSOCKET ROUTES
// ============================================================================
//
// SYNTAX:
// router_websocket('/path', 'controller#handler')
// → Upgrades connection to WebSocket
// → Handler receives WebSocket-specific request
//
// ============================================================================

router_websocket("/ws/chat", "websocket#chat_handler");
router_websocket("/ws/notifications", "websocket#notifications");

// ============================================================================
// LIVEVIEW ROUTES
// ============================================================================
//
// SYNTAX:
// router_live('/path', 'LiveComponent')
// → Registers LiveView component
// → Component handles real-time updates
//
// ============================================================================

router_live("/counter", "CounterLive");
router_live("/chat", "ChatLive");
router_live("/metrics", "MetricsLive");

// ============================================================================
// ROUTE ORDERING
// ============================================================================
//
// ROUTE MATCHING:
// - Routes are matched in order of definition
// - More specific routes should come before general ones
// - Parameters (:id) are catch-all at the end
//
// EXAMPLE ORDERING:
// 1. Exact matches: get("/about", "home#about")
// 2. Prefix matches: get("/posts", "posts#index")
// 3. Parameterized: get("/posts/:id", "posts#show")
//
// ============================================================================

// Specific routes first
get("/about", "home#about");
get("/contact", "home#contact");

// Collection routes
get("/posts", "posts#index");
get("/articles", "articles#index");

// Member routes (with parameters)
get("/posts/:id", "posts#show");
get("/articles/:slug", "articles#show");

// ============================================================================
// COMPLETE EXAMPLE: Full Resource with Scopes
// ============================================================================
//
// // Public routes (no authentication)
// get("/products", "products#index");
// get("/products/:id", "products#show");
// get("/products/:id/reviews", "products#reviews");
//
// // Protected routes (authentication required)
// middleware("authenticate", -> {
//     get("/products/new", "products#new");
//     post("/products", "products#create");
//     get("/products/:id/edit", "products#edit");
//     put("/products/:id", "products#update");
//     delete("/products/:id", "products#destroy");
//     
//     // User's own products
//     get("/my/products", "products#my_products");
// });
//
// // Admin-only routes
// middleware("authenticate", -> {
//     middleware("require_admin", -> {
//         get("/admin/products", "admin#products");
//         delete("/admin/products/:id", "admin#delete_product");
//     });
// });
//
// ============================================================================

print("Routes loaded!");
