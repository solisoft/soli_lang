# String operations benchmark
def build_string(n)
  s = ""
  i = 0
  while i < n
    s += "x"
    i += 1
  end
  s
end

def count_chars(s)
  s.length
end

s = build_string(500)
result = count_chars(s)

# Warmup
100.times { build_string(500) }

# Benchmark
iterations = 100000
start = Time.now
iterations.times do
  s = build_string(500)
  count_chars(s)
end
elapsed = (Time.now - start) * 1000

puts "Ruby string ops: #{elapsed.round(2)}ms for #{iterations} iterations"