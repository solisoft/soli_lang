# Iterative fibonacci - tests loop performance
def fib(n)
  if n <= 1
    return n
  end
  a = 0
  b = 1
  i = 2
  while i <= n
    temp = a + b
    a = b
    b = temp
    i = i + 1
  end
  return b
end

result = fib(30)
