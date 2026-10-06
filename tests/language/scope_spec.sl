# Scope: what a nested block, a branch, a loop, a try, a function and a lambda
# can see and change of the bindings around them. The spec runner uses the
# tree-walking engine, which scopes strictly by block; the VM lets a name first
# assigned in an `if`/`while` body be read after it (see www/docs/soli-language.md).

def reads_callers_local
  # soli-lint-disable-next-line smell/undefined-local
  callers_local
end

def assigns_its_own(value)
  shared_name = value
  shared_name
end

def make_counter
  count = 0
  return fn() {
    count = count + 1
    count
  }
end

describe("Scope") do
  context("nested blocks") do
    test("a let inside shadows the outer variable only inside") do
      value = 1
      inner_seen = 0
      {
        let value = 2
        inner_seen = value
      }
      assert_eq(inner_seen, 2)
      assert_eq(value, 1)
    end

    test("a bare assignment inside updates the outer variable") do
      value = 1
      {
        value = 2
      }
      assert_eq(value, 2)
    end

    test("an outer variable is readable inside") do
      outer = 10
      result = 0
      {
        result = outer + 5
      }
      assert_eq(result, 15)
    end

    test("every enclosing level is visible from the innermost") do
      seen = []
      level_one = 1
      {
        level_two = 2
        {
          level_three = 3
          seen = [level_one, level_two, level_three]
        }
        seen.push(level_two)
      }
      assert_eq(seen, [1, 2, 3, 2])
    end

    test("a name first assigned inside is gone after the block") do
      {
        block_local = 1
      }
      # soli-lint-disable-next-line smell/undefined-local
      assert_raises("Undefined variable 'block_local'") do
        block_local
      end
    end
  end

  context("branches") do
    test("an if body updates an outer variable") do
      result = ""
      if true
        result = "inside if"
      end
      assert_eq(result, "inside if")
    end

    test("a let in an if body shadows only inside it") do
      message = "outer"
      if true
        let message = "inner"
        assert_eq(message, "inner")
      end
      assert_eq(message, "outer")
    end

    test("a name first assigned in an if body is gone after it") do
      if true
        branch_local = 1
      end
      # soli-lint-disable-next-line smell/undefined-local
      assert_raises("Undefined variable 'branch_local'") do
        branch_local
      end
    end
  end

  context("loops") do
    test("a for body updates an outer accumulator") do
      sum = 0
      for i in [1, 2, 3]
        sum = sum + i
      end
      assert_eq(sum, 6)
    end

    test("the for loop variable is gone after the loop") do
      seen = []
      for item in [1, 2]
        seen.push(item)
      end
      assert_eq(seen, [1, 2])
      # soli-lint-disable-next-line smell/undefined-local
      assert_raises("Undefined variable 'item'") do
        item
      end
    end

    test("a while body updates the outer counter and sees each fresh local") do
      count = 0
      seen = []
      while count < 3
        current = count
        seen.push(current)
        count = count + 1
      end
      assert_eq(count, 3)
      assert_eq(seen, [0, 1, 2])
    end
  end

  context("try and catch") do
    test("an assignment in the try body reaches the outer variable") do
      status = "before"
      try
        status = "in try"
      catch error
        status = "in catch"
      end
      assert_eq(status, "in try")
    end

    test("the catch variable is gone after the catch") do
      message = ""
      try
        throw "boom"
      catch failure
        message = "#{failure}"
      end
      assert_contains(message, "boom")
      # soli-lint-disable-next-line smell/undefined-local
      assert_raises("Undefined variable 'failure'") do
        failure
      end
    end
  end

  context("functions") do
    test("a function cannot see the caller's locals") do
      callers_local = 1
      assert_eq(callers_local, 1)
      assert_raises("Undefined variable 'callers_local'") do
        reads_callers_local()
      end
    end

    test("a function's assignment does not reach the caller's variable of the same name") do
      shared_name = 100
      assert_eq(assigns_its_own(50), 50)
      assert_eq(shared_name, 100)
    end
  end

  context("lambdas") do
    test("capture the enclosing variables") do
      outer = 42
      get_outer = fn() { outer }
      assert_eq(get_outer(), 42)
    end

    test("see a captured variable's later value") do
      outer = 1
      get_outer = fn() { outer }
      outer = 2
      assert_eq(get_outer(), 2)
    end

    test("a bare assignment inside updates the captured variable") do
      total = 0
      add = fn(amount) { total = total + amount }
      add(3)
      add(4)
      assert_eq(total, 7)
    end

    test("a let inside shadows the captured variable") do
      value = 1
      shadow = fn() {
        let value = 77
        value
      }
      assert_eq(shadow(), 77)
      assert_eq(value, 1)
    end

    test("each closure from a factory keeps its own state") do
      first = make_counter()
      second = make_counter()
      first()
      first()
      assert_eq(first(), 3)
      assert_eq(second(), 1)
    end
  end
end
