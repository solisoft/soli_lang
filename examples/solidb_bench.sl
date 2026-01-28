// Benchmark script for SoliDB HTTP wrapper via Soli framework
// Run with: soli serve . --dev

let db = new Solidb("http://localhost:6745", "solidb");

println("SoliDB HTTP Wrapper Benchmark");
println("==============================");

// Test 1: Ping
let start = datetime_now();
for i in 1..100 {
    let result = db.ping();
}
let ping_time = datetime_now() - start;
println("100 pings:", ping_time, "ms");

// Test 2: Query
let start = datetime_now();
for i in 1..100 {
    let result = db.query("FOR u IN users LIMIT 1 RETURN u", {});
}
let query_time = datetime_now() - start;
println("100 queries:", query_time, "ms");

// Test 3: Get document
let start = datetime_now();
for i in 1..100 {
    let result = db.get("users", "user1");
}
let get_time = datetime_now() - start;
println("100 gets:", get_time, "ms");

println("Benchmark complete!");
