// Dotenv Example
// This example demonstrates loading environment variables from a .env file

// Load environment variables from .env file (defaults to ".env")
let count = dotenv();
println("Loaded", count, "environment variables from .env");

// Access environment variables
let database_url = getenv("DATABASE_URL");
let api_key = getenv("API_KEY");

if database_url != null {
    println("Database URL:", database_url);
} else {
    println("DATABASE_URL not set");
}

if api_key != null {
    println("API Key is configured");
} else {
    println("API_KEY not set (create a .env file to configure)");
}

// Check if specific variables exist
if hasenv("DEBUG") {
    let debug = getenv("DEBUG");
    println("Debug mode:", debug);
}

// Set environment variables programmatically
setenv("MY_APP_VAR", "my_value");
println("Set MY_APP_VAR:", getenv("MY_APP_VAR"));

// Check system environment variables
let home = getenv("HOME");
let path = getenv("PATH");

if home != null {
    println("Home directory:", home);
}

println("Path length:", len(path));

// Example .env file format:
// DATABASE_URL=postgres://user:pass@localhost:5432/mydb
// API_KEY=your-api-key-here
// DEBUG=true
// PORT=8080

println("\nDone!");
