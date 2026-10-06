# Classes: fields, methods, constructors, static methods (every spelling),
# static and const fields, visibility, nested classes, and keyword method names.
# NOTE: this file spells static methods as `def self.x`, `fn self.x` and
# `class << self` on purpose — `soli fmt` rewrites those to `static def`, so
# do not run the formatter over it.

describe("Basic classes") do
  test("declaration and instantiation") do
    class Point
      x: Int
      y: Int

      new(x: Int, y: Int)
        @x = x
        @y = y
      end
    end

    point = new Point(3, 4)
    assert_eq(point.x, 3)
    assert_eq(point.y, 4)
  end

  test("instance methods") do
    class Rectangle
      width: Int
      height: Int

      new(width: Int, height: Int)
        @width = width
        @height = height
      end

      def area
        @width * @height
      end

      def perimeter
        2 * (@width + @height)
      end
    end

    rect = new Rectangle(5, 3)
    assert_eq(rect.area, 15)
    assert_eq(rect.perimeter, 16)
  end

  test("default field values, and each instance has its own") do
    class Counter
      count: Int = 0

      def increment
        @count = @count + 1
      end
    end

    counter = new Counter()
    other = new Counter()
    assert_eq(counter.count, 0)
    counter.increment
    counter.increment
    assert_eq(counter.count, 2)
    assert_eq(other.count, 0)
  end

  test("a field is writable from outside") do
    class Settable
      label: String = "before"
    end

    settable = new Settable()
    settable.label = "after"
    assert_eq(settable.label, "after")
  end

  test("method chaining by returning this") do
    class Builder
      value: String = ""

      def add(text: String)
        @value = @value + text
        this
      end

      def build
        @value
      end
    end

    assert_eq(new Builder().add("Hello").add(" ").add("World").build, "Hello World")
  end

  test("a wrong argument count to a constructor raises") do
    class Pair
      left: Int
      right: Int

      new(left: Int, right: Int)
        @left = left
        @right = right
      end
    end

    assert_raises("Wrong number of arguments: expected 2, got 3") do
      new Pair(1, 2, 3)
    end
  end

  test("classes compare with ==, and .class names the class") do
    class EqAccount
    end

    class EqOther
    end

    klass = EqAccount
    assert(klass == EqAccount)
    assert(EqAccount == EqAccount)
    assert(klass != EqOther)
    record = new EqAccount()
    assert_eq(record.class, "EqAccount")
    assert(record.class == EqAccount)
    assert(record.class != EqOther)
  end
end

describe("Static methods") do
  test("called on the class without an instance") do
    class MathUtils
      static def random
        42
      end
    end

    assert_eq(MathUtils.random, 42)
  end

  test("with typed parameters and a return type") do
    class MathUtils
      static def add(a: Int, b: Int) -> Int
        a + b
      end

      static def multiply(a: Float, b: Float) -> Float
        a * b
      end
    end

    assert_eq(MathUtils.add(5, 3), 8)
    assert_eq(MathUtils.multiply(3.0, 4.0), 12.0)
  end

  test("are inherited") do
    class StaticBase
      static def greet -> String
        "Hello"
      end
    end

    class StaticDerived < StaticBase
    end

    assert_eq(StaticDerived.greet, "Hello")
  end

  test("can be overridden") do
    class StaticBase
      static def get_value -> Int
        10
      end
    end

    class StaticDerived < StaticBase
      static def get_value -> Int
        20
      end
    end

    assert_eq(StaticDerived.get_value, 20)
    assert_eq(StaticBase.get_value, 10)
  end

  test("call each other through the class name") do
    class MathOps
      static def double(x: Int) -> Int
        x * 2
      end

      static def quadruple(x: Int) -> Int
        MathOps.double(MathOps.double(x))
      end
    end

    assert_eq(MathOps.quadruple(5), 20)
  end

  test("a static factory builds instances") do
    class Vec
      x: Float
      y: Float

      new(x: Float, y: Float)
        @x = x
        @y = y
      end

      static def origin -> Vec
        new Vec(0.0, 0.0)
      end

      static def unit -> Vec
        new Vec(1.0, 1.0)
      end
    end

    origin = Vec.origin
    assert_eq([origin.x, origin.y], [0.0, 0.0])
    unit = Vec.unit
    assert_eq([unit.x, unit.y], [1.0, 1.0])
  end

  test("are not callable on an instance") do
    class OnlyStatic
      static def tool
        "tool"
      end
    end

    assert_raises("Cannot access property 'tool' on OnlyStatic") do
      new OnlyStatic().tool
    end
  end

  test("def self.foo declares one") do
    class SelfUtils
      def self.random
        42
      end
    end

    assert_eq(SelfUtils.random, 42)
  end

  test("def self.foo with parameters and a return type") do
    class SelfUtils
      def self.add(a: Int, b: Int) -> Int
        a + b
      end
    end

    assert_eq(SelfUtils.add(5, 3), 8)
  end

  test("fn self.foo declares one too") do
    class SelfCalculator
      fn self.multiply(a: Float, b: Float) -> Float
        a * b
      end
    end

    assert_eq(SelfCalculator.multiply(3.0, 4.0), 12.0)
  end

  test("static def self.foo is still one static method") do
    class SelfUtils
      static def self.cube(x: Int) -> Int
        x * x * x
      end
    end

    assert_eq(SelfUtils.cube(3), 27)
  end

  test("def self.foo, static def and instance methods mix") do
    class Box
      value: Int

      new(value: Int)
        @value = value
      end

      def self.zero -> Box
        new Box(0)
      end

      static def one -> Box
        new Box(1)
      end

      def get -> Int
        @value
      end
    end

    assert_eq(Box.zero.get, 0)
    assert_eq(Box.one.get, 1)
    assert_eq(new Box(7).get, 7)
  end
end

describe("class << self block") do
  test("declares a static method") do
    class SingletonUtils
      class << self
        def square(x: Int) -> Int
          x * x
        end
      end
    end

    assert_eq(SingletonUtils.square(4), 16)
  end

  test("groups several static methods") do
    class SingletonUtils
      class << self
        def square(x: Int) -> Int
          x * x
        end

        def cube(x: Int) -> Int
          x * x * x
        end
      end
    end

    assert_eq(SingletonUtils.square(3), 9)
    assert_eq(SingletonUtils.cube(3), 27)
  end

  test("accepts fn as well as def") do
    class SingletonGreeter
      class << self
        fn hello -> String
          "hi"
        end
      end
    end

    assert_eq(SingletonGreeter.hello, "hi")
  end

  test("its methods are static only, next to instance methods") do
    class SingletonCounter
      value: Int = 0

      def increment
        @value = @value + 1
      end

      class << self
        def zero -> Int
          0
        end

        def one -> Int
          1
        end
      end
    end

    counter = new SingletonCounter()
    counter.increment
    counter.increment
    assert_eq(counter.value, 2)
    assert_eq(SingletonCounter.zero, 0)
    assert_eq(SingletonCounter.one, 1)
    assert_raises("Cannot access property 'zero'") do
      counter.zero
    end
  end

  test("its methods call each other through the class name") do
    class Math2
      class << self
        def double(x: Int) -> Int
          x * 2
        end

        def quadruple(x: Int) -> Int
          Math2.double(Math2.double(x))
        end
      end
    end

    assert_eq(Math2.quadruple(5), 20)
  end

  test("sits alongside a plain static def") do
    class Mixed
      static def outside -> Int
        1
      end

      class << self
        def inside -> Int
          2
        end
      end
    end

    assert_eq(Mixed.outside, 1)
    assert_eq(Mixed.inside, 2)
  end
end

describe("Static fields") do
  test("declared with a type and an initial value") do
    class StaticConfig
      static count: Int = 0
      static debug: Bool = false
      static version: String = "1.0.0"
      static pi: Float = 3.14159
    end

    assert_eq(StaticConfig.count, 0)
    assert_eq(StaticConfig.debug, false)
    assert_eq(StaticConfig.version, "1.0.0")
    assert_eq(StaticConfig.pi, 3.14159)
  end

  test("mutated with Class.field = value") do
    class MutableConfig
      static debug: Bool = false
    end

    MutableConfig.debug = true
    assert_eq(MutableConfig.debug, true)
  end

  test("a subclass shares its parent's static field") do
    class SharedBase
      static instance_count: Int = 0
    end

    class SharedDerived < SharedBase
    end

    assert_eq(SharedDerived.instance_count, 0)
    SharedBase.instance_count = 5
    assert_eq(SharedDerived.instance_count, 5)
  end

  test("holds class-level state between calls") do
    class IdGenerator
      static next_id: Int = 1

      static def generate -> Int
        id = IdGenerator.next_id
        IdGenerator.next_id = IdGenerator.next_id + 1
        id
      end
    end

    assert_eq(IdGenerator.generate, 1)
    assert_eq(IdGenerator.generate, 2)
    assert_eq(IdGenerator.generate, 3)
  end

  test("an instance does not see a static field") do
    class StaticOnly
      static total: Int = 3
    end

    assert_raises("Cannot access property 'total'") do
      new StaticOnly().total
    end
  end
end

describe("Private and protected visibility") do
  test("a private field is reachable on self only") do
    class Secret
      private password: String = "secret"

      def get_password
        @password
      end
    end

    secret = new Secret()
    assert_eq(secret.get_password, "secret")
    assert_raises("private field 'password' accessed for an instance of Secret") do
      secret.password
    end
    assert_raises("private field 'password' accessed for an instance of Secret") do
      secret.password = "stolen"
    end
  end

  test("a private method is callable on self only") do
    class Container
      private

      def compute -> Int
        42
      end

      public

      def get_value
        @compute
      end
    end

    container = new Container()
    assert_eq(container.get_value, 42)
    assert_raises("private method 'compute' called for an instance of Container") do
      container.compute
    end
  end

  test("a protected field is reachable from the class, refused from outside") do
    class Guarded
      protected value: Int = 10

      def value_of(other)
        other.value
      end
    end

    guarded = new Guarded()
    assert_eq(guarded.value_of(new Guarded()), 10)
    assert_raises("protected field 'value'") do
      guarded.value
    end
  end

  test("a protected method is reachable from its class, refused from outside") do
    class Internal
      protected

      def internal_method -> String
        "internal"
      end

      public

      def peek(other)
        other.internal_method
      end
    end

    internal = new Internal()
    assert_eq(internal.peek(new Internal()), "internal")
    assert_raises("protected method 'internal_method' called for an instance of Internal") do
      internal.internal_method
    end
  end

  test("private fields with defaults keep state across calls") do
    class SafeBox
      private code: String = "1234"
      private attempts: Int = 0

      def try_code(input: String) -> Bool
        @attempts = @attempts + 1
        @code == input
      end

      def get_attempts -> Int
        @attempts
      end
    end

    box = new SafeBox()
    assert_eq(box.try_code("0000"), false)
    assert_eq(box.try_code("1234"), true)
    assert_eq(box.get_attempts, 2)
  end

  test("one class mixes private, protected and public fields") do
    class Member
      private id: Int
      protected email: String
      name: String

      new(id: Int, email: String, name: String)
        @id = id
        @email = email
        @name = name
      end

      def get_id -> Int
        @id
      end
    end

    member = new Member(1, "test@example.com", "Alice")
    assert_eq(member.get_id, 1)
    assert_eq(member.name, "Alice")
    assert_raises("private field 'id'") do
      member.id
    end
    assert_raises("protected field 'email'") do
      member.email
    end
  end

  test("a private static field works from static methods") do
    class SecureCounter
      private static counter: Int = 0

      static def increment
        SecureCounter.counter = SecureCounter.counter + 1
      end

      static def get_count -> Int
        SecureCounter.counter
      end
    end

    assert_eq(SecureCounter.get_count, 0)
    SecureCounter.increment
    SecureCounter.increment
    assert_eq(SecureCounter.get_count, 2)
  end

  test("a private static field is refused from outside") do
    pending("bug: `private static` fields are readable and writable from outside the class")
    class HiddenCounter
      private static counter: Int = 0
    end

    assert_raises("private") do
      HiddenCounter.counter
    end
  end
end

describe("Native static methods") do
  test("DateTime.now returns the current DateTime") do
    before = datetime_now()
    now = DateTime.now
    after = datetime_now()
    assert_eq(type(now), "DateTime")
    assert(now.to_unix >= before)
    assert(now.to_unix <= after)
  end

  test("DateTime.parse parses an ISO string") do
    parsed = DateTime.parse("2024-01-15T10:30:00Z")
    assert_eq(type(parsed), "DateTime")
    assert_eq(parsed.to_unix, 1705314600)
    assert_eq(parsed.year, 2024)
  end

  test("DateTime.parse raises on garbage") do
    assert_raises() do
      DateTime.parse("garbage")
    end
  end

  test("Duration.of_seconds / of_minutes / of_hours") do
    assert_eq(type(Duration.of_seconds(120)), "Duration")
    assert_eq(Duration.of_seconds(120).total_seconds, 120)
    assert_eq(Duration.of_minutes(5).total_seconds, 300)
    assert_eq(Duration.of_hours(2).total_seconds, 7200)
  end

  test("Duration.between is signed, later minus earlier") do
    earlier = DateTime.parse("2024-01-15T10:00:00Z")
    later = DateTime.parse("2024-01-15T11:30:00Z")
    assert_eq(Duration.between(earlier, later).total_seconds, 5400)
    assert_eq(Duration.between(later, earlier).total_seconds, -5400)
  end
end

describe("Constructor named parameters") do
  test("every argument named") do
    class Account
      name: String
      age: Int
      active: Bool

      new(name: String = "Guest", age: Int = 0, active: Bool = true)
        @name = name
        @age = age
        @active = active
      end
    end

    account = new Account(name: "Alice", age: 30, active: false)
    assert_eq([account.name, account.age, account.active], ["Alice", 30, false])
    guest = new Account()
    assert_eq([guest.name, guest.age, guest.active], ["Guest", 0, true])
  end

  test("positional then named, skipping a default") do
    class ConnectionConfig
      host: String
      port: Int
      ssl: Bool
      debug: Bool

      new(host: String, port: Int = 80, ssl: Bool = false, debug: Bool = false)
        @host = host
        @port = port
        @ssl = ssl
        @debug = debug
      end
    end

    config = new ConnectionConfig("example.com", ssl: true)
    assert_eq(config.host, "example.com")
    assert_eq(config.port, 80)
    assert_eq(config.ssl, true)
    assert_eq(config.debug, false)
  end

  test("named arguments in any order") do
    class Server
      name: String
      port: Int
      workers: Int

      new(name: String, port: Int = 8080, workers: Int = 4)
        @name = name
        @port = port
        @workers = workers
      end
    end

    server = new Server("api-server", workers: 8)
    assert_eq([server.name, server.port, server.workers], ["api-server", 8080, 8])
    swapped = new Server(workers: 2, name: "jobs", port: 9000)
    assert_eq([swapped.name, swapped.port, swapped.workers], ["jobs", 9000, 2])
  end

  test("a duplicate named argument raises") do
    class Coord
      x: Int
      y: Int

      new(x: Int = 0, y: Int = 0)
        @x = x
        @y = y
      end
    end

    assert_raises("duplicate named argument 'x'") do
      new Coord(x: 5, x: 10)
    end
  end

  test("an unknown argument name raises") do
    class Circle
      radius: Int

      new(radius: Int = 1)
        @radius = radius
      end
    end

    assert_raises("'diameter'") do
      new Circle(diameter: 10)
    end
  end
end

describe("Nested classes") do
  test("instantiated through Outer::Inner") do
    class Outer
      class Inner
        def greet
          "Hello from Inner"
        end
      end
    end

    inner = new Outer::Inner()
    assert_eq(inner.greet, "Hello from Inner")
    assert_eq(type(inner), "Inner")
  end

  test("several nested classes in one outer class") do
    class Service
      class Database
        def connect
          "DB connected"
        end
      end

      class Cache
        def get(key)
          "cached:" + key
        end
      end
    end

    assert_eq(new Service::Database().connect, "DB connected")
    assert_eq(new Service::Cache().get("test"), "cached:test")
  end

  test("a nested class does not shadow a top-level class of the same name") do
    class Item
      name: String

      new(name)
        @name = name
      end
    end

    class Container
      class Item
        def create_value
          42
        end
      end
    end

    assert_eq(new Container::Item().create_value, 42)
    assert_eq(new Item("top").name, "top")
  end

  test("a nested class uses its outer class and a sibling by full name") do
    class Parent
      def get_name
        "Parent"
      end

      class Child
        def introduce
          "Child of " + new Parent().get_name
        end

        def sibling
          new Parent::Sibling().hi
        end
      end

      class Sibling
        def hi
          "sibling"
        end
      end
    end

    child = new Parent::Child()
    assert_eq(child.introduce, "Child of Parent")
    assert_eq(child.sibling, "sibling")
  end

  test("an unknown nested name raises") do
    class Shell
    end

    assert_raises("Cannot access property 'Missing' on Shell") do
      new Shell::Missing()
    end
  end
end

describe("Const fields") do
  test("an instance const") do
    class MaxConfig
      const MAX_LENGTH = 500
    end

    assert_eq(new MaxConfig().MAX_LENGTH, 500)
  end

  test("static consts") do
    class Message
      static const TYPE_REPLY = "reply"
      static const TYPE_FORWARD = "forward"
    end

    assert_eq(Message.TYPE_REPLY, "reply")
    assert_eq(Message.TYPE_FORWARD, "forward")
  end

  test("consts with a type annotation") do
    class Limits
      const MAX_SIZE: Int = 1000
      static const VERSION: String = "2.0"
    end

    assert_eq(new Limits().MAX_SIZE, 1000)
    assert_eq(Limits.VERSION, "2.0")
  end

  test("an instance const cannot be reassigned") do
    class Immutable
      const VALUE = 42
    end

    immutable = new Immutable()
    assert_raises("cannot reassign const field 'VALUE'") do
      immutable.VALUE = 100
    end
    assert_eq(immutable.VALUE, 42)
  end

  test("a static const cannot be reassigned") do
    class Constants
      static const PI = 3.14159
    end

    assert_raises("cannot reassign static const field 'PI'") do
      Constants.PI = 0
    end
    assert_eq(Constants.PI, 3.14159)
  end

  test("an instance const reads through @ in a method") do
    class LimitSettings
      const LIMIT = 10

      def get_limit
        @LIMIT
      end
    end

    assert_eq(new LimitSettings().get_limit, 10)
  end

  test("a static const reads through the class name in a static method") do
    class MaxCounter
      static const MAX = 100

      static def get_max
        MaxCounter.MAX
      end
    end

    assert_eq(MaxCounter.get_max, 100)
  end

  test("const and mutable instance fields side by side") do
    class Product
      const CATEGORY = "default"
      name: String = "unnamed"

      new(name: String)
        @name = name
      end
    end

    product = new Product("Widget")
    assert_eq(product.CATEGORY, "default")
    assert_eq(product.name, "Widget")
    product.name = "Updated"
    assert_eq(product.name, "Updated")
  end

  test("static const and static mutable fields side by side") do
    class Registry
      static const TYPE = "singleton"
      static count: Int = 0
    end

    assert_eq(Registry.TYPE, "singleton")
    assert_eq(Registry.count, 0)
    Registry.count = 5
    assert_eq(Registry.count, 5)
  end
end

describe("A method named new") do
  test("def new is an ordinary method; calling the class builds an instance") do
    class AppsController
      def new(req)
        "new form"
      end
    end

    controller = AppsController()
    assert_eq(controller.new("GET"), "new form")
  end

  test("fn new is an ordinary method too") do
    class UsersController
      fn new(req)
        "create user form"
      end
    end

    assert_eq(new UsersController().new("GET"), "create user form")
  end

  test("sits among other action methods") do
    class ItemsController
      def index(req)
        "list"
      end

      def new(req)
        "new form"
      end

      def create(req)
        "created"
      end

      def show(req)
        "detail"
      end
    end

    controller = ItemsController()
    assert_eq(controller.index("GET"), "list")
    assert_eq(controller.new("GET"), "new form")
    assert_eq(controller.create("POST"), "created")
    assert_eq(controller.show("GET"), "detail")
  end

  test("coexists with a constructor") do
    class Widget
      name: String

      new(name: String)
        @name = name
      end

      def new(req)
        "new " + @name + " form"
      end
    end

    widget = new Widget("button")
    assert_eq(widget.name, "button")
    assert_eq(widget.new("GET"), "new button form")
  end
end

describe("A method named match") do
  test("is an ordinary method") do
    class Validator
      def match(pattern)
        "matched: " + pattern
      end
    end

    assert_eq(new Validator().match("^[a-z]+$"), "matched: ^[a-z]+$")
  end

  test("String#match returns the matches, or nil") do
    assert_eq("hello".match("^[a-z]+$"), ["hello"])
    assert_null("HELLO".match("^[a-z]+$"))
  end

  test("does not break the match expression") do
    class Checker
      def match(input)
        "checking: " + input
      end
    end

    assert_eq(new Checker().match("test"), "checking: test")
    answer = 42
    result = match answer {
      42 => "forty-two",
      _ => "other",
    }
    assert_eq(result, "forty-two")
  end
end
