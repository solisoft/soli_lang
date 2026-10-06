# Array.define_method and Hash.define_method add methods to collection values.
# Extensions outlive this file, so every name starts with `spec_`.

describe("Array.define_method") do
  test("adds a method callable on any array") do
    Array.define_method("spec_second", fn() { this[1] })
    assert_eq([10, 20, 30].spec_second, 20)
    assert_raises("Index out of bounds: 1 (length 1)") do
      [10].spec_second
    end
  end

  test("this is the receiver itself, so the method can mutate it") do
    Array.define_method("spec_push_twice", fn(item) {
      this.push(item)
      this.push(item)
    })
    numbers = [1]
    numbers.spec_push_twice(2)
    assert_eq(numbers, [1, 2, 2])
  end

  test("builtin methods still work alongside") do
    Array.define_method("spec_noop", fn() { this })
    numbers = [1, 2, 3]
    assert_eq(numbers.length, 3)
    assert_eq(numbers.map { |x| x * 2 }, [2, 4, 6])
  end
end

describe("Hash.define_method") do
  test("adds a method callable on any hash") do
    Hash.define_method("spec_size_label", fn() { "n=#{this.length}" })
    assert_eq({"a": 1, "b": 2}.spec_size_label, "n=2")
    assert_eq({}.spec_size_label, "n=0")
  end

  # `hash.key` reads a key when no method has that name; a user method wins.
  test("a user method wins over the dot-access key fallback") do
    Hash.define_method("spec_marker", fn() { "method-wins" })
    values = {"spec_marker": "key-value"}
    assert_eq(values.spec_marker, "method-wins")
    assert_eq(values["spec_marker"], "key-value")
  end
end
