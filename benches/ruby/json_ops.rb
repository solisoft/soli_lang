require 'json'

data = {
    "users" => [
        {"id" => 1, "name" => "Alice", "email" => "alice@example.com"},
        {"id" => 2, "name" => "Bob", "email" => "bob@example.com"},
        {"id" => 3, "name" => "Charlie", "email" => "charlie@example.com"},
    ],
    "count" => 3,
    "timestamp" => 1234567890
}

iterations = 10000

# Benchmark JSON.generate (stringify)
start = Time.now
i = 0
while i < iterations
    json = JSON.generate(data)
    i += 1
end
stringify_time = (Time.now - start) * 1000

# Benchmark JSON.parse
start = Time.now
i = 0
while i < iterations
    obj = JSON.parse(json)
    i += 1
end
parse_time = (Time.now - start) * 1000

puts "JSON benchmark (#{iterations} iterations)"
puts "  stringify: #{stringify_time.round}ms"
puts "  parse: #{parse_time.round}ms"
puts "  total: #{(stringify_time + parse_time).round}ms"
