# match: literal, wildcard, binding, typed, array, hash and nested patterns,
# guards, and what happens when no arm matches. Enum-variant patterns are
# covered in tests/builtins/enum_spec.sl.
# Do not run `soli fmt` here: it rewrites the type-first pattern `Int: n` into
# `n: Int`, which does not parse, and `nil` patterns into `null`.

class MatchCat
  name: String

  new(name: String)
    @name = name
  end
end

class MatchDog
end

def number_word(x)
  match x {
    0 => "zero",
    1 => "one",
    _ => "other",
  }
end

def literal_kind(value)
  match value {
    "a" => "string a",
    1.5 => "float",
    true => "true",
    false => "false",
    nil => "nil",
    0 => "zero",
    _ => "other"
  }
end

# The linter's undefined-local rule does not see most names a pattern binds
# (typed, array, hash, and a bare name after a guarded arm); every flagged read
# below is of a pattern binding.
# soli-lint-disable smell/undefined-local
def classify(x)
  match x {
    n if n < 0 => "negative",
    n if n == 0 => "zero",
    n if n > 0 => "positive",
  }
end

def fizzbuzz(number)
  match number {
    n if n % 15 == 0 => "fizzbuzz",
    n if n % 5 == 0 => "buzz",
    n if n % 3 == 0 => "fizz",
    n => str(n),
  }
end

def primitive_kind(value)
  match value {
    Int: n if n > 10 => "big int #{n}",
    Int: n => "int #{n}",
    String: text => "string #{text}",
    Float: number => "float #{number}",
    Bool: flag => "bool #{flag}",
    _ => "other"
  }
end

def class_kind(value)
  match value {
    cat: MatchCat => "cat",
    dog: MatchDog => "dog",
    nil => "nil",
    _ => "other"
  }
end

def builtin_kind(value)
  match value {
    list: Array => "array",
    map: Hash => "hash",
    _ => "other"
  }
end

def shape(list)
  match list {
    [] => "empty",
    [x] => "single: #{x}",
    [x, y] => "pair: #{x}, #{y}",
    _ => "many",
  }
end

def head_and_tail(list)
  match list {
    [] => "empty",
    [head, ...tail] => [head, tail],
  }
end

def first_two(list)
  match list {
    [first, second, ...rest] => [first, second, rest],
    _ => "too short"
  }
end

def person_label(person)
  match person {
    {name: name, age: age} => "#{name} is #{age}",
    {name: name} => name,
    _ => "unknown",
  }
end

def split_name(person)
  match person {
    {name: name, ...others} => [name, others],
    _ => "no name"
  }
end

def node_value(node)
  match node {
    {type: "leaf", value: value} => value,
    {type: "pair", left: left, right: right} => node_value(left) + node_value(right),
    _ => 0
  }
end

def author_and_first_tag(post)
  match post {
    {author: {name: author_name}, tags: [first, ..._rest]} => "#{author_name}:#{first}",
    _ => "none"
  }
end

# soli-lint-enable smell/undefined-local

def only_one(value)
  match value {
    1 => "one"
  }
end

describe("Pattern matching") do
  context("literal patterns") do
    test("match equal integers, with _ as the fallback") do
      assert_eq(number_word(0), "zero")
      assert_eq(number_word(1), "one")
      assert_eq(number_word(99), "other")
    end

    test("match strings, floats, booleans and nil by value") do
      assert_eq(literal_kind("a"), "string a")
      assert_eq(literal_kind(1.5), "float")
      assert_eq(literal_kind(true), "true")
      assert_eq(literal_kind(false), "false")
      assert_eq(literal_kind(nil), "nil")
      assert_eq(literal_kind("b"), "other")
    end

    test("an Int pattern does not match other Ints") do
      assert_eq(literal_kind(0), "zero")
      assert_eq(literal_kind(1), "other")
    end

    test("compare numbers by value across Int and Float, as == does") do
      # `0 == 0.0` is true on both engines and the VM matches; the tree engine
      # returns "other".
      pending("bug: on the tree engine an Int literal pattern misses an equal Float")
      assert_eq(literal_kind(0.0), "zero")
    end

    test("false and nil are distinct patterns") do
      assert_eq(literal_kind(false), "false")
      assert_eq(literal_kind(nil), "nil")
    end
  end

  context("binding patterns") do
    test("a bare name matches anything and binds it") do
      result = match 42 {
        0 => "zero",
        n => "number: #{n}",
      }
      assert_eq(result, "number: 42")
    end

    test("the first matching arm wins") do
      result = match 5 {
        n => "first #{n}",
        5 => "second",
      }
      assert_eq(result, "first 5")
    end
  end

  context("guards") do
    test("select an arm by condition") do
      assert_eq(classify(-5), "negative")
      assert_eq(classify(0), "zero")
      assert_eq(classify(5), "positive")
    end

    test("a failing guard falls through to the next arm") do
      assert_eq(fizzbuzz(15), "fizzbuzz")
      assert_eq(fizzbuzz(5), "buzz")
      assert_eq(fizzbuzz(3), "fizz")
      assert_eq(fizzbuzz(7), "7")
    end

    test("can read variables from the enclosing scope") do
      limit = 10
      result = match 12 {
        n if n > limit => "over",
        _ => "under"
      }
      assert_eq(result, "over")
    end
  end

  context("typed patterns") do
    test("a primitive type before the name tests it and binds the value") do
      assert_eq(primitive_kind(5), "int 5")
      assert_eq(primitive_kind("x"), "string x")
      assert_eq(primitive_kind(1.5), "float 1.5")
      assert_eq(primitive_kind(false), "bool false")
      assert_eq(primitive_kind(nil), "other")
    end

    test("a primitive typed pattern takes a guard") do
      assert_eq(primitive_kind(50), "big int 50")
    end

    test("a class after the name tests the value's class") do
      assert_eq(class_kind(new MatchCat("Tom")), "cat")
      assert_eq(class_kind(new MatchDog()), "dog")
      assert_eq(class_kind(nil), "nil")
      assert_eq(class_kind(1), "other")
    end

    test("Array or Hash after the name tests for the built-in collections") do
      # The VM matches both; the tree engine falls through to `_`.
      pending("bug: on the tree engine `x: Array` / `x: Hash` never match")
      assert_eq(builtin_kind([1]), "array")
      assert_eq(builtin_kind({"a": 1}), "hash")
      assert_eq(builtin_kind(1), "other")
    end

    test("a class after the name binds the name, as soli-language.md shows") do
      # Docs (Type-Based Matching) write `s: String => str(len(s))`; `String`
      # there does not even parse (a type keyword), and for a class the name
      # is parsed and discarded: `cat: MatchCat => cat.name` raises
      # "Undefined variable 'cat'".
      pending("bug: a `name: Type` pattern binds nothing, though the docs use the name")
      result = match new MatchCat("Tom") {
        cat: MatchCat => cat.name,
        _ => "other"
      }
      assert_eq(result, "Tom")
    end
  end

  context("array patterns") do
    test("match by length") do
      assert_eq(shape([]), "empty")
      assert_eq(shape([1]), "single: 1")
      assert_eq(shape([1, 2]), "pair: 1, 2")
      assert_eq(shape([1, 2, 3]), "many")
    end

    test("do not match a value that is not an array") do
      assert_eq(shape("ab"), "many")
      assert_eq(shape({"a": 1}), "many")
    end

    test("...rest collects the remaining elements") do
      assert_eq(head_and_tail([1, 2, 3]), [1, [2, 3]])
    end

    test("...rest is empty when nothing remains") do
      assert_eq(head_and_tail([1]), [1, []])
      assert_eq(first_two([1, 2]), [1, 2, []])
    end

    test("elements before ...rest are still required") do
      assert_eq(first_two([1, 2, 3, 4]), [1, 2, [3, 4]])
      assert_eq(first_two([1]), "too short")
    end
  end

  context("hash patterns") do
    test("bind fields by bare key name") do
      assert_eq(person_label({"name": "Alice", "age": 30}), "Alice is 30")
    end

    test("a missing key fails the arm, extra keys are allowed") do
      assert_eq(person_label({"name": "Bob"}), "Bob")
      assert_eq(person_label({"name": "Cy", "age": 4, "city": "Paris"}), "Cy is 4")
      assert_eq(person_label({"foo": "bar"}), "unknown")
    end

    test("a key holding nil is present") do
      result = match {"a": nil} {
        {a: value} => "has a: #{value.nil?}",
        _ => "no a"
      }
      assert_eq(result, "has a: true")
    end

    test("{} matches any hash but nothing else") do
      result = match {"a": 1} {
        {} => "hash",
        _ => "not"
      }
      assert_eq(result, "hash")
      not_hash = match 5 {
        {} => "hash",
        _ => "not"
      }
      assert_eq(not_hash, "not")
    end

    test("...rest collects the remaining keys") do
      assert_eq(split_name({"name": "Al", "age": 3, "city": "P"}), ["Al", {"age": 3, "city": "P"}])
      assert_eq(split_name({"name": "Al"}), ["Al", {}])
      assert_eq(split_name({"age": 1}), "no name")
    end

    test("a field pattern can be a literal") do
      leaf = {"type": "leaf", "value": 4}
      tree = {"type": "pair", "left": leaf, "right": {"type": "leaf", "value": 6}}
      assert_eq(node_value(leaf), 4)
      assert_eq(node_value(tree), 10)
      assert_eq(node_value({"type": "other"}), 0)
    end
  end

  context("nested patterns") do
    test("array patterns nest") do
      result = match [1, [2, 3]] {
        [first, [second, third]] => first + second + third,
        _ => 0
      }
      assert_eq(result, 6)
    end

    test("hash and array patterns nest in each other") do
      post = {"author": {"name": "Al", "id": 1}, "tags": ["a", "b"]}
      assert_eq(author_and_first_tag(post), "Al:a")
    end

    test("a nested part that fails fails the whole arm") do
      assert_eq(author_and_first_tag({"author": {"name": "Al"}, "tags": []}), "none")
      assert_eq(author_and_first_tag({"author": "Al", "tags": ["a"]}), "none")
    end
  end

  context("as an expression") do
    test("the matched arm's value is the value of match") do
      doubled = match 5 { x => x * 2 }
      assert_eq(doubled, 10)
    end

    test("no matching arm raises") do
      assert_eq(only_one(1), "one")
      assert_raises("no pattern matched the value") do
        only_one(2)
      end
    end
  end
end
