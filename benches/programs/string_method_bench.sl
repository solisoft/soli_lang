// Benchmark string methods
let iterations = 10000;
let result = "";

let start = std.time();

let i = 0;
while (i < iterations) {
    let s = "hello world test string";
    s = s.to_uppercase();
    s = s.to_lowercase();
    s = s.trim();
    s = s.replace("world", "universe");
    s = s.contains("test")?.to_string();
    i = i + 1;
}

let end = std.time();
let elapsed = end - start;
print("String methods: " + elapsed.to_string() + "ms for " + iterations.to_string() + " iterations");