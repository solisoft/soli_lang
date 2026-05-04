// Routes configuration

// Home page
get("/", "home#index", name: "root");

// Health check endpoint
get("/health", "home#health");

// Tip: `resources("posts")` registers the seven RESTful routes plus
// matching `posts_path` / `post_path(post)` / `new_post_path` /
// `edit_post_path(post)` helpers (and `*_url` variants). Use these in
// controllers and views instead of concatenating URLs by hand.
//
//   resources("posts");
//
// For one-off routes, attach a `name:` keyword arg to get the helper:
//
//   get("/about", "pages#about", name: "about");
//   # → about_path()  →  "/about"
