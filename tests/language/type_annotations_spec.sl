# Type annotations, part one: typed parameters and returns on functions,
# typed class fields, and untyped code alongside typed code. Part two
# (Void, runtime return checks, function/nullable/array types) is
# type_system_spec.sl.

def double(n: Int) -> Int
  n * 2
end

def get_answer -> Int
  42
end

def typed_add(a: Int, b: Int) -> Int
  a + b
end

def concat(text: String, n: Int) -> String
  text + str(n)
end

def is_positive(n: Int) -> Bool
  n > 0
end

def area(radius: Float) -> Float
  radius * radius * 3.14159
end

def greet(name: String) -> String
  "Hello, " + name
end

def identity(value)
  value
end

def get_numbers
  [1, 2, 3]
end

class AnnotatedPerson
  name: String
  age: Int

  new(name: String, age: Int)
    @name = name
    @age = age
  end
end

class AnnotatedConfig
  debug: Bool = false
  version: String = "1.0"
end

class AnnotatedPoint
  x: Float
  y: Float

  new(x: Float, y: Float)
    @x = x
    @y = y
  end
end

class BankAccount
  balance: Int = 0

  def deposit(amount: Int)
    @balance = @balance + amount
  end

  def get_balance -> Int
    @balance
  end
end

class Container
  value: Int = 42
end

describe("Function type annotations") do
  test("a typed parameter and return") do
    assert_eq(double(5), 10)
  end

  test("a return type on a parameterless function") do
    assert_eq(get_answer(), 42)
  end

  test("several parameters of one type") do
    assert_eq(typed_add(3, 4), 7)
  end

  test("parameters of different types") do
    assert_eq(concat("hello", 5), "hello5")
  end

  test("a Bool return") do
    assert_eq(is_positive(5), true)
    assert_eq(is_positive(-1), false)
    assert_eq(is_positive(0), false)
  end

  test("a Float parameter and return") do
    assert_eq(area(1.0), 3.14159)
    assert_eq(area(2.0), 12.56636)
  end

  test("a String parameter and return") do
    assert_eq(greet("World"), "Hello, World")
  end

  test("parameter types are not checked at the call") do
    # Only return types are checked at runtime (see type_system_spec.sl).
    assert_eq(concat("n", 1.5), "n1.5")
  end
end

describe("Class field type annotations") do
  test("typed fields set by the constructor") do
    person = new AnnotatedPerson("Alice", 30)
    assert_eq(person.name, "Alice")
    assert_eq(person.age, 30)
  end

  test("typed fields with default values") do
    config = new AnnotatedConfig()
    assert_eq(config.debug, false)
    assert_eq(config.version, "1.0")
  end

  test("Float fields keep their exact value") do
    point = new AnnotatedPoint(3.5, 4.2)
    assert_eq(point.x, 3.5)
    assert_eq(point.y, 4.2)
  end

  test("a typed method parameter updates a typed field") do
    account = new BankAccount()
    account.deposit(100)
    account.deposit(25)
    assert_eq(account.get_balance, 125)
  end

  test("each instance starts from the field default") do
    first = new BankAccount()
    first.deposit(10)
    second = new BankAccount()
    assert_eq(second.get_balance, 0)
  end

  test("a typed field can be read and reassigned") do
    container = new Container()
    assert_eq(container.value, 42)
    container.value = 100
    assert_eq(container.value, 100)
  end
end

describe("Untyped code") do
  test("variables take the type of their literal") do
    assert_eq(type(42), "int")
    assert_eq(type("hello"), "string")
    assert_eq(type(true), "bool")
    assert_eq(type(1.5), "float")
  end

  test("an untyped function accepts any type") do
    assert_eq(identity(42), 42)
    assert_eq(identity("test"), "test")
    assert_null(identity(nil))
  end

  test("arrays may mix types") do
    mixed = [1, "two", true]
    assert_eq(mixed.map { |item| type(item) }, ["int", "string", "bool"])
  end

  test("a hash may mix value types") do
    config = {"port": 8080, "debug": false, "host": "localhost"}
    assert_eq(config["port"], 8080)
    assert_eq(config["debug"], false)
    assert_eq(config["host"], "localhost")
  end

  test("a for loop accumulates over an array") do
    total = 0
    for n in [1, 2, 3, 4, 5]
      total = total + n
    end
    assert_eq(total, 15)
  end

  test("a function returning a collection") do
    assert_eq(get_numbers(), [1, 2, 3])
  end
end
