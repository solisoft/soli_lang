# Array operations benchmark (similar to soli program)
ITERATIONS = 100000

def array_sum(arr)
  total = 0
  for x in arr
    total = total + x
  end
  total
end

def create_array(n)
  arr = []
  i = 0
  while i < n
    arr.push(i * 2)
    i = i + 1
  end
  arr
end

arr = create_array(1000)
result = array_sum(arr)

# Warmup
100.times { create_array(1000) }

# Benchmark
start = Process.clock_gettime(Process::CLOCK_MONOTONIC)
ITERATIONS.times do
  arr = create_array(1000)
  array_sum(arr)
end
elapsed = Process.clock_gettime(Process::CLOCK_MONOTONIC) - start

puts "Ruby (array): #{ (elapsed * 1000).round(2) }ms for #{ITERATIONS} iterations"