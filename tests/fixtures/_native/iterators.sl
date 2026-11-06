# kernels: score, big, add, double
# Iterators handed a function compiled to machine code: the result must not
# depend on whether the kernel or the bytecode runs it.
def score(n: Int) -> Int
  total = 0
  i = 0
  while i < 5
    total = total + (n * i) % 7
    i = i + 1
  end
  total
end

def big(n: Int) -> Bool
  score(n) > 10
end

def add(a: Int, b: Int) -> Int
  a + b
end

def double(n: Int) -> Int
  n * 2
end

xs = [3, 9, 4, 12, 7]
print(xs.map(score))
print(xs.filter(big))
print(xs.reject(big))
print(xs.count(big))
print(xs.find(big))
print(xs.any?(big), xs.all?(big), xs.none?(big), xs.one?(big))
print(xs.sort_by(score))
print(xs.reduce(add, 0))
print(xs.reduce(add))
print(xs.map(double))
print(xs.each_with_index.map { |x, i| add(x, i) })
running = 0
4.times { |i| running = add(running, i) }
print(running)
print(5.times(double))
