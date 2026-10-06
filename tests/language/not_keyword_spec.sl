# Negation with `!` and the `not` keyword, and method names ending in `!`.

class BangHelper
  def save!
    "saved"
  end

  def delete!
    "deleted"
  end

  def valid?
    true
  end
end

def insert!
  "inserted"
end

def delete_record!(id)
  "deleted " + id
end

def validate!
  false
end

def step_one!
  1
end

def step_two!
  2
end

def three_items
  [1, 2, 3]
end

describe("Negation") do
  context("with !") do
    test("flips booleans") do
      assert_eq(!true, false)
      assert_eq(!false, true)
    end

    test("nil negates to true") do
      assert_eq(!nil, true)
    end

    test("flips a variable") do
      flag = true
      assert_eq(!flag, false)
      other = false
      assert_eq(!other, true)
    end

    test("flips a parenthesized comparison") do
      assert_eq(!(1 == 2), true)
      assert_eq(!(1 != 1), true)
      assert_eq(!(5 > 3), false)
    end

    test("double negation gives the truthiness as a Bool") do
      assert_eq(!!true, true)
      assert_eq(!!false, false)
      assert_eq(!!"text", true)
      assert_eq(!!nil, false)
    end

    test("flips a parenthesized logical expression") do
      assert_eq(!(true && false), true)
      assert_eq(!(false || false), true)
      assert_eq(!(true && true), false)
    end

    test("applies to a call result") do
      assert_eq(!three_items().empty?, true)
      assert_eq(!!three_items().empty?, false)
    end
  end

  context("with not") do
    test("flips booleans and nil like !") do
      assert_eq(not true, false)
      assert_eq(not false, true)
      assert_eq(not nil, true)
    end

    test("gives the same result as ! for every value") do
      values = [true, false, nil, 0, 1, "", "a", [], [1], {}, 0.0]
      values.each do |value|
        assert_eq(not value, !value)
      end
    end

    test("not not gives the truthiness as a Bool") do
      assert_eq(not not 0, false)
      assert_eq(not not "a", true)
    end
  end

  context("precedence") do
    test("! binds tighter than ==") do
      # (!1) == 2 is false; !(1 == 2) would be true
      assert_eq(!1 == 2, false)
    end

    test("not binds as tightly as !, unlike Ruby's low-precedence not") do
      # (not 1) == 2 is false; not (1 == 2) would be true
      assert_eq(not 1 == 2, false)
      # (not true) && false is false; not (true && false) would be true
      assert_eq(not true && false, false)
      assert_eq(!true && false, false)
    end

    test("so a negated comparison needs parentheses") do
      x = 5
      assert_raises("Cannot compare bool and int") do
        !x > 3
      end
      assert_raises("Cannot compare bool and int") do
        not x > 3
      end
      assert_eq(not (x > 3), false)
    end
  end

  context("falsy values") do
    test("empty string") do
      assert_eq(!"", true)
      assert_eq(!!"", false)
    end

    test("zero") do
      assert_eq(!0, true)
      assert_eq(!!0, false)
    end

    test("0.0 is truthy, unlike 0") do
      assert_eq(!0.0, false)
    end

    test("empty array") do
      assert_eq(![], true)
      assert_eq(!![], false)
    end

    test("empty hash") do
      assert_eq(!{}, true)
      assert_eq(!!{}, false)
    end
  end
end

describe("Bang suffix on method names") do
  test("a top-level function name can end with !") do
    assert_eq(insert!(), "inserted")
  end

  test("with parameters") do
    assert_eq(delete_record!("123"), "deleted 123")
  end

  test("on instance methods, called without parentheses") do
    helper = new BangHelper()
    assert_eq(helper.save!, "saved")
    assert_eq(helper.delete!, "deleted")
  end

  test("the suffix is part of the name: a ! in front still negates") do
    assert_eq(!insert!(), false)
    assert_eq(!new BangHelper().valid?, false)
  end

  test("a bang method can return false") do
    assert_eq(validate!(), false)
  end

  test("results of bang functions combine like any value") do
    assert_eq(step_one!() + step_two!(), 3)
  end

  test("a name followed by != without spaces compares") do
    # `count!=1` lexes as an assignment to a new name `count!`; Ruby reads it
    # as `count != 1`.
    pending("bug: `x!=1` assigns to `x!` instead of comparing")
    count = 2
    assert_eq(count!=1, true)
  end
end
