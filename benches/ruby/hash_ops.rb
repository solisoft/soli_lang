# Hash operations - tests hash creation, access, and iteration
def create_hash(n)
  h = {}
  i = 0
  while i < n
    h["key" + i.to_s] = i * 2
    i = i + 1
  end
  return h
end

def hash_sum(h)
  total = 0
  h.values.each do |v|
    total = total + v
  end
  return total
end

h = create_hash(500)
result = hash_sum(h)
