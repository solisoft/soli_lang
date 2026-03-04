# Deep inheritance benchmark - tests method lookup with inheritance chain

class Base
  def baseMethod
    return 1
  end
end

class Level1 < Base
  def level1Method
    return 2
  end
end

class Level2 < Level1
  def level2Method
    return 3
  end
end

class Level3 < Level2
  def level3Method
    return 4
  end
end

class Level4 < Level3
  def level4Method
    return 5
  end
end

# Final class with deep inheritance
class DeepClass < Level4
  def deepMethod
    return self.baseMethod + self.level1Method + self.level2Method + self.level3Method + self.level4Method
  end
end

obj = DeepClass.new
sum = 0
i = 0
while i < 1000
  # Each iteration calls methods from all levels of inheritance
  sum = sum + obj.deepMethod
  i = i + 1
end
result = sum
