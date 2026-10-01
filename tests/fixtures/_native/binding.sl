# kernels: early, later, twice
def early(n: Int) -> Int
  later(n) + 1
end

try
  print(early(1))
catch e
  print("before later exists: #{e}")
end

def later(n: Int) -> Int
  n * 10
end

print(early(1))

def twice(n: Int) -> Int
  n * 2
end

alias = twice
print(alias(21))

# Refused: these still run, interpreted.
def shout(n: Int) -> Int
  print("calling shout")
  n
end

def again(n: Int) -> Int
  n + 1
end

def again(n: Int) -> Int
  n + 2
end

print(shout(1))
print(again(1))
