# kernels: series, mixed, divide, remainder, is_less, same, differs, hypot_sq
def series(n: Int) -> Float
  sum = 0.0
  i = 0
  while i < n
    sum = sum + (i % 7) * 1.5
    i = i + 1
  end
  sum
end

def mixed(a: Int, b: Float) -> Float
  a + b * 2 - a / 4
end

def divide(a: Float, b: Float) -> Float
  a / b
end

def remainder(a: Float, b: Float) -> Float
  a % b
end

def is_less(a: Float, b: Float) -> Bool
  a < b
end

def same(a: Int, b: Float) -> Bool
  a == b
end

def differs(a: Float, b: Float) -> Bool
  a != b
end

def hypot_sq(x: Float, y: Float) -> Float
  x * x + y * y
end

print(series(1000))
print(mixed(7, 0.25))
print(divide(1.0, 3.0))
print(divide(-7.5, 2.0))
print(remainder(7.5, 2.0))
print(remainder(-7.5, 2.0))
print(remainder(1.0, 0.0))
print(is_less(1.0, 2.0))
print(is_less(0.0 / 1.0, -0.0))
print(same(3, 3.0))
print(same(3, 3.5))
print(differs(1.5, 1.5))
print(hypot_sq(3.0, 4.0))
