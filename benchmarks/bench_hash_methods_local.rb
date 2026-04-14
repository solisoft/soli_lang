def report(label, iterations, elapsed)
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

def run_suite
  size = 1024
  read_n = 50_000
  clone_n = 5_000
  base = build_hash(size, 0)
  other = build_hash(size, size)
  sparse = build_sparse_hash(size)

  puts "=== Focused Hash Methods Local ==="
  puts "size=#{size}"

  start = Process.clock_gettime(Process::CLOCK_MONOTONIC)
  read_n.times { base["key_512"] }
  report("get", read_n, Process.clock_gettime(Process::CLOCK_MONOTONIC) - start)

  start = Process.clock_gettime(Process::CLOCK_MONOTONIC)
  read_n.times { base["key_512"] = 42 }
  report("set existing", read_n, Process.clock_gettime(Process::CLOCK_MONOTONIC) - start)

  start = Process.clock_gettime(Process::CLOCK_MONOTONIC)
  read_n.times { base.key?("key_512") }
  report("has_key", read_n, Process.clock_gettime(Process::CLOCK_MONOTONIC) - start)

  start = Process.clock_gettime(Process::CLOCK_MONOTONIC)
  read_n.times do
    old = base.delete("key_512")
    base["key_512"] = old
  end
  report("delete+restore", read_n, Process.clock_gettime(Process::CLOCK_MONOTONIC) - start)

  start = Process.clock_gettime(Process::CLOCK_MONOTONIC)
  clone_n.times { base.keys }
  report("keys", clone_n, Process.clock_gettime(Process::CLOCK_MONOTONIC) - start)

  start = Process.clock_gettime(Process::CLOCK_MONOTONIC)
  clone_n.times { base.values }
  report("values", clone_n, Process.clock_gettime(Process::CLOCK_MONOTONIC) - start)

  start = Process.clock_gettime(Process::CLOCK_MONOTONIC)
  clone_n.times { base.to_a }
  report("entries", clone_n, Process.clock_gettime(Process::CLOCK_MONOTONIC) - start)

  start = Process.clock_gettime(Process::CLOCK_MONOTONIC)
  clone_n.times { base.merge(other) }
  report("merge", clone_n, Process.clock_gettime(Process::CLOCK_MONOTONIC) - start)

  start = Process.clock_gettime(Process::CLOCK_MONOTONIC)
  clone_n.times { base.invert }
  report("invert", clone_n, Process.clock_gettime(Process::CLOCK_MONOTONIC) - start)

  start = Process.clock_gettime(Process::CLOCK_MONOTONIC)
  clone_n.times { sparse.compact }
  report("compact", clone_n, Process.clock_gettime(Process::CLOCK_MONOTONIC) - start)
end

run_suite
