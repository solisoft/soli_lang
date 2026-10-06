# Regression: `.nil?`, `.class`, `.blank?`, `.present?`, `.inspect` must work on
# Function values. They're advertised as "universal methods on ALL types" and
# several defensive patterns (e.g. view partials doing
# `type(x) != "function" && !x.nil?`) rely on the nil? check not crashing when
# a name accidentally resolves to a function.
#
# Note: functions with zero parameters auto-invoke on bare access, so these
# tests use functions with parameters, whose value survives to member access.

def sum_of(a, b)
  a + b
end

describe("universal methods on functions") do
  context("a lambda") do
    test("responds to .nil? with false") do
      increment = fn(x) { x + 1 }
      assert_eq(increment.nil?, false)
    end

    test("responds to .blank? with false and .present? with true") do
      identity = fn(x) { x }
      assert_eq(identity.blank?, false)
      assert_eq(identity.present?, true)
    end

    test("responds to .class with \"Function\"") do
      identity = fn(x) { x }
      assert_eq(identity.class, "Function")
    end

    test("responds to .inspect with \"<function>\"") do
      identity = |x| { x }
      assert_eq(identity.inspect, "<function>")
    end

    test("is still callable after the universal methods ran") do
      increment = fn(x) { x + 1 }
      assert_eq(increment.nil?, false)
      assert_eq(increment(2), 3)
    end
  end

  context("a named function") do
    test("answers the same universal methods") do
      named = sum_of
      assert_eq(named.nil?, false)
      assert_eq(named.blank?, false)
      assert_eq(named.present?, true)
      assert_eq(named.class, "Function")
      assert_eq(named.inspect, "<function>")
    end
  end

  context("a builtin function") do
    test("answers the same universal methods") do
      builtin = len
      assert_eq(builtin.nil?, false)
      assert_eq(builtin.class, "Function")
      assert_eq(builtin.inspect, "<function>")
    end
  end
end
