# Padding a string to a width: lpad / rjust on the left, rpad / ljust on the
# right, center on both sides. A string already that wide is left alone.

describe("lpad and rjust") do
  test("pad on the left with spaces by default") do
    assert_eq("hello".lpad(10), "     hello")
    assert_eq("hi".rjust(5), "   hi")
  end

  test("pad with a given character") do
    assert_eq("hi".lpad(5, "*"), "***hi")
    assert_eq("hi".rjust(5, "0"), "000hi")
  end

  test("leave a string at or past the width alone") do
    assert_eq("hello".lpad(5), "hello")
    assert_eq("hello".lpad(3), "hello")
    assert_eq("hello".lpad(3, "*"), "hello")
  end

  test("pad the empty string") do
    assert_eq("".lpad(5), "     ")
  end
end

describe("rpad and ljust") do
  test("pad on the right with spaces by default") do
    assert_eq("hello".rpad(10), "hello     ")
    assert_eq("hi".ljust(5), "hi   ")
  end

  test("pad with a given character") do
    assert_eq("hi".rpad(5, "*"), "hi***")
    assert_eq("hi".ljust(5, "."), "hi...")
  end

  test("leave a string at or past the width alone") do
    assert_eq("hello".rpad(5), "hello")
    assert_eq("hello".rpad(3), "hello")
  end

  test("pad the empty string") do
    assert_eq("".rpad(5), "     ")
  end
end

describe("center") do
  test("pads both sides") do
    assert_eq("hi".center(6), "  hi  ")
    assert_eq("hi".center(6, "-"), "--hi--")
  end

  test("puts the odd column on the right") do
    assert_eq("hi".center(5), " hi  ")
  end

  test("leaves a wider string alone") do
    assert_eq("hello".center(3), "hello")
  end
end

describe("padding non-ASCII text") do
  test("counts characters, not bytes") do
    pending("bug: padding measures bytes: \"é\".ljust(3, \"*\") is \"é*\", \"héllo\".center(9, \"*\") \"*héllo**\"")
    assert_eq("é".ljust(3, "*"), "é**")
    assert_eq("é".rjust(3, "*"), "**é")
    assert_eq("héllo".center(9, "*"), "**héllo**")
  end
end
