# kernels: half, flip, truthy_float
# Run with --no-type-check. Some arguments do not match the annotations, so
# those calls decline and the engine does what it always did; and a Float in a
# condition, which the checker refuses, is truthy even at 0.0.
def half(n: Int) -> Int
  n / 2
end

def flip(b: Bool) -> Bool
  !b
end

def truthy_float(x: Float) -> Int
  if x
    1
  else
    0
  end
end

print(half(10))
print(half(7.0) rescue "half rejected a Float")
print(flip(true))
print(flip(0))
print(truthy_float(0.0))
print(truthy_float(-0.0))
