// Routes configuration

// Home page
get("/", "home#index");

// Health check endpoint
get("/health", "home#health");
