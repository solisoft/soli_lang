# String operations - tests string concatenation and manipulation
def build_string(n)
  s = ""
  i = 0
  while i < n
    s = s + "x"
    i = i + 1
  end
  return s
end

def count_chars(s)
  return s.length
end

s = build_string(500)
result = count_chars(s)
