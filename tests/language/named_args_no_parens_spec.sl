# Ruby-style command calls with named arguments and no parentheses:
# `greet name: "Alice"`, `user.update name: "Bob", age: 25`.

def greet(name: String)
  "Hello, " + name + "!"
end

def configure(host: String, port: Int, debug: Bool)
  host + ":" + str(port) + " (debug: " + str(debug) + ")"
end

def add(a: Int, b: Int, c: Int)
  a + b + c
end

class User
  name: String
  age: Int

  new()
    @name = "default"
    @age = 0
  end

  def update(name: String, age: Int)
    @name = name
    @age = age
    "updated"
  end
end

describe("Named args without parentheses") do
  context("on a function") do
    test("a single named arg") do
      result = greet name: "Alice"
      assert_eq(result, "Hello, Alice!")
    end

    test("several comma-separated named args") do
      result = configure host: "example.com", port: 3000, debug: true
      assert_eq(result, "example.com:3000 (debug: true)")
      total = add a: 1, b: 2, c: 3
      assert_eq(total, 6)
    end

    test("named args in any order") do
      result = configure port: 3000, debug: false, host: "x"
      assert_eq(result, "x:3000 (debug: false)")
    end

    test("gives the same result as the parenthesized call") do
      bare = greet name: "Ann"
      assert_eq(bare, greet(name: "Ann"))
    end
  end

  context("on a method") do
    test("returns the method's value and binds every argument") do
      user = new User()
      result = user.update name: "Bob", age: 25
      assert_eq(result, "updated")
      assert_eq(user.name, "Bob")
      assert_eq(user.age, 25)
    end

    test("as a statement, with args in any order") do
      user = new User()
      user.update age: 30, name: "Cy"
      assert_eq(user.name, "Cy")
      assert_eq(user.age, 30)
    end
  end
end
