# `this` (the receiver) and `super` (the parent's method or constructor).
# This file is about the keywords themselves, so it writes `this.` where other
# specs would write `@`.

class ThisCounter
  value: Int = 0

  def get_value
    this.value
  end

  def increment
    this.value = this.value + 1
  end
end

class ThisPoint
  x: Int
  y: Int

  new(x: Int, y: Int)
    this.x = x
    this.y = y
  end
end

class ThisContainer
  value: Int = 42

  def via_lambda
    helper = fn() { this.value }
    helper()
  end

  def via_block
    [1, 2].map { |n| this.value + n }
  end
end

class Chainer
  value: Int = 0

  def add(n: Int)
    this.value = this.value + n
    this
  end

  def multiply(n: Int)
    this.value = this.value * n
    this
  end

  def same?(other)
    this == other
  end
end

class Animal
  name: String

  new(name)
    this.name = name
  end

  def speak
    "sound"
  end
end

class Dog < Animal
  def speak
    super.speak + " bark"
  end
end

class Puppy < Dog
  new(name)
    super(name)
  end
end

class Base
  value: Int

  new(value)
    this.value = value
  end
end

class Derived < Base
  extra: Int

  new(a, b)
    super(a)
    this.extra = b
  end
end

class AfterSuper < Base
  new(value)
    super(value)
    this.value = this.value * 10
  end
end

class Adder
  def add(a: Int, b: Int) -> Int
    a + b
  end
end

class Multiplier < Adder
  def multiply(a: Int, b: Int) -> Int
    super.add(a, b) * 2
  end
end

class Level1
  def get_name
    "Level1"
  end
end

class Level2 < Level1
  def get_name
    super.get_name + " -> Level2"
  end
end

class Level3 < Level2
  def get_name
    super.get_name + " -> Level3"
  end
end

class FieldBase
  value: Int = 100
end

class FieldDerived < FieldBase
  def get_base_value
    super.value
  end
end

class Parent
  def greet
    "Hello"
  end
end

class Child < Parent
  def greet
    super.greet + ", Child!"
  end

  def greet_verbose
    this.greet
  end
end

class Orphan
  def call_up
    super.anything
  end
end

class EmptyParentChild < Parent
  def call_missing
    super.missing
  end
end

describe("this") do
  test("reads and writes a field in an instance method") do
    counter = new ThisCounter()
    assert_eq(counter.get_value, 0)
    counter.increment
    assert_eq(counter.get_value, 1)
  end

  test("sets fields in a constructor") do
    point = new ThisPoint(3, 4)
    assert_eq(point.x, 3)
    assert_eq(point.y, 4)
  end

  test("a lambda inside a method captures it") do
    assert_eq(new ThisContainer().via_lambda, 42)
  end

  test("a block inside a method captures it") do
    assert_eq(new ThisContainer().via_block, [43, 44])
  end

  test("returning this allows chaining") do
    chainer = new Chainer()
    chainer.add(5).multiply(2)
    assert_eq(chainer.value, 10)
  end

  test("is the receiver itself, not a copy") do
    chainer = new Chainer()
    assert(chainer.add(1) == chainer)
    assert(chainer.same?(chainer))
    assert_not(chainer.same?(new Chainer()))
  end
end

describe("super") do
  test("super.method calls the parent's version") do
    assert_eq(new Dog("rex").speak, "sound bark")
  end

  test("super(args) runs the parent constructor") do
    derived = new Derived(10, 20)
    assert_eq(derived.value, 10)
    assert_eq(derived.extra, 20)
  end

  test("statements after super(args) see the fields it set") do
    assert_eq(new AfterSuper(4).value, 40)
  end

  test("a class without a constructor inherits its parent's") do
    assert_eq(new Dog("rex").name, "rex")
  end

  test("super(args) skips a parent with no constructor") do
    pending("bug: tree engine: super(args) via a constructor-less parent skips the grandparent constructor")
    assert_eq(new Puppy("bit").name, "bit")
  end

  test("super.other_method calls a different parent method") do
    assert_eq(new Multiplier().multiply(3, 4), 14)
  end

  test("chains through several levels") do
    assert_eq(new Level3().get_name, "Level1 -> Level2 -> Level3")
  end

  test("super.field reads an inherited field") do
    assert_eq(new FieldDerived().get_base_value, 100)
  end

  test("a method the parent lacks raises") do
    assert_raises("Cannot access property 'missing' on Parent") do
      new EmptyParentChild().call_missing
    end
  end

  test("a class with no superclass raises") do
    assert_raises("class has no superclass") do
      new Orphan().call_up
    end
  end
end

describe("this and super combined") do
  test("this.method dispatches to the override, which uses super") do
    assert_eq(new Child().greet_verbose, "Hello, Child!")
  end
end
