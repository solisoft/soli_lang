# Routes configuration
# Define your application routes here

# Home page
get("/", "home#index");

# Health check endpoint
get("/health", "home#health");

print("Routes loaded!");
