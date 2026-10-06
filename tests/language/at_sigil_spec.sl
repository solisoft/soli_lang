# `@foo` is sugar for `this.foo`: a field read or write, or a method call on
# self, inside any instance method or constructor.

class Box
  value: Int

  new(value: Int)
    this.value = value
  end

  def peek -> Int
    @value
  end
end

class Counter
  n: Int

  new
    @n = 0
  end

  def bump
    @n = @n + 1
  end

  def get -> Int
    @n
  end
end

class Twin
  x: Int

  new
    @x = 0
  end

  def via_at(value: Int)
    @x = value
  end

  def via_this -> Int
    this.x
  end

  def this_write(value: Int)
    this.x = value
  end

  def at_read -> Int
    @x
  end
end

class Greeter
  name: String

  new(name: String)
    @name = name
  end

  def hello -> String
    "Hello, " + @name_upcase()
  end

  def hello_bare -> String
    "Hello, " + @name_upcase
  end

  def name_upcase -> String
    @name.upcase
  end

  def add(a, b)
    a + b
  end

  def sum
    @add(1, 2)
  end

  def short?
    @name.length < 4
  end

  def asks
    @short?
  end
end

class Acc
  total: Int

  new
    @total = 10
  end

  def add(n: Int)
    @total += n
  end
end

class Inner
  label: String

  new(label: String)
    @label = label
  end
end

class Outer
  inner: Any

  new(inner)
    @inner = inner
  end

  def inner_label -> String
    @inner.label
  end
end

class Bag
  items: Array

  new
    @items = ["a", "b", "c"]
  end

  def second -> String
    @items[1]
  end

  def push(item: String)
    @items.push(item)
  end
end

class Tagged
  tag: String

  new(tag: String)
    @tag = tag
  end
end

class TaggedChild < Tagged
  new(tag: String)
    super(tag)
  end

  def wrapped -> String
    "<" + @tag + ">"
  end
end

class Log
  entries: Array

  new
    @entries = []
  end

  def add(line: String)
    @entries.push(line)
  end

  def count -> Int
    @entries.length
  end
end

class Cache
  value: Any
  calls: Int = 0

  def get -> Int
    @value = @compute if @value.nil?
    @value
  end

  def memo
    @memoized ||= @compute
  end

  def compute
    @calls += 1
    42
  end
end

class MaybeSet
  slot: Any

  def peek -> Any
    @slot
  end
end

class Scaler
  items: Array

  new
    @items = [1, 2, 3]
    @factor = 10
  end

  def scaled
    @items.map { |x| x * @factor }
  end

  def with_lambda
    times = fn(x) { x * @factor }
    times(3)
  end
end

def outside_any_class
  @value
end

describe("@ sigil") do
  context("fields") do
    test("@foo reads the field this.foo set") do
      assert_eq(new Box(7).peek, 7)
    end

    test("@foo = x writes the field") do
      counter = new Counter()
      counter.bump
      counter.bump
      counter.bump
      assert_eq(counter.get, 3)
    end

    test("@foo and this.foo are the same field, both ways") do
      twin = new Twin()
      twin.via_at(42)
      assert_eq(twin.via_this, 42)
      twin.this_write(7)
      assert_eq(twin.at_read, 7)
    end

    test("@foo += n compound-assigns") do
      acc = new Acc()
      acc.add(5)
      acc.add(3)
      assert_eq(acc.total, 18)
    end

    test("@foo.bar chains member access") do
      assert_eq(new Outer(new Inner("nested")).inner_label, "nested")
    end

    test("@foo[k] indexes, and @foo.push mutates the field") do
      bag = new Bag()
      assert_eq(bag.second, "b")
      bag.push("d")
      assert_eq(bag.items, ["a", "b", "c", "d"])
    end

    test("reads a field the parent's constructor set") do
      assert_eq(new TaggedChild("hi").wrapped, "<hi>")
    end

    test("a mutation persists across method calls") do
      log = new Log()
      log.add("one")
      log.add("two")
      log.add("three")
      assert_eq(log.count, 3)
      assert_eq(log.entries, ["one", "two", "three"])
    end

    test("a parameter of the same name does not shadow the field") do
      assert_eq(new Greeter("soli").name, "soli")
    end

    test("reads nil for a declared field never set") do
      assert_null(new MaybeSet().peek)
    end

    test("an undeclared field can be assigned in the constructor") do
      assert_eq(new Scaler().factor, 10)
    end
  end

  context("lazy initialization") do
    test("@foo = x if @foo.nil? computes once") do
      cache = new Cache()
      assert_eq(cache.get, 42)
      assert_eq(cache.get, 42)
      assert_eq(cache.calls, 1)
    end

    test("@foo ||= x computes once") do
      cache = new Cache()
      assert_eq(cache.memo, 42)
      assert_eq(cache.memo, 42)
      assert_eq(cache.calls, 1)
    end
  end

  context("method calls") do
    test("@foo() calls an instance method") do
      assert_eq(new Greeter("ada").hello, "Hello, ADA")
    end

    test("@foo without parentheses calls it too") do
      assert_eq(new Greeter("ada").hello_bare, "Hello, ADA")
    end

    test("@foo(args) passes arguments") do
      assert_eq(new Greeter("ada").sum, 3)
    end

    test("@foo? calls a predicate method") do
      assert(new Greeter("ada").asks)
      assert_not(new Greeter("grace").asks)
    end
  end

  context("closures") do
    test("a block inside a method sees @foo") do
      assert_eq(new Scaler().scaled, [10, 20, 30])
    end

    test("a lambda inside a method sees @foo") do
      assert_eq(new Scaler().with_lambda, 30)
    end
  end

  test("outside any class it raises") do
    assert_raises("'this' outside of class") do
      outside_any_class()
    end
  end
end
