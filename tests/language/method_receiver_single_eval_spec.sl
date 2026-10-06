# Method-call receiver is evaluated exactly once.
#
# Regression for the old fast-path design where the hash/string/model call
# interceptors each evaluated the receiver expression and returned "not
# mine" on a type mismatch — so a side-effectful receiver like
# `make().map(f)` ran `make()` twice. The unified dispatcher evaluates the
# receiver once and dispatches on the value.

eval_count = 0

def make_array
  eval_count = eval_count + 1
  [1, 2, 3]
end

def make_hash
  eval_count = eval_count + 1
  {"a": 1, "b": 2}
end

def make_string
  eval_count = eval_count + 1
  "hello"
end

class Greeter
  def greet(name)
    "hi " + name
  end
end

def make_instance
  eval_count = eval_count + 1
  new Greeter()
end

# A plain class whose method is named like a model persist interceptor.
class Doc
  new()
    @saved = false
  end

  def save
    @saved = true
    "saved"
  end
end

def make_doc
  eval_count = eval_count + 1
  new Doc()
end

describe("Method receiver single evaluation") do
  before_each() do
    eval_count = 0
  end

  context("arrays") do
    test("a block method evaluates the receiver once") do
      doubled = make_array().map { |x| x * 2 }
      assert_eq(doubled, [2, 4, 6])
      assert_eq(eval_count, 1)
    end

    test("a pure method evaluates the receiver once") do
      assert_eq(make_array().sum, 6)
      assert_eq(eval_count, 1)
    end

    test("a mutating method evaluates the receiver once") do
      assert_eq(make_array().push(4), [1, 2, 3, 4])
      assert_eq(eval_count, 1)
    end
  end

  context("hashes") do
    test("a method evaluates the receiver once") do
      assert_eq(make_hash().keys, ["a", "b"])
      assert_eq(eval_count, 1)
    end

    test("get with a literal key evaluates the receiver once") do
      assert_eq(make_hash().get("a"), 1)
      assert_eq(eval_count, 1)
    end

    test("delete evaluates the receiver once") do
      # "delete" is also a model-interceptor name; the interceptor must
      # not re-evaluate non-model receivers.
      assert_eq(make_hash().delete("a"), 1)
      assert_eq(eval_count, 1)
    end
  end

  test("a string method evaluates the receiver once") do
    assert_eq(make_string().upcase, "HELLO")
    assert_eq(eval_count, 1)
  end

  context("instances") do
    test("a method evaluates the receiver once") do
      assert_eq(make_instance().greet("bob"), "hi bob")
      assert_eq(eval_count, 1)
    end

    test("a save-named method evaluates the receiver once") do
      # "save" is a model persist-interceptor name; a plain class with a
      # user-defined save must not be evaluated twice (or intercepted).
      assert_eq(make_doc().save, "saved")
      assert_eq(eval_count, 1)
    end
  end

  test("each test starts from a reset counter") do
    assert_eq(eval_count, 0)
  end
end

describe("Comparator side effects") do
  test("a sort comparator may mutate the receiver without panicking") do
    # `sort` runs a user comparator, so it must iterate over a snapshot:
    # a comparator that mutates the receiver used to panic with a
    # RefCell double-borrow because `sort` was missing from the
    # closure-takes-user-code list and ran on a live borrow.
    numbers = [3, 1, 2]
    sorted = numbers.sort { |x, y|
      numbers.push(99)
      x - y
    }
    assert_eq(sorted, [1, 2, 3])
    assert_eq(numbers.take(3), [3, 1, 2])
    assert_gt(numbers.length, 3)
  end
end
