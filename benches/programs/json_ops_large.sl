# JSON benchmark with larger payload - 1000 iterations

let users: Array = []
let i = 0
while i < 50
    users.push({"id" => i, "name" => "User " + str(i), "email" => "user" + str(i) + "@example.com", "active" => true, "score" => 42.5})
    i = i + 1
end

let data = {
    "users" => users,
    "count" => 50,
    "timestamp" => 1234567890,
    "metadata" => {"version" => "1.0", "source" => "benchmark"}
}

let json_string = JSON.stringify(data)
let iterations = 1000

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

print("JSON large benchmark (" + str(iterations) + " iterations, 50 users)")
print("  json size: " + str(json_string.length) + " bytes")
print("  stringify: " + str(stringify_time) + "ms")
print("  parse: " + str(parse_time) + "ms")
print("  total: " + str(stringify_time + parse_time) + "ms")
