# JSON benchmark - parse and stringify

let data = {
    "users" => [
        {"id" => 1, "name" => "Alice", "email" => "alice@example.com"},
        {"id" => 2, "name" => "Bob", "email" => "bob@example.com"},
        {"id" => 3, "name" => "Charlie", "email" => "charlie@example.com"},
    ],
    "count" => 3,
    "timestamp" => 1234567890
}

let json_string = JSON.stringify(data)
let iterations = 10000

# Benchmark JSON.stringify
let json_result = ""
let start = clock()
let i = 0
while i < iterations
    json_result = JSON.stringify(data)
    i = i + 1
end
let stringify_time = (clock() - start) * 1000

# Benchmark JSON.parse
let parse_result = null
let start = clock()
let i = 0
while i < iterations
    parse_result = JSON.parse(json_string)
    i = i + 1
end
let parse_time = (clock() - start) * 1000

print("JSON benchmark (" + str(iterations) + " iterations)")
print("  stringify: " + str(stringify_time) + "ms")
print("  parse: " + str(parse_time) + "ms")
print("  total: " + str(stringify_time + parse_time) + "ms")
