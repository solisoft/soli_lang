def bench(label, iterations, &block)
  start = Process.clock_gettime(Process::CLOCK_MONOTONIC)
  iterations.times { block.call }
  elapsed = Process.clock_gettime(Process::CLOCK_MONOTONIC) - start
  per_iter = elapsed / iterations * 1_000_000
  puts "#{label}: #{elapsed.round(6)}s (#{per_iter.round(3)} µs/iter)"
end

def build_hash(size, offset)
  h = {}
  i = 0
  while i < size
    h["key_#{i + offset}"] = i + offset
    i += 1
  end
  h
end

def build_sparse_hash(size)
  h = {}
  i = 0
  while i < size
    h["key_#{i}"] = (i % 4).zero? ? nil : i
    i += 1
  end
  h
end

size = 1024
read_n = 50_000
clone_n = 5_000

base = build_hash(size, 0)
other = build_hash(size, size)
sparse = build_sparse_hash(size)

puts "=== Focused Hash Methods ==="
puts "size=#{size}"

bench("get", read_n) do
  base["key_512"]
end

bench("set existing", read_n) do
  base["key_512"] = 42
end

bench("has_key", read_n) do
  base.key?("key_512")
end

bench("delete+restore", read_n) do
  old = base.delete("key_512")
  base["key_512"] = old
end

bench("keys", clone_n) do
  base.keys
end

bench("values", clone_n) do
  base.values
end

bench("entries", clone_n) do
  base.to_a
end

bench("merge", clone_n) do
  base.merge(other)
end

bench("invert", clone_n) do
  base.invert
end

bench("compact", clone_n) do
  sparse.compact
end
