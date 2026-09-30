# Private methods: callable on self only (`@name`, `this.name`, or the bare
# `name`), refused on any other receiver. The same cases run on both engines in
# tests/differential_engines_test.rs.

class Ledger
  total: Int

  new(total: Int)
    @total = total
  end

  def doubled
    _scale(2)
  end

  def label
    _describe
  end

  def via_at
    @_scale(3)
  end

  def via_block
    [1, 2].map { |n| _scale(n) }
  end

  def peek(other)
    other._scale(1)
  end

  private

  def _scale(factor)
    @total * factor
  end

  def _describe
    "ledger #{@total}"
  end
end

describe("private methods") do
  test("a bare name calls the method on self") do
    ledger = new Ledger(5)
    assert_eq(ledger.doubled, 10)
    assert_eq(ledger.label, "ledger 5")
    assert_eq(ledger.via_at, 15)
    assert_eq(ledger.via_block, [5, 10])
  end

  test("a call from outside the class is refused") do
    ledger = new Ledger(5)
    message = ""
    try
      ledger._scale(2)
    catch error
      message = "#{error}"
    end
    assert_contains(message, "private method '_scale' called for an instance of Ledger")
  end

  test("another instance of the same class is refused too") do
    refused = (new Ledger(1).peek(new Ledger(2)) rescue "refused")
    assert_eq(refused, "refused")
  end

  test("a private section covers every method after it") do
    assert_eq((new Ledger(1)._describe rescue "refused"), "refused")
  end
end

class Wallet
  cents: Int

  new(cents: Int)
    @cents = cents
  end

  def richer_than?(other)
    amount > other.amount
  end

  protected

  def amount
    @cents
  end
end

class Purse < Wallet
  def compare(other)
    other.amount
  end
end

describe("protected methods") do
  test("reachable from the class and its subclasses, on any instance") do
    assert(new Wallet(10).richer_than?(new Wallet(5)))
    assert_eq(new Purse(1).compare(new Wallet(7)), 7)
  end

  test("refused from outside") do
    message = ""
    try
      new Wallet(3).amount
    catch error
      message = "#{error}"
    end
    assert_contains(message, "protected method 'amount' called for an instance of Wallet")
  end
end
