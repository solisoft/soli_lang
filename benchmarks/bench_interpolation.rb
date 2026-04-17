def bench(label, iterations, &block)
  start = Process.clock_gettime(Process::CLOCK_MONOTONIC)
  iterations.times { block.call }
  elapsed = Process.clock_gettime(Process::CLOCK_MONOTONIC) - start
  per_iter = elapsed / iterations * 1_000_000
  puts "#{label}: #{elapsed.round(6)}s (#{per_iter.round(3)} µs/iter)"
end

N = 100_000

puts '=== String Interpolation ==='

bench('single var', N) do
  name = 'Alice'
  s = "Hello #{name}!"
end

bench('two vars', N) do
  first = 'John'
  last = 'Doe'
  s = "#{first} #{last}"
end

bench('int var', N) do
  n = 42
  s = "Value: #{n}"
end

bench('float var', N) do
  f = 3.14159
  s = "Pi: #{f}"
end

bench('expression', N) do
  a = 2
  b = 3
  s = "Sum is #{a + b}"
end

bench('method call', N) do
  text = 'hello'
  s = "Upper: #{text.upcase}"
end

bench('array index', N) do
  names = %w[Alice Bob Carol]
  s = "First: #{names[0]}"
end

bench('hash access', N) do
  person = { 'name' => 'Charlie', 'age' => 30 }
  s = "Name: #{person['name']}"
end

bench('many vars (5)', N) do
  a = 'A'
  b = 'B'
  c = 'C'
  d = 'D'
  e = 'E'
  s = "#{a}-#{b}-#{c}-#{d}-#{e}"
end

bench('many vars (10)', N) do
  v1 = 1
  v2 = 2
  v3 = 3
  v4 = 4
  v5 = 5
  v6 = 6
  v7 = 7
  v8 = 8
  v9 = 9
  v10 = 10
  s = "#{v1},#{v2},#{v3},#{v4},#{v5},#{v6},#{v7},#{v8},#{v9},#{v10}"
end

bench('mixed types', N) do
  name = 'Alice'
  age = 30
  score = 98.5
  active = true
  s = "#{name} (#{age}) score=#{score} active=#{active}"
end

bench('long text + vars', N) do
  user = 'Alice'
  count = 42
  s = "Dear #{user}, you have #{count} new messages waiting in your inbox."
end

bench('concat (baseline)', N) do
  name = 'Alice'
  s = 'Hello ' + name + '!'
end

bench('concat many (5)', N) do
  a = 'A'
  b = 'B'
  c = 'C'
  d = 'D'
  e = 'E'
  s = a + '-' + b + '-' + c + '-' + d + '-' + e
end
