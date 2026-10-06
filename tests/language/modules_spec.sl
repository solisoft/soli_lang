# The `module` keyword as a namespace and a bag of functions: module-level
# methods, constants, nesting, and what an `include` carries along.
# The mixin basics (include, extend, hooks) live in mixin_modules_spec.sl.

module Geometry
  static const PI = 3
  static UNITS: String = "cm"

  def area(radius)
    Geometry.PI * radius * radius
  end

  def double_area(radius)
    Geometry.area(radius) * 2
  end

  def sibling_area(radius)
    area(radius) * 2
  end

  static def label
    "geometry"
  end

  module Shapes
    def name
      "shapes"
    end

    class Square
      def sides
        4
      end
    end
  end

  class Point
    new(x)
      @x = x
    end

    def x
      @x
    end
  end
end

module Empty
end

class WithEmpty
  include Empty

  def ok
    "ok"
  end
end

class Disc
  include Geometry
end

module Inner
  def inner_name
    "inner"
  end
end

module Outer
  include Inner

  def outer_name
    "outer"
  end
end

class UsesOuter
  include Outer
end

module Shouting
  def shout
    @name.upcase + @suffix
  end

  def describe_kind
    @shout + "/" + @kind
  end
end

module Quiet
  def whisper
    @name.downcase
  end
end

class Speaker
  include Shouting, Quiet

  new(name)
    @name = name
  end

  def suffix
    "!"
  end

  def kind
    "speaker"
  end
end

class Announcer < Speaker
  def kind
    "announcer"
  end
end

module Sided
  const SIDES = 4

  def sides
    @SIDES
  end
end

class Tile
  include Sided
end

describe("Module definition") do
  test("a module's methods are callable on the module") do
    assert_eq(Geometry.area(2), 12)
  end

  test("a module method reaches a sibling through the module name") do
    assert_eq(Geometry.double_area(1), 6)
  end

  test("a module method reaches a sibling by its bare name") do
    pending("bug: a bare sibling call in a module method invoked on the module raises \"Undefined variable 'area'\"")
    assert_eq(Geometry.sibling_area(1), 6)
  end

  test("static def defines a function on the module") do
    assert_eq(Geometry.label, "geometry")
  end

  test("a module is a Class value") do
    assert_eq(type(Geometry), "Class")
    assert_eq(Geometry.to_s, "<class Geometry>")
  end

  test("an unknown module function raises") do
    assert_raises("Cannot access property 'missing' on Geometry") do
      Geometry.missing(1)
    end
  end

  test("an empty module can be included") do
    assert_eq(new WithEmpty().ok, "ok")
  end
end

describe("Constants in modules") do
  test("a static const reads with . and ::") do
    assert_eq(Geometry.PI, 3)
    assert_eq(Geometry::PI, 3)
  end

  test("a static field with a type annotation reads like a constant") do
    assert_eq(Geometry.UNITS, "cm")
  end

  test("a static const refuses reassignment") do
    assert_raises("cannot reassign static const field 'PI'") do
      Geometry.PI = 4
    end
    assert_eq(Geometry.PI, 3)
  end

  test("an instance const reaches the including class") do
    pending("bug: tree engine: a module's `const` field is not copied into the includer")
    assert_eq(new Tile().sides, 4)
  end
end

describe("Nesting") do
  test("a nested module is reached with ::") do
    assert_eq(Geometry::Shapes.name, "shapes")
  end

  test("a class nested in a module is instantiated through ::") do
    assert_eq(new Geometry::Point(7).x, 7)
  end

  test("a class two modules deep is reached through ::") do
    square_class = Geometry::Shapes::Square
    assert_eq(new square_class().sides, 4)
  end

  test("nested names stay out of the global scope") do
    assert_raises("Undefined variable 'Point'") do
      Point
    end
    assert_raises("Undefined variable 'Shapes'") do
      Shapes
    end
  end
end

describe("What include carries") do
  test("instance methods, but not the module's static functions") do
    assert_eq(new Disc().area(1), 3)
    assert_raises("Cannot access property 'label' on Disc") do
      Disc.label
    end
  end

  test("a module included in a module reaches the final class") do
    assert_eq(new UsesOuter().outer_name, "outer")
    assert_eq(new UsesOuter().inner_name, "inner")
  end

  test("one include statement takes several modules") do
    speaker = new Speaker("Ada")
    assert_eq(speaker.shout, "ADA!")
    assert_eq(speaker.whisper, "ada")
  end

  test("an included method calls the host's methods and other included ones") do
    assert_eq(new Speaker("Ada").describe_kind, "ADA!/speaker")
  end

  test("a subclass inherits the included methods and its overrides apply") do
    announcer = new Announcer("Bo")
    assert_eq(announcer.shout, "BO!")
    assert_eq(announcer.describe_kind, "BO!/announcer")
  end

  test("respond_to? sees an included method") do
    assert(new Speaker("x").respond_to?("whisper"))
    assert_not(new Speaker("x").respond_to?("missing"))
  end
end
