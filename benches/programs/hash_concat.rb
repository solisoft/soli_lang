# Hash operations benchmark (similar to soli program)
ITERATIONS = 100000

def create_hash(n)
  h = {}
  i = 0
  while i < n
    h["key" + i.to_s] = i * 2
    i += 1
  end
  h
end

def hash_sum(h)
  total = 0
  for v in h.values
    total += v
  end
  total
end

# Warmup
100.times { create_hash(100) }

# Benchmark
start = Process.clock_gettime(Process::CLOCK_MONOTONIC)
ITERATIONS.times do
  h = create_hash(100)
  hash_sum(h)
end
elapsed = Process.clock_gettime(Process::CLOCK_MONOTONIC) - start

puts "Ruby (hash): #{ (elapsed * 1000).round(2) }ms for #{ITERATIONS} iterations"