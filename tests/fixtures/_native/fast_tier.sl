# kernels: addf, bump, half
# Kernel calls from a VM loop, which runs in the fast tier: answered by the
# kernel, or declined (an argument of another type, an overflow) and run
# as bytecode, by name and through a local.
def addf(a: Float, b: Float) -> Float
  a + b
end

def bump(n: Int) -> Int
  n + 1
end

def half(n: Int) -> Int
  n / 2
end

def drive(count)
  total = 0.0
  ints = 0
  add = addf
  i = 0
  while i < count
    total = total + addf(1.5, 0.25)
    total = total + addf(i, 0.5)
    total = total + add(0.25, 0.25)
    ints = ints + bump(i) + half(i)
    i = i + 1
  end
  [total, ints]
end

print(drive(10))
print(bump(9223372036854775807)) rescue print("overflow raised")
print(half(7))
