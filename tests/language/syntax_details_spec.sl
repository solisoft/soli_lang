# Small syntax features: safe navigation `&.` on objects, `_` digit separators,
# backtick command substitution, and `defined(name)`. Symbols are covered in
# tests/builtins/symbol_spec.sl, `&.` on nil locals in nullish_coalescing_spec.sl.

recorded = []

def record(value)
  recorded.push(value)
  value
end

class Address
  city: String

  new(city)
    @city = city
  end
end

class Member
  name: String
  address: Address

  new(name, address)
    @name = name
    @address = address
  end

  def greet(greeting)
    "#{greeting}, #{@name}"
  end

  def shout
    @name.upcase
  end
end

paris_member = nil
homeless_member = nil
missing_member = nil

describe("safe navigation &.") do
  before_each() do
    recorded.clear
    paris_member = new Member("ada", new Address("Paris"))
    homeless_member = new Member("bob", nil)
    missing_member = nil
  end

  context("on a non-nil receiver") do
    test("reads a field") do
      assert_eq(paris_member&.name, "ada")
    end

    test("calls a method with arguments") do
      assert_eq(paris_member&.greet("hi"), "hi, ada")
    end

    test("calls a zero-argument method") do
      assert_eq(paris_member&.shout, "ADA")
    end

    test("chains through nested objects") do
      assert_eq(paris_member&.address&.city, "Paris")
    end

    test("evaluates the arguments") do
      paris_member&.greet(record("hey"))
      assert_eq(recorded, ["hey"])
    end
  end

  context("on nil") do
    test("a field read is nil") do
      assert_null(missing_member&.name)
    end

    test("a method call is nil") do
      assert_null(missing_member&.greet("hi"))
      assert_null(missing_member&.shout)
    end

    test("the arguments are not evaluated") do
      assert_null(missing_member&.greet(record("never")))
      assert_eq(recorded, [])
    end

    test("a chain stops at the first nil") do
      assert_null(missing_member&.address&.city)
      assert_null(missing_member&.anything&.at&.all)
    end

    test("a nil field in the middle of a chain gives nil") do
      assert_null(homeless_member&.address&.city)
      assert_eq(homeless_member&.name, "bob")
    end

    test("combines with ?? for a default only when the chain is nil") do
      assert_eq(homeless_member&.address&.city ?? "nowhere", "nowhere")
      assert_eq(paris_member&.address&.city ?? "nowhere", "Paris")
    end
  end
end

describe("digit separators (basics in literals_spec.sl)") do
  test("may sit between any two digits, and under a minus sign") do
    assert_eq(1_0_0, 100)
    assert_eq(-1_000, -1000)
  end

  test("leave no trace in the value") do
    assert_eq(1_000.to_s, "1000")
    assert_eq(1_000 + 1, 1001)
  end

  test("underscores are allowed in a float literal") do
    assert_eq(1_000.25, 1000.25)
    assert_eq(3.141_592, 3.141592)
    assert_eq(type(1_000.5), "float")
  end
end

describe("backtick command substitution") do
  test("captures stdout, stderr and the exit code") do
    result = `echo hi`
    assert_eq(result.stdout, "hi\n")
    assert_eq(result.stderr, "")
    assert_eq(result.exit_code, 0)
  end

  test("runs through a shell, so pipes work") do
    assert_eq(`printf 'a\nb\n' | wc -l`.stdout.trim, "2")
  end

  test("reports stderr and a non-zero exit code without raising") do
    result = `echo oops 1>&2; exit 4`
    assert_eq(result.stdout, "")
    assert_eq(result.stderr, "oops\n")
    assert_eq(result.exit_code, 4)
  end
end

describe("defined(name)") do
  test("is true for a variable of the test") do
    local_count = 3
    assert(defined("local_count"))
    assert_eq(local_count, 3)
  end

  test("is true for a top-level variable, function and class") do
    assert(defined("recorded"))
    assert(defined("record"))
    assert(defined("Member"))
  end

  test("is true for a builtin function") do
    assert(defined("print"))
  end

  test("is true for a function parameter") do
    checker = fn(value) { defined("value") }
    assert(checker(1))
  end

  test("is false for an unknown name and for the empty string") do
    assert_not(defined("no_such_name_anywhere"))
    assert_not(defined(""))
  end
end
