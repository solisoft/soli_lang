# Array operations benchmark
def array_sum(arr)
  total = 0
  arr.each do |x|
    total += x
  end
  total
end

def create_array(n)
  arr = []
  i = 0
  while i < n
    arr << i * 2
    i += 1
  end
  arr
end

arr = create_array(1000)
result = array_sum(arr)

# Warmup
100.times { create_array(1000) }

# Benchmark
iterations = 100000
start = Time.now
iterations.times do
  arr = create_array(1000)
  array_sum(arr)
end
elapsed = (Time.now - start) * 1000

puts "Ruby array ops: #{elapsed.round(2)}ms for #{iterations} iterations"