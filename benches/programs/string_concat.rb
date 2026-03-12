# String concatenation benchmark (similar to soli program)
ITERATIONS = 100000

def build_string(n)
  s = ""
  i = 0
  while i < n
    s = s + "x"
    i += 1
  end
  s
end

def count_chars(s)
  s.length
end

# Warmup
100.times { build_string(500) }

# Benchmark
start = Process.clock_gettime(Process::CLOCK_MONOTONIC)
ITERATIONS.times do
  s = build_string(500)
  count_chars(s)
end
elapsed = Process.clock_gettime(Process::CLOCK_MONOTONIC) - start

puts "Ruby (concat): #{ (elapsed * 1000).round(2) }ms for #{ITERATIONS} iterations"