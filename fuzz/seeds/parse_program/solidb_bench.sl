// Benchmark script for SoliDB HTTP wrapper via Soli framework
// Run with: soli serve . --dev

let db = new Solidb("http://localhost:6745", "solidb");

println("SoliDB HTTP Wrapper Benchmark");
println("==============================");

// Test 1: Ping
let start = DateTime.now();
for i in 1..100 {
    let result = db.ping();
}
let end = DateTime.now();
let ping_time = Duration.between(start, end);
println("100 pings:", ping_time.total_millis(), "ms");

// Test 2: Query
let start = DateTime.now();
for i in 1..100 {
    let result = db.query("FOR u IN users LIMIT 1 RETURN u", {});
}
let end = DateTime.now();
let query_time = Duration.between(start, end);
println("100 queries:", query_time.total_millis(), "ms");

// Test 3: Get document
let start = DateTime.now();
for i in 1..100 {
    let result = db.get("users", "user1");
}
let end = DateTime.now();
let get_time = Duration.between(start, end);
println("100 gets:", get_time.total_millis(), "ms");

println("Benchmark complete!");
