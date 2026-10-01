# kernels: times_big, quotient, modulo, deep, maybe, fdiv, negate
def times_big(n: Int) -> Int
  n * 9223372036854775807
end

def quotient(a: Int, b: Int) -> Int
  a / b
end

def modulo(a: Int, b: Int) -> Int
  a % b
end

def deep(n: Int) -> Int
  return 0 if n == 0

  1 + deep(n - 1)
end

def maybe(n: Int) -> Int
  if n > 0
    n
  end
end

def fdiv(a: Float, b: Float) -> Float
  a / b
end

def negate(n: Int) -> Int
  -n
end

print(times_big(1))
try
  times_big(2)
catch e
  print("overflow: #{e}")
end
print(quotient(-7, 2))
print(modulo(-7, 2))
try
  quotient(1, 0)
catch e
  print("div: #{e}")
end
try
  modulo(1, 0)
catch e
  print("mod: #{e}")
end
try
  quotient(-9223372036854775807 - 1, -1)
catch e
  print("min/-1: #{e}")
end
try
  modulo(-9223372036854775807 - 1, -1)
catch e
  print("min%-1: #{e}")
end
print(deep(10))
try
  deep(100000)
catch e
  print("deep: #{e}")
end
print(maybe(3))
try
  print(maybe(-3))
catch e
  print("maybe: #{e}")
end
try
  fdiv(1.0, 0.0)
catch e
  print("fdiv: #{e}")
end
print(negate(5))
print(9223372036854775807 > 9223372036854775806)
print(times_big(3))
