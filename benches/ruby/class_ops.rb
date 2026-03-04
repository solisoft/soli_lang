# Class operations - tests object creation and method dispatch
class Counter
  def initialize
    @count = 0
  end

  def increment
    @count = @count + 1
  end

  def add(n)
    @count = @count + n
  end

  def getCount
    return @count
  end
end

counter = Counter.new
i = 0
while i < 1000
  counter.increment
  i = i + 1
end
result = counter.getCount
