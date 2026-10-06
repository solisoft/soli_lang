# Interfaces: declaration, `implements` / `~`, combined with a superclass.
# A missing method is a static error (`soli check`, the type checker), not a
# runtime one, so it is not exercised here.

interface Greetable {
  fn greet()
}

interface Crud {
  fn create()
  fn read()
  fn update(data)
  fn delete()
}

interface Calculator {
  fn add(a, b)
  fn multiply(a, b)
}

interface Blank {
}

interface Printable {
  fn print()
}

interface Loggable {
  fn log()
}

interface Serializable {
  fn serialize()
}

interface Incrementable {
  fn increment()
}

interface Summable {
  fn sum(a: Int, b: Int) -> Int
}

interface Formatter {
  fn format(name: String, age: Int) -> String
}

interface Drawable {
  fn draw() -> String
}

interface Resizable {
  def resize(factor: Float) -> Float
}

class Person implements Greetable
  def greet
    "Hello"
  end
end

class SimpleStore implements Crud
  data: String = ""

  def create
    @data = "created"
  end

  def read
    @data
  end

  def update(data)
    @data = data
  end

  def delete
    @data = ""
  end
end

class SimpleCalc implements Calculator
  def add(a, b)
    a + b
  end

  def multiply(a, b)
    a * b
  end
end

class BlankImpl implements Blank
  def ping
    "pong"
  end
end

class Document implements Printable
  content: String

  new(content: String)
    @content = content
  end

  def print
    @content
  end
end

class Datum implements Loggable, Serializable
  value: Int

  new(value: Int)
    @value = value
  end

  def log
    "Data: " + str(@value)
  end

  def serialize
    "{\"value\":" + str(@value) + "}"
  end
end

class Base
  base_value: Int = 10
end

class Derived < Base implements Incrementable
  def increment
    @base_value = @base_value + 1
  end
end

class Adder implements Summable
  def sum(a: Int, b: Int) -> Int
    a + b
  end
end

class PersonFormatter implements Formatter
  def format(name: String, age: Int) -> String
    name + " is " + str(age) + " years old"
  end
end

class Circle implements Drawable
  def draw -> String
    "Circle"
  end
end

class Square implements Drawable
  def draw -> String
    "Square"
  end
end

class Greeter ~ Greetable
  def greet
    "Hi"
  end
end

class Box ~ Drawable, Resizable
  size: Float = 1.0

  def draw -> String
    "Box"
  end

  def resize(factor: Float) -> Float
    @size = @size * factor
  end
end

class Animal
  def breathe -> String
    "breathing"
  end
end

class Dog extends Animal ~ Greetable
  def greet
    "woof"
  end
end

class Cat < Animal ~ Greetable
  def greet
    "meow"
  end
end

describe("Interface declaration") do
  test("a single method signature") do
    assert_eq(new Person().greet, "Hello")
  end

  test("several method signatures") do
    store = new SimpleStore()
    store.create
    assert_eq(store.read, "created")
    store.update("updated")
    assert_eq(store.read, "updated")
    store.delete
    assert_eq(store.read, "")
  end

  test("untyped parameters") do
    calc = new SimpleCalc()
    assert_eq(calc.add(3, 4), 7)
    assert_eq(calc.multiply(2, 5), 10)
  end

  test("an empty interface puts no constraint on the class") do
    assert_eq(new BlankImpl().ping, "pong")
  end

  test("a signature may be written with def") do
    assert_eq(new Box().resize(2.0), 2.0)
  end
end

describe("Interface implementation") do
  test("a class implements one interface") do
    assert_eq(new Document("Hello World").print, "Hello World")
  end

  test("a class implements several interfaces") do
    datum = new Datum(42)
    assert_eq(datum.log, "Data: 42")
    assert_eq(datum.serialize, "{\"value\":42}")
  end

  test("a superclass and an interface together") do
    derived = new Derived()
    assert_eq(derived.base_value, 10)
    assert_eq(derived.increment, 11)
    assert_eq(derived.increment, 12)
  end

  test("a signature with a return type") do
    adder = new Adder()
    assert_eq(adder.sum(3, 4), 7)
    assert_eq(adder.sum(10, 20), 30)
  end

  test("a signature with typed parameters and a return type") do
    formatter = new PersonFormatter()
    assert_eq(formatter.format("Alice", 30), "Alice is 30 years old")
    assert_eq(formatter.format("Bob", 25), "Bob is 25 years old")
  end

  test("different classes behind one interface are used alike") do
    shapes = [new Circle(), new Square()]
    assert_eq(shapes.map { |shape| shape.draw }, ["Circle", "Square"])
  end

  test("an interface is not a runtime type for is_a?") do
    assert_eq(new Person().is_a?("Person"), true)
    assert_eq(new Person().is_a?("Greetable"), false)
  end
end

describe("The ~ shorthand") do
  test("replaces implements for a single interface") do
    assert_eq(new Greeter().greet, "Hi")
  end

  test("accepts a comma-separated list") do
    box = new Box()
    assert_eq(box.draw, "Box")
    assert_eq(box.resize(3.0), 3.0)
    assert_eq(box.resize(2.0), 6.0)
  end

  test("combines with extends") do
    dog = new Dog()
    assert_eq(dog.breathe, "breathing")
    assert_eq(dog.greet, "woof")
  end

  test("combines with <") do
    cat = new Cat()
    assert_eq(cat.breathe, "breathing")
    assert_eq(cat.greet, "meow")
  end
end
