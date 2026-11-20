# Abstract classes: `abstract class`, bodiless `abstract def`, `.new` refused,
# and a subclass that forgot an abstract method failing loudly. A concrete
# subclass missing one is a static error (`soli check`, the type checker, the
# boot check), so the "forgot" case is reached through an abstract subclass.

abstract class Shape
  abstract def area -> Float
  abstract def name -> String

  def describe -> String
    "#{@name} of area #{@area}"
  end
end

class Square < Shape
  side: Float

  new(side: Float)
    @side = side
  end

  def area -> Float
    @side * @side
  end

  def name -> String
    "square"
  end
end

# Leaves `area` to a subclass of its own.
abstract class Labelled < Shape
  def name -> String
    "labelled"
  end
end

describe("Abstract classes") do
  test("a concrete subclass runs the shared method over its own") do
    assert_eq(Square.new(1.5).describe, "square of area 2.25")
    assert(Square.new(2.0).is_a?("Shape"))
  end

  test("an abstract class cannot be instantiated") do
    assert_raises("cannot instantiate abstract class Shape") do
      Shape.new
    end
    assert_raises("cannot instantiate abstract class Labelled") do
      Labelled.new
    end
  end

  test("abstract stays an ordinary name") do
    let abstract = 3
    assert_eq(abstract + 1, 4)
  end
end
