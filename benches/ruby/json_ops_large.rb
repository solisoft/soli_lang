require 'json'

users = (0...50).map do |i|
  {"id" => i, "name" => "User #{i}", "email" => "user#{i}@example.com", "active" => true, "score" => 42.5}
end

data = {
  "users" => users,
  "count" => 50,
  "timestamp" => 1234567890,
  "metadata" => {"version" => "1.0", "source" => "benchmark"}
}

iterations = 1000

# Benchmark JSON.generate (stringify)
start = Time.now
json = nil
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

puts "JSON large benchmark (#{iterations} iterations, 50 users)"
puts "  json size: #{json.length} bytes"
puts "  stringify: #{stringify_time.round(1)}ms"
puts "  parse: #{parse_time.round(1)}ms"
puts "  total: #{(stringify_time + parse_time).round(1)}ms"
