# kernels: fib, is_even, is_odd, gcd, ackermann, sign, clamp
def fib(n: Int) -> Int
  return n if n < 2

  fib(n - 1) + fib(n - 2)
end

def is_even(n: Int) -> Bool
  return true if n == 0

  is_odd(n - 1)
end

def is_odd(n: Int) -> Bool
  return false if n == 0

  is_even(n - 1)
end

def gcd(a: Int, b: Int) -> Int
  return a if b == 0

  gcd(b, a % b)
end

def ackermann(m: Int, n: Int) -> Int
  return n + 1 if m == 0
  return ackermann(m - 1, 1) if n == 0

  ackermann(m - 1, ackermann(m, n - 1))
end

def sign(n: Int) -> Int
  if n > 0
    1
  elsif n < 0
    -1
  else
    0
  end
end

def clamp(n: Int, low: Int, high: Int) -> Int
  return low if n < low
  return high if n > high

  n
end

print(fib(20))
print(is_even(10))
print(is_odd(7))
print(gcd(1071, 462))
print(gcd(-48, 18))
print(ackermann(2, 3))
print([sign(-5), sign(0), sign(9)])
print(clamp(15, 0, 10))
print(clamp(-3, 0, 10))
print(clamp(4, 0, 10))
