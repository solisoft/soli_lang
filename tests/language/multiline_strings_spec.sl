# Triple-quoted strings: `"""…"""` is the raw multi-line string — no escape
# processing, no interpolation, no dedent. `[[ … ]]` was one too, and is a
# pair of brackets now.

describe("Multiline strings") do
  test("spans lines, keeping each newline") do
    text = """hello
world"""
    assert_eq(text, "hello\nworld")
    assert_eq(text.split("\n"), ["hello", "world"])
  end

  test("keeps the newline after the opening quotes and the indentation") do
    text = """
  indented
"""
    assert_eq(text, "\n  indented\n")
  end

  test("is raw: escapes stay as backslash sequences") do
    text = """hello\nworld\t\\"""
    assert_eq(text, "hello\\nworld\\t\\\\")
    assert_eq(text.length, 16)
  end

  test("is raw: an interpolation stays literal text") do
    name = "n"
    assert_eq("""#{name}""", "#" + "{name}")
  end

  test("closes mid-expression") do
    # It used to close only at the end of the file, so a call argument
    # swallowed everything after it.
    query = ["""
FOR p IN posts
  RETURN p
""", "second"]
    assert_eq(query, ["\nFOR p IN posts\n  RETURN p\n", "second"])
  end

  test("allows quotes inside, and quotes before the closing ones") do
    text = """say "hi" and ""twice"""""
    assert_eq(text, "say \"hi\" and \"\"twice\"\"")
  end

  test("can be empty") do
    assert_eq("""""", "")
  end

  test("works as a hash value") do
    record = {"description": """This is a
multiline description."""}
    assert_eq(record["description"], "This is a\nmultiline description.")
  end

  test("double brackets are a nested array") do
    pairs = [["a", 1], ["b", 2]]
    assert_eq(pairs.length, 2)
    assert_eq(pairs[0][0], "a")
    names = [[x for x in ["p", "q"]]]
    assert_eq(names, [["p", "q"]])
    assert_eq([[]], [[]])
    assert_eq([[]].length, 1)
  end
end
