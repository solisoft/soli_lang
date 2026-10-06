# ============================================================================
# Multiline Strings Test Suite
# `"""…"""` is the raw multi-line string. `[[ … ]]` was one too, and is a
# pair of brackets now.
# ============================================================================

describe("Multiline Strings", fn() {
  test("basic multiline string", fn() {
    text = """hello
world"""
    assert_contains(text, "hello")
    assert_contains(text, "world")
  })

  test("multiline string preserves newlines", fn() {
    text = """line1
line2"""
    assert_contains(text, "\n")
  })

  test("multiline string is raw (no escape processing)", fn() {
    text = """hello\nworld"""
    assert_contains(text, "\\n")
  })

  test("multiline string closes mid-expression", fn() {
    # It used to close only at the end of the file, so a call argument
    # swallowed everything after it.
    query = ["""
FOR p IN posts
  RETURN p
""", "second"]
    assert_eq(query.length, 2)
    assert_contains(query[0], "FOR p IN posts")
    assert_eq(query[1], "second")
  })

  test("quotes inside, and quotes before the closing ones", fn() {
    text = """say "hi" and ""twice"""""
    assert_eq(text, "say \"hi\" and \"\"twice\"\"")
  })

  test("empty multiline string", fn() {
    text = """"""
    assert_eq(text, "")
  })

  test("multiline string in hash value", fn() {
    h = {"description": """This is a
multiline description."""}
    assert_contains(h["description"], "This is a")
    assert_contains(h["description"], "multiline description")
  })

  test("double brackets are a nested array", fn() {
    pairs = [["a", 1], ["b", 2]]
    assert_eq(pairs.length, 2)
    assert_eq(pairs[0][0], "a")
    names = [[x for x in ["p", "q"]]]
    assert_eq(names[0], ["p", "q"])
    assert_eq([[]].length, 1)
  })
})
