# Named-argument calls (parenthesized form): reordering, mixing with
# positional, and selecting which default to override.
#
# The bytecode VM does not compile named-argument calls — its calling
# convention can't reorder arguments by parameter name at runtime — so the
# production request path falls back to the tree-walking interpreter for them
# (see src/vm/compiler_exprs.rs `named_args_compile_tests`). This spec pins the
# interpreter behavior that fallback relies on: the observable result must be
# correct regardless of engine.

def add(a, b)
  a + b
end

def greet(name = "World", punct = "!")
  "Hi " + name + punct
end

class Box
  def resize(width, height = 1)
    "#{width}x#{height}"
  end
end

describe("Named arguments (paren form)") do
  test("named args can be given in any order") do
    assert_eq(add(b: 2, a: 1), 3)
    assert_eq(add(a: 10, b: 20), 30)
    assert_eq(greet(punct: "?", name: "Zoe"), "Hi Zoe?")
  end

  test("named args mix with leading positional args") do
    assert_eq(greet("Bob", punct: "?"), "Hi Bob?")
  end

  test("named args select which default to override") do
    assert_eq(greet(punct: "?"), "Hi World?")
    assert_eq(greet(name: "Ann"), "Hi Ann!")
  end

  test("all-positional calls still work") do
    assert_eq(add(4, 5), 9)
    assert_eq(greet("X", "!"), "Hi X!")
  end

  test("named args reach a method and fill its defaults") do
    box = new Box()
    assert_eq(box.resize(height: 3, width: 2), "2x3")
    assert_eq(box.resize(width: 4), "4x1")
  end

  test("a missing required argument raises") do
    assert_raises("Wrong number of arguments") do
      add(a: 1)
    end
  end

  test("an unknown argument name raises") do
    assert_raises() do
      add(a: 1, c: 2)
    end
  end
end
