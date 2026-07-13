# Named-argument calls (parenthesized form): reordering, mixing with
# positional, and selecting which default to override.
#
# The bytecode VM does not compile named-argument calls — its calling
# convention can't reorder arguments by parameter name at runtime — so the
# production request path falls back to the tree-walking interpreter for them
# (see src/vm/compiler_exprs.rs `named_args_compile_tests`). This spec pins the
# interpreter behavior that fallback relies on: the observable result must be
# correct regardless of engine.

def add(a, b) { return a + b }
def greet(name = "World", punct = "!") { return "Hi " + name + punct }

describe("Named arguments (paren form)", fn() {
    test("named args can be given in any order", fn() {
        assert_eq(add(b: 2, a: 1), 3)
        assert_eq(add(a: 10, b: 20), 30)
    })

    test("named args mix with leading positional args", fn() {
        assert_eq(greet("Bob", punct: "?"), "Hi Bob?")
    })

    test("named args select which default to override", fn() {
        assert_eq(greet(punct: "?"), "Hi World?")
        assert_eq(greet(name: "Ann"), "Hi Ann!")
    })

    test("all-positional calls still work", fn() {
        assert_eq(add(4, 5), 9)
        assert_eq(greet("X", "!"), "Hi X!")
    })
})
