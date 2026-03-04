# Simple loop sum - tests basic loop overhead
def sum_to(n)
  total = 0
  i = 1
  while i <= n
    total = total + i
    i = i + 1
  end
  return total
end

result = sum_to(10000)
