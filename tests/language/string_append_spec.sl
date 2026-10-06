describe("in-place string append", fn() {
  test("+= grows a string", fn() {
    let buf = ""
    buf += "ab"
    buf += "c"
    assert_eq(buf, "abc")
  })

  test("assigning a string plus a string grows the same binding", fn() {
    let buf = "a"
    buf = buf + "b"
    buf = buf + "c"
    assert_eq(buf, "abc")
  })

  test("<< appends to a string variable and returns it", fn() {
    let buf = "hi"
    let returned = buf << "!"
    assert_eq(buf, "hi!")
    assert_eq(returned, "hi!")
  })

  test("appending does not change another variable that already aliased it", fn() {
    let original = "ab"
    let alias = original
    original = original + "c"
    assert_eq(original, "abc")
    assert_eq(alias, "ab")
  })

  test("<< still pushes onto an array", fn() {
    let items = [1]
    items << 2
    assert_eq(items, [1, 2])
  })
})

describe("mutating an array while iterating it", fn() {
  test("each stops when the callback shrinks the array", fn() {
    let items = [1, 2, 3]
    items.each(fn(x) { items.pop() })
    assert_eq(items, [1])
  })

  test("map keeps the elements visited before the array shrinks past the index", fn() {
    let items = [1, 2, 3]
    let mapped = items.map(fn(x) {
      items.pop()
      x
    })
    assert_eq(mapped, [1, 2])
    assert_eq(items, [1])
  })

  test("reduce adds only the elements visited", fn() {
    let items = [1, 2, 3]
    let total = items.reduce(fn(acc, x) {
      items.pop()
      acc + x
    }, 0)
    assert_eq(total, 3)
  })
})
