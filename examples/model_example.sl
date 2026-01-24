// Model/ORM Example
// This example demonstrates how to use the Model/ORM system

// Define field types
let nameField = Field.string("name").required();
let ageField = Field.int("age").min(0).max(150);
let emailField = Field.string("email").unique().index();

// Create a new user
let user = model_create("User", "users", {
    "name": "Alice",
    "email": "alice@example.com",
    "age": 30
});

// Query users with filters
let adults = model_where("User", "users", "database", "age", ">=", 18);
adults.order_by("name", "asc");
adults.limit(10);
let results = adults.find();

// Get a specific user by key
let user = model_get("User", "users", "user001");

// Update a user
model_update("User", "users", "user001", {
    "age": 31
});

// Delete a user
model_delete("User", "users", "user001");

// Count users
let count = model_count("User", "users");

// Define a migration
let migration = Migration("CreateUsersTable", "001", "Create users table");

// Run migrations
model_migrate();

// Rollback last migration
model_rollback();

// Check migration status
model_status();

println("Model example completed!");
