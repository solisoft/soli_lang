# kernels: count_down, first_multiple, collatz_steps, choose, both, either, accumulate, until_five, nested_scopes, ternary, not_zero
def count_down(n: Int) -> Int
  steps = 0
  while n > 0
    n -= 1
    steps += 1
  end
  steps
end

def first_multiple(of: Int, start: Int) -> Int
  i = start
  while true
    break if i % of == 0
    i += 1
  end
  i
end

def collatz_steps(n: Int) -> Int
  steps = 0
  while n != 1
    if n % 2 == 0
      n = n / 2
    else
      n = 3 * n + 1
    end
    steps += 1
  end
  steps
end

def choose(flag: Bool, a: Int, b: Int) -> Int
  unless flag
    return b
  end
  a
end

def both(a: Int, b: Int) -> Int
  a && b
end

def either(a: Int, b: Int) -> Int
  a || b
end

def accumulate(n: Int) -> Int
  total = 0
  i = 1
  while i <= n
    total += i * i
    total %= 1000003
    i += 1
  end
  total
end

def until_five(n: Int) -> Int
  i = 0
  while i < n
    i += 1
    return i if i == 5
  end
  -1
end

def nested_scopes(n: Int) -> Int
  result = 0
  if n > 0
    doubled = n * 2
    result = doubled + 1
  else
    let negated = -n
    result = negated
  end
  result
end

def ternary(n: Int) -> Int
  n > 10 ? n - 10 : n + 10
end

def not_zero(n: Int) -> Bool
  !(n == 0) && !!n
end

print(count_down(5))
print(first_multiple(7, 50))
print(collatz_steps(27))
print(choose(true, 1, 2))
print(choose(false, 1, 2))
print([both(0, 5), both(3, 5)])
print([either(0, 5), either(3, 5)])
print(accumulate(10000))
print(until_five(3))
print(until_five(9))
print(nested_scopes(4))
print(nested_scopes(-4))
print([ternary(15), ternary(5)])
print([not_zero(0), not_zero(7)])
