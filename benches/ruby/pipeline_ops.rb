# Pipeline operations - tests pipeline operator overhead
def double(x)
  return x * 2
end

def addOne(x)
  return x + 1
end

def square(x)
  return x * x
end

def transform(x)
  # Ruby doesn't have a pipeline operator, simulate it
  return square(addOne(double(x)))
end

total = 0
i = 0
while i < 1000
  total = total + transform(i)
  i = i + 1
end
result = total
