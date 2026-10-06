# Metaprogramming: respond_to?, send, method_missing, instance variable
# introspection, instance_eval / class_eval, define_method, alias_method and
# the inherited hook.

class MetaFoo
  def greet(name)
    "Hello, " + name + "!"
  end

  def method_missing(name)
    "Method '" + name + "' was called"
  end
end

class MetaRecorder
  def method_missing(name, args)
    "#{name} called with #{args.length} argument(s)"
  end
end

class MetaNamed
  name: String
  value: Int = 10
end

class MetaCounter
  count: Int = 0

  def total
    1
  end
end

class MetaStatic
  static label: String = "FooClass"
  static value: Int = 42
end

class MetaCalculator
end

class MetaBar
end

class MetaGreeter
  def say
    "hello"
  end
end

class MetaAliasFresh
  def say
    "hello"
  end
end

class MetaAliasDefined
end

class MetaAliasAfterUse
  def say
    "hello"
  end
end

inherited_log = []

class MetaParent
  static def inherited(child)
    inherited_log.push(child)
  end
end

class MetaChild < MetaParent
end

describe("respond_to?") do
  test("is true for a defined method") do
    assert_eq(MetaFoo.new().respond_to?("greet"), true)
  end

  test("is false for an undefined method, even with method_missing") do
    assert_eq(MetaFoo.new().respond_to?("nonexistent"), false)
  end

  test("is true for built-in methods") do
    foo = MetaFoo.new()
    assert_eq(foo.respond_to?("inspect"), true)
    assert_eq(foo.respond_to?("class"), true)
  end
end

describe("send") do
  test("calls a method by name with arguments") do
    assert_eq(MetaFoo.new().send("greet", "World"), "Hello, World!")
  end

  test("falls back to method_missing") do
    assert_eq(MetaFoo.new().send("foobar"), "Method 'foobar' was called")
  end
end

describe("method_missing") do
  test("is called for an undefined method") do
    assert_eq(MetaFoo.new().undefined_method(), "Method 'undefined_method' was called")
  end

  test("a last parameter named args collects every argument") do
    recorder = new MetaRecorder()
    assert_eq(recorder.anything(1, 2, 3), "anything called with 3 argument(s)")
    assert_eq(recorder.anything(), "anything called with 0 argument(s)")
  end
end

describe("instance_variables") do
  test("is empty for a fresh instance") do
    assert_eq(MetaFoo.new().instance_variables, [])
  end

  test("lists the variables set on the instance, with @") do
    foo = MetaFoo.new()
    foo._name = "test"
    foo._count = 42
    assert_eq(foo.instance_variables.sort(), ["@_count", "@_name"])
  end
end

describe("instance_variable_get") do
  test("reads an existing variable") do
    foo = MetaFoo.new()
    foo._name = "test_value"
    assert_eq(foo.instance_variable_get("@_name"), "test_value")
  end

  test("returns nil for a missing variable") do
    assert_null(MetaFoo.new().instance_variable_get("@_nonexistent"))
  end

  test("accepts a name without @") do
    foo = MetaFoo.new()
    foo._name = "test_value"
    assert_eq(foo.instance_variable_get("_name"), "test_value")
  end
end

describe("instance_variable_set") do
  test("sets a variable and returns the value") do
    foo = MetaFoo.new()
    assert_eq(foo.instance_variable_set("@_name", "set_value"), "set_value")
    assert_eq(foo.instance_variable_get("@_name"), "set_value")
    assert_eq(foo._name, "set_value")
  end

  test("accepts a name without @") do
    foo = MetaFoo.new()
    foo.instance_variable_set("_count", 42)
    assert_eq(foo.instance_variable_get("@_count"), 42)
  end
end

describe("methods") do
  test("lists user methods first, then the built-in ones") do
    methods = MetaFoo.new().methods
    assert_eq(methods.take(2).sort(), ["greet", "method_missing"])
    assert_contains(methods, "respond_to?")
    assert_contains(methods, "send")
    assert_contains(methods, "inspect")
  end
end

describe("instance_eval") do
  test("binds this, self and @ to the instance") do
    named = new MetaNamed()
    named.name = "Test"
    assert_eq(named.instance_eval { this.name }, "Test")
    assert_eq(named.instance_eval { self.name }, "Test")
    assert_eq(named.instance_eval { @name }, "Test")
  end

  test("reads a field default") do
    assert_eq(new MetaNamed().instance_eval { self.value }, 10)
  end

  test("can modify instance state") do
    counter = new MetaCounter()
    counter.instance_eval do
      @count = 42
    end
    assert_eq(counter.count, 42)
  end
end

describe("class_eval") do
  test("binds self to the class") do
    assert_eq(MetaStatic.class_eval { self.label }, "FooClass")
  end

  test("binds this to the class") do
    assert_eq(MetaStatic.class_eval { this.value }, 42)
  end
end

describe("define_method") do
  test("defines a method on the instance's class") do
    foo = MetaFoo.new()
    foo.define_method("greet2", fn() { "Hello!" })
    assert_eq(foo.respond_to?("greet2"), true)
    assert_eq(foo.greet2(), "Hello!")
  end

  test("the defined method takes arguments") do
    calculator = MetaCalculator.new()
    calculator.define_method("add", fn(a, b) { a + b })
    assert_eq(calculator.add(1, 2), 3)
  end

  test("the method reaches every instance of the class") do
    first = MetaBar.new()
    second = MetaBar.new()
    first.define_method("hello", fn() { "Hi!" })
    assert_eq(second.hello(), "Hi!")
  end

  test("on a class, a zero-argument method auto-invokes") do
    MetaGreeter.define_method("shout", fn() { "HELLO!" })
    greeter = MetaGreeter.new()
    assert_eq(greeter.shout, "HELLO!")
    assert_eq(greeter.say, "hello")
  end

  test("on a primitive type, this is the receiver") do
    Int.define_method("meta_spec_doubled", fn() { this * 2 })
    assert_eq(3.meta_spec_doubled, 6)
  end
end

describe("alias_method") do
  test("creates an alias for an existing method") do
    foo = MetaFoo.new()
    foo.alias_method("say_hello", "greet")
    assert_eq(foo.respond_to?("say_hello"), true)
    assert_eq(foo.say_hello("World"), "Hello, World!")
  end

  test("the original keeps working beside the alias") do
    counter = MetaCounter.new()
    counter.alias_method("counter", "total")
    assert_eq(counter.counter(), 1)
    assert_eq(counter.total, 1)
  end

  test("on a class, the alias reaches its instances") do
    assert_null(MetaAliasFresh.alias_method("greeting", "say"))
    assert_eq(MetaAliasFresh.new().greeting, "hello")
  end

  test("on a class, a method added by define_method can be aliased") do
    pending("bug: Class.alias_method of a define_method'd method returns nil, but instances cannot call it")
    MetaAliasDefined.define_method("shout", fn() { "HELLO!" })
    MetaAliasDefined.alias_method("yell", "shout")
    assert_eq(MetaAliasDefined.new().yell, "HELLO!")
  end

  test("on a class, an alias added after its methods ran reaches new instances") do
    pending("bug: once an instance method ran, Class.alias_method no longer reaches new instances")
    assert_eq(MetaAliasAfterUse.new().say, "hello")
    MetaAliasAfterUse.alias_method("greeting", "say")
    assert_eq(MetaAliasAfterUse.new().greeting, "hello")
  end

  test("aliasing a missing method returns an error message and defines nothing") do
    foo = MetaFoo.new()
    assert_eq(foo.alias_method("fake", "nonexistent"), "alias_method: method 'nonexistent' not found")
    assert_eq(foo.respond_to?("fake"), false)
  end
end

describe("inherited") do
  test("runs once when a class inherits, receiving the child class") do
    assert_eq(inherited_log.length, 1)
    assert_eq(inherited_log[0], MetaChild)
  end
end

describe("Class.new") do
  test("with parentheses builds an instance") do
    assert_eq(MetaGreeter.new().say, "hello")
  end

  test("without parentheses builds an instance") do
    pending("bug: `MetaGreeter.new` without parens returns the constructor Function (and is a type error in a script)")
    greeter = MetaGreeter.new
    assert_eq(greeter.say, "hello")
  end
end
