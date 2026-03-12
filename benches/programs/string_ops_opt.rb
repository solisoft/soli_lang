# Optimized string operations benchmark
ITERATIONS = 100000

def build_string(n)
  'x' * n
end

def count_chars(s)
  s.length
end

# Warmup
1000.times { build_string(500) }

# Benchmark
start = Process.clock_gettime(Process::CLOCK_MONOTONIC)
ITERATIONS.times do
  s = build_string(500)
  count_chars(s)
end
elapsed = Process.clock_gettime(Process::CLOCK_MONOTONIC) - start

puts "Ruby optimized: #{ (elapsed * 1000).round(2) }ms for #{ITERATIONS} iterations"