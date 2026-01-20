// Test default parameters
fn greet(name: String = "World") -> String {
    return "Hello \(name)!";
}

print("Testing default parameters:");
print(greet());           // Hello World!
print(greet("Alice"));    // Hello Alice!

fn add(a: Int, b: Int = 10) -> Int {
    return a + b;
}

print(add(5));     // 15
print(add(5, 3));  // 8

fn configure(debug: Bool = false, port: Int = 3000, host: String = "localhost") -> String {
    return "debug=\(debug), port=\(port), host=\(host)";
}

print(configure());                    // debug=false, port=3000, host=localhost
print(configure(true));                // debug=true, port=3000, host=localhost
print(configure(true, 8080));          // debug=true, port=8080, host=localhost
print(configure(false, 9000, "0.0.0.0")); // debug=false, port=9000, host=0.0.0.0

print("Default parameters tests passed!");
