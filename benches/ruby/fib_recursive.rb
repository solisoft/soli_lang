# Recursive fibonacci - tests function call overhead
def fib(n)
  if n <= 1
    return n
  end
  return fib(n - 1) + fib(n - 2)
end

result = fib(20)
