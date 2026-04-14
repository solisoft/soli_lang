def bench(label, iterations, &block)
  start = Process.clock_gettime(Process::CLOCK_MONOTONIC)
  iterations.times { block.call }
  elapsed = Process.clock_gettime(Process::CLOCK_MONOTONIC) - start
  per_iter = elapsed / iterations * 1_000_000
  puts "#{label}: #{elapsed.round(6)}s (#{per_iter.round(3)} µs/iter)"
end

N = 10_000

puts '=== Array ==='

bench('map', N) do
  a = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
  a.map { |x| x * 2 }
end

bench('filter', N) do
  a = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
  a.select { |x| x > 5 }
end

bench('reduce', N) do
  a = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
  a.reduce(0) { |acc, x| acc + x }
end

bench('sort', N) do
  a = [5, 3, 8, 1, 9, 2, 7, 4, 6, 10]
  a.sort
end

bench('each', N) do
  a = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
  s = 0
  a.each { |x| s += x }
end

bench('join', N) do
  a = %w[a b c d e f g h i j]
  a.join(', ')
end

bench('reverse', N) do
  a = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
  a.reverse
end

bench('uniq', N) do
  a = [1, 1, 2, 2, 3, 3, 4, 4, 5, 5]
  a.uniq
end

bench('flatten', N) do
  a = [[1, 2], [3, 4], [5, 6], [7, 8], [9, 10]]
  a.flatten
end

bench('push/pop', N) do
  a = [1, 2, 3, 4, 5]
  a.push(6)
  a.pop
end

puts ''
puts '=== Hash ==='

bench('get', N) do
  h = { 'a' => 1, 'b' => 2, 'c' => 3, 'd' => 4, 'e' => 5 }
  h['c']
end

bench('set', N) do
  h = { 'a' => 1, 'b' => 2, 'c' => 3 }
  h['d'] = 4
end

bench('keys', N) do
  h = { 'a' => 1, 'b' => 2, 'c' => 3, 'd' => 4, 'e' => 5 }
  h.keys
end

bench('values', N) do
  h = { 'a' => 1, 'b' => 2, 'c' => 3, 'd' => 4, 'e' => 5 }
  h.values
end

bench('merge', N) do
  h1 = { 'a' => 1, 'b' => 2, 'c' => 3 }
  h2 = { 'd' => 4, 'e' => 5, 'f' => 6 }
  h1.merge(h2)
end

bench('has_key', N) do
  h = { 'a' => 1, 'b' => 2, 'c' => 3, 'd' => 4, 'e' => 5 }
  h.key?('c')
end

bench('delete', N) do
  h = { 'a' => 1, 'b' => 2, 'c' => 3, 'd' => 4, 'e' => 5 }
  h.delete('c')
end

bench('entries', N) do
  h = { 'a' => 1, 'b' => 2, 'c' => 3, 'd' => 4, 'e' => 5 }
  h.to_a
end

bench('invert', N) do
  h = { 'a' => 1, 'b' => 2, 'c' => 3, 'd' => 4, 'e' => 5 }
  h.invert
end

bench('compact', N) do
  h = { 'a' => 1, 'b' => nil, 'c' => 3, 'd' => nil, 'e' => 5 }
  h.compact
end

puts ''
puts '=== String ==='

bench('length', N) do
  s = 'hello, world!'
  s.length
end

bench('upcase', N) do
  s = 'hello, world!'
  s.upcase
end

bench('downcase', N) do
  s = 'HELLO, WORLD!'
  s.downcase
end

bench('reverse', N) do
  s = 'hello, world!'
  s.reverse
end

bench('split', N) do
  s = 'a,b,c,d,e,f,g,h,i,j'
  s.split(',')
end

bench('replace', N) do
  s = 'hello, world!'
  s.gsub('world', 'ruby')
end

bench('trim', N) do
  s = '  hello, world!  '
  s.strip
end

bench('contains', N) do
  s = 'hello, world!'
  s.include?('world')
end

bench('starts_with?', N) do
  s = 'hello, world!'
  s.start_with?('hello')
end

bench('ends_with?', N) do
  s = 'hello, world!'
  s.end_with?('world!')
end

bench('concat', N) do
  s = 'hello'
  s + ', world!'
end
