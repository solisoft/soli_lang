# Variables: bare assignment, `let` with a type annotation, reassignment, and
# which assignments reach an enclosing binding. Scoping rules proper live in
# scope_spec.sl.

describe("Variables") do
  context("declaring") do
    test("a bare assignment creates the binding") do
      count = 10
      assert_eq(count, 10)
    end

    test("let declares a variable too") do
      let count = 10
      assert_eq(count, 10)
    end

    test("let with a type annotation keeps the value and its type") do
      let answer: Int = 42
      assert_eq(answer, 42)
      assert_eq(type(answer), "int")
    end

    test("several variables in a row are independent bindings") do
      first = 1
      second = 2
      third = 3
      assert_eq([first, second, third], [1, 2, 3])
      assert_eq(first + second + third, 6)
    end

    test("an assignment is an expression, so assignments chain") do
      left = right = 3
      assert_eq(left, 3)
      assert_eq(right, 3)
    end
  end

  context("reassigning") do
    test("a variable can be reassigned") do
      value = 1
      value = 2
      assert_eq(value, 2)
    end

    test("a let variable can be reassigned") do
      let value = 1
      value = 2
      assert_eq(value, 2)
    end

    test("reassignment can change the value's type") do
      value = 1
      value = "one"
      assert_eq(value, "one")
      assert_eq(type(value), "string")
    end

    test("a variable can be reassigned from its own value") do
      total = 5
      total = total * 2 + 1
      assert_eq(total, 11)
    end

    test("assigning one variable to another copies the value, not the name") do
      original = 1
      copy = original
      original = 2
      assert_eq(copy, 1)
    end

    test("two names for one array see each other's mutations") do
      original = [1]
      alias_name = original
      alias_name.push(2)
      assert_eq(original, [1, 2])
    end
  end

  context("reading") do
    test("reading a name that was never assigned raises") do
      # soli-lint-disable-next-line smell/undefined-local
      assert_raises("Undefined variable 'never_assigned'") do
        never_assigned
      end
    end

    test("nil is a value: a variable holding nil is defined") do
      empty = nil
      assert_null(empty)
      assert(empty.nil?)
    end
  end
end
