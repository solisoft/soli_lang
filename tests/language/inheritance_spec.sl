# Class inheritance: what a subclass inherits (methods, fields, constructors,
# static methods), overriding, super chains, and `this` in inherited code.
# Each test declares its own small hierarchy.

describe("Inheritance basics") do
  test("a subclass inherits the constructor and fields, and overrides a method") do
    class Animal
      name: String

      new(name: String)
        @name = name
      end

      def speak -> String
        "..."
      end
    end

    class Dog < Animal
      def speak -> String
        "Woof!"
      end
    end

    dog = new Dog("Buddy")
    assert_eq(dog.name, "Buddy")
    assert_eq(dog.speak, "Woof!")
    assert_eq(new Animal("Generic").speak, "...")
  end

  test("a subclass adds fields and methods next to the inherited ones") do
    class Shape
      def description -> String
        "A shape"
      end
    end

    class Circle < Shape
      radius: Float

      new(radius: Float)
        @radius = radius
      end

      def area -> Float
        3.14159 * @radius * @radius
      end
    end

    circle = new Circle(5.0)
    assert_eq(circle.description, "A shape")
    assert_eq(circle.area, 78.53975)
  end

  test("methods come from every level of a chain") do
    class ChainA
      def method_a -> String
        "A"
      end
    end

    class ChainB < ChainA
      def method_b -> String
        "B"
      end
    end

    class ChainC < ChainB
      def method_c -> String
        "C"
      end
    end

    leaf = new ChainC()
    assert_eq(leaf.method_a, "A")
    assert_eq(leaf.method_b, "B")
    assert_eq(leaf.method_c, "C")
  end

  test("< and extends declare the same thing") do
    class Awesome
      def greet -> String
        "awesome"
      end
    end

    class ViaLess < Awesome
    end

    class ViaExtends extends Awesome
    end

    assert_eq(new ViaLess().greet, "awesome")
    assert_eq(new ViaExtends().greet, "awesome")
  end

  test("type() and .class name the most derived class") do
    class TypeBase
    end

    class TypeDerived < TypeBase
    end

    derived = new TypeDerived()
    assert_eq(type(derived), "TypeDerived")
    assert_eq(derived.class, "TypeDerived")
  end

  test("is_a? sees every ancestor, not descendants") do
    class Vehicle
    end

    class Car < Vehicle
    end

    assert(new Car().is_a?("Car"))
    assert(new Car().is_a?("Vehicle"))
    assert_not(new Vehicle().is_a?("Car"))
  end

  test("an unknown method still raises on a subclass") do
    class Quiet
    end

    class Quieter < Quiet
    end

    assert_raises("Cannot access property 'shout' on Quieter") do
      new Quieter().shout
    end
  end
end

describe("super") do
  test("super.method in an override") do
    class Greeting
      def greet -> String
        "Hello"
      end
    end

    class LoudGreeting < Greeting
      def greet -> String
        super.greet + " World"
      end
    end

    assert_eq(new LoudGreeting().greet, "Hello World")
  end

  test("an inherited field default is read through @") do
    class Defaults
      value: Int = 10
    end

    class MoreDefaults < Defaults
      def get_value -> Int
        @value
      end
    end

    assert_eq(new MoreDefaults().get_value, 10)
  end

  test("super(args) in a constructor runs the parent's") do
    class Person
      name: String

      new(name: String)
        @name = name
      end
    end

    class Employee < Person
      employee_id: Int

      new(name: String, id: Int)
        super(name)
        @employee_id = id
      end
    end

    employee = new Employee("Alice", 123)
    assert_eq(employee.name, "Alice")
    assert_eq(employee.employee_id, 123)
  end

  test("super chains through three levels") do
    class GrandParent
      def identify -> String
        "GrandParent"
      end
    end

    class Parent < GrandParent
      def identify -> String
        super.identify + " -> Parent"
      end
    end

    class Child < Parent
      def identify -> String
        super.identify + " -> Child"
      end
    end

    assert_eq(new Child().identify, "GrandParent -> Parent -> Child")
  end

  test("a static override calls the parent's static method by name") do
    class NamedBase
      static def get_class_name -> String
        "Base"
      end
    end

    class NamedDerived < NamedBase
      static def get_class_name -> String
        NamedBase.get_class_name + "_Derived"
      end
    end

    assert_eq(NamedDerived.get_class_name, "Base_Derived")
    assert_eq(NamedBase.get_class_name, "Base")
  end

  test("a static method is inherited") do
    class Logger
      static def level -> String
        "INFO"
      end
    end

    class DebugLogger < Logger
    end

    assert_eq(DebugLogger.level, "INFO")
  end

  test("super values compose across levels") do
    class Level1
      def level -> Int
        1
      end
    end

    class Level2 < Level1
      def level -> Int
        super.level + 10
      end
    end

    class Level3 < Level2
      def level -> Int
        super.level + 100
      end
    end

    assert_eq(new Level3().level, 111)
    assert_eq(new Level2().level, 11)
  end
end

describe("this in inherited code") do
  test("an inherited method dispatches through this to the subclass override") do
    class Speaker
      def speak
        "..."
      end

      def intro
        "I say " + this.speak
      end
    end

    class Barker < Speaker
      def speak
        "Woof"
      end
    end

    assert_eq(new Barker().intro, "I say Woof")
    assert_eq(new Speaker().intro, "I say ...")
  end

  test("inherited chaining methods return the subclass instance") do
    class Chainer
      value: Int = 0

      def add(n: Int)
        @value = @value + n
        this
      end

      def multiply(n: Int)
        @value = @value * n
        this
      end

      def reset
        @value = 0
        this
      end
    end

    class SubChainer < Chainer
    end

    chainer = new SubChainer()
    assert_eq(chainer.add(5).multiply(2).value, 10)
    assert_eq(chainer.reset.add(3).value, 3)
    assert_eq(type(chainer.add(1)), "SubChainer")
  end

  test("an inherited constructor sets the subclass instance's fields") do
    class Box
      width: Int
      height: Int
      depth: Int

      new(width: Int, height: Int, depth: Int)
        @width = width
        @height = height
        @depth = depth
      end
    end

    class Crate < Box
      def volume -> Int
        @width * @height * @depth
      end
    end

    assert_eq(new Crate(2, 3, 4).volume, 24)
  end

  test("an inherited method reads a field the subclass constructor set") do
    class NameBase
      name: String = "default"

      def get_name -> String
        @name
      end
    end

    class NameMiddle < NameBase
    end

    class NameLeaf < NameMiddle
      new
        @name = "leaf"
      end
    end

    assert_eq(new NameLeaf().get_name, "leaf")
    assert_eq(new NameMiddle().get_name, "default")
  end

  test("this in a static method raises") do
    class StaticThis
      static def bad
        this
      end
    end

    assert_raises("'this' outside of class") do
      StaticThis.bad
    end
  end
end

describe("Method overriding") do
  test("an override replaces the parent's method for the subclass only") do
    class Original
      def get_value -> Int
        1
      end
    end

    class Replacement < Original
      def get_value -> Int
        2
      end
    end

    assert_eq(new Original().get_value, 1)
    assert_eq(new Replacement().get_value, 2)
  end

  test("an override builds on super's result") do
    class Doubler
      def compute(x: Int) -> Int
        x * 2
      end
    end

    class DoublerPlusOne < Doubler
      def compute(x: Int) -> Int
        super.compute(x) + 1
      end
    end

    assert_eq(new DoublerPlusOne().compute(5), 11)
  end

  test("an override may change the signature") do
    class Processor
      def process(data: String) -> String
        "processed: " + data
      end
    end

    class PrefixProcessor < Processor
      def process(data: String, prefix: String) -> String
        prefix + ": " + data
      end
    end

    assert_eq(new Processor().process("test"), "processed: test")
    assert_eq(new PrefixProcessor().process("test", "p"), "p: test")
    assert_raises("Wrong number of arguments: expected 2, got 1") do
      new PrefixProcessor().process("test")
    end
  end

  test("a subclass adds new methods") do
    class Existing
      def existing -> String
        "exists"
      end
    end

    class Extended < Existing
      def new_method -> String
        "new"
      end
    end

    extended = new Extended()
    assert_eq(extended.existing, "exists")
    assert_eq(extended.new_method, "new")
    assert_raises("Cannot access property 'new_method' on Existing") do
      new Existing().new_method
    end
  end
end

describe("Constructor behavior") do
  test("a class without new gets a default constructor and field defaults") do
    class Simple
      value: Int = 42
    end

    assert_eq(new Simple().value, 42)
  end

  test("a custom constructor") do
    class Rectangle
      width: Int
      height: Int

      new(width: Int, height: Int)
        @width = width
        @height = height
      end

      def area -> Int
        @width * @height
      end
    end

    assert_eq(new Rectangle(5, 3).area, 15)
  end

  test("a constructor with default parameters, inherited by a subclass") do
    class Cuboid
      width: Int
      height: Int
      depth: Int

      new(width: Int, height: Int = 1, depth: Int = 1)
        @width = width
        @height = height
        @depth = depth
      end

      def volume -> Int
        @width * @height * @depth
      end
    end

    class Cube < Cuboid
    end

    assert_eq(new Cuboid(2).volume, 2)
    assert_eq(new Cuboid(2, 3).volume, 6)
    assert_eq(new Cuboid(2, 3, 4).volume, 24)
    assert_eq(new Cube(2, 2).volume, 4)
  end
end

describe("Multi-level inheritance") do
  test("a leaf override and a middle-level addition") do
    class Controller
      def action -> String
        "Controller"
      end
    end

    class BaseController < Controller
      def before -> String
        "authenticated"
      end
    end

    class HomeController < BaseController
      def action -> String
        "home"
      end
    end

    home = new HomeController()
    assert_eq(home.action, "home")
    assert_eq(home.before, "authenticated")
  end

  test("super chaining across three controller levels") do
    class RootController
      def action -> String
        "base"
      end
    end

    class AppController < RootController
      def action -> String
        super.action + " -> base_ctrl"
      end
    end

    class PageController < AppController
      def action -> String
        super.action + " -> home"
      end
    end

    assert_eq(new PageController().action, "base -> base_ctrl -> home")
  end

  test("fields from three levels") do
    class FieldA
      x: Int = 1
    end

    class FieldB < FieldA
      y: Int = 2
    end

    class FieldC < FieldB
      z: Int = 3
    end

    leaf = new FieldC()
    assert_eq([leaf.x, leaf.y, leaf.z], [1, 2, 3])
  end

  test("a constructor inherited through two constructor-less levels") do
    class Root
      name: String

      new(name: String)
        @name = name
      end
    end

    class Middle < Root
    end

    class Leaf < Middle
    end

    assert_eq(new Leaf("hello").name, "hello")
  end

  test("a four-level super chain") do
    class L1
      def id -> String
        "L1"
      end
    end

    class L2 < L1
      def id -> String
        super.id + ".L2"
      end
    end

    class L3 < L2
      def id -> String
        super.id + ".L3"
      end
    end

    class L4 < L3
      def id -> String
        super.id + ".L4"
      end
    end

    assert_eq(new L4().id, "L1.L2.L3.L4")
  end

  test("a leaf calls helpers the middle class added") do
    class EmptyRoot
    end

    class HelperController < EmptyRoot
      def layout -> String
        "application"
      end

      def current_user -> String
        "admin"
      end
    end

    class PostsController < HelperController
      def index -> String
        @current_user + " - " + @layout
      end
    end

    assert_eq(new PostsController().index, "admin - application")
  end

  test("a middle override is what the leaf inherits") do
    class GreetA
      def greet -> String
        "A"
      end

      def farewell -> String
        "bye from A"
      end
    end

    class GreetB < GreetA
      def greet -> String
        "B"
      end
    end

    class GreetC < GreetB
    end

    leaf = new GreetC()
    assert_eq(leaf.greet, "B")
    assert_eq(leaf.farewell, "bye from A")
  end

  test("a static method inherited through three levels") do
    class StaticA
      static def class_type -> String
        "A"
      end
    end

    class StaticB < StaticA
    end

    class StaticC < StaticB
    end

    assert_eq(StaticC.class_type, "A")
  end

  test("type() names each level's own class") do
    class TypeA
    end

    class TypeB < TypeA
    end

    class TypeC < TypeB
    end

    assert_eq(type(new TypeC()), "TypeC")
    assert_eq(type(new TypeB()), "TypeB")
    assert_eq(type(new TypeA()), "TypeA")
  end
end
