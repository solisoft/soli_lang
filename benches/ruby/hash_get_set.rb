# Self-timed hash get/set — paired with benches/programs/hash_get_set.sl

ITERS = 200_000

def build(n)
  keys = []
  h = {}
  i = 0
  while i < n
    key = "k" + i.to_s
    keys << key
    h[key] = i
    i += 1
  end
  [keys, h]
end

def bench_get_index(n)
  keys, h = build(n)
  j = 0
  total = 0
  start = Process.clock_gettime(Process::CLOCK_MONOTONIC)
  while j < ITERS
    k = j % n
    total += h[keys[k]]
    j += 1
  end
  (Process.clock_gettime(Process::CLOCK_MONOTONIC) - start) * 1000
end

def bench_get_method(n)
  keys, h = build(n)
  j = 0
  total = 0
  start = Process.clock_gettime(Process::CLOCK_MONOTONIC)
  while j < ITERS
    k = j % n
    total += h.fetch(keys[k])
    j += 1
  end
  (Process.clock_gettime(Process::CLOCK_MONOTONIC) - start) * 1000
end

def bench_set_index(n)
  keys, h = build(n)
  j = 0
  start = Process.clock_gettime(Process::CLOCK_MONOTONIC)
  while j < ITERS
    k = j % n
    h[keys[k]] = j
    j += 1
  end
  (Process.clock_gettime(Process::CLOCK_MONOTONIC) - start) * 1000
end

def bench_set_method(n)
  keys, h = build(n)
  j = 0
  start = Process.clock_gettime(Process::CLOCK_MONOTONIC)
  while j < ITERS
    k = j % n
    h.store(keys[k], j)
    j += 1
  end
  (Process.clock_gettime(Process::CLOCK_MONOTONIC) - start) * 1000
end

puts "hash_get_set (#{ITERS} iterations)"
[4, 8, 16, 64, 256].each do |n|
  puts "  get_index/#{n}: #{bench_get_index(n)}ms"
  puts "  get_method/#{n}: #{bench_get_method(n)}ms"
  puts "  set_index/#{n}: #{bench_set_index(n)}ms"
  puts "  set_method/#{n}: #{bench_set_method(n)}ms"
end
