// SoliDB Example
// This example demonstrates how to use the Solidb class to interact with SoliDB

// Create a database connection
let db = new Solidb("http://localhost:6745", "solidb");

// Authenticate (if your database requires authentication)
// db.auth("username", "password");

// Insert some documents
db.insert("users", "user001", {
    "name": "Alice",
    "email": "alice@example.com",
    "age": 30,
    "city": "Paris"
});

db.insert("users", "user002", {
    "name": "Bob",
    "email": "bob@example.com",
    "age": 25,
    "city": "London"
});

db.insert("users", "user003", {
    "name": "Charlie",
    "email": "charlie@example.com",
    "age": 35,
    "city": "Paris"
});

// Query all users
println("All users:");
let all_users = db.query("FOR u IN users RETURN u", {});
println(all_users);

// Query with filter
println("\nUsers in Paris:");
let paris_users = db.query("FOR u IN users FILTER u.city == 'Paris' RETURN u", {});
println(paris_users);

// Query with bind variables
println("\nUsers older than 27:");
let older_users = db.query("FOR u IN users FILTER u.age > @min_age RETURN u", {"min_age": 27});
println(older_users);

// Get a specific user
println("\nUser user001:");
let user = db.get("users", "user001");
println(user);
println("Name:", user["name"]);
println("Email:", user["email"]);

// Update a document
db.update("users", "user001", {"age": 31, "name": "Alice Smith"}, true);
let updated = db.get("users", "user001");
println("\nUpdated user001:");
println(updated);

// Upsert (merge update)
db.update("users", "user004", {"name": "David", "age": 28}, true);
let new_user = db.get("users", "user004");
println("\nNew user (upserted):");
println(new_user);

// List all documents in a collection
println("\nAll users in collection:");
let users = db.list("users", 100, 0);
println(users);

// Delete a document
db.delete("users", "user004");
println("\nDeleted user004");

// Check connection status
println("\nDatabase connected:", db.connected());

// Ping the server
let timestamp = db.ping();
println("Server timestamp:", timestamp);

// Explain a query
let plan = db.explain("FOR u IN users FILTER u.age > @age RETURN u", {"age": 25});
println("\nQuery execution plan:");
println(plan);

println("\nDone!");
