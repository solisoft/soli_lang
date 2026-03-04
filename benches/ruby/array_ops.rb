# Array operations - tests array creation, access, and iteration
def array_sum(arr)
  total = 0
  arr.each do |x|
    total = total + x
  end
  return total
end

def create_array(n)
  arr = []
  i = 0
  while i < n
    arr.push(i * 2)
    i = i + 1
  end
  return arr
end

arr = create_array(1000)
result = array_sum(arr)
