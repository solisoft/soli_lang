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

  def via_this
    this._scale(4)
  end

  def via_block
    [1, 2].map do |n|
      _scale(n)
    end
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

  public

  def reopened
    "public again"
  end

  private def _one_off
    "one off"
  end

  def after_one_off
    "still public"
  end
end

class SubLedger < Ledger
  def tenfold
    _scale(10)
  end
end

describe("private methods") do
  test("a bare name calls the method on self") do
    ledger = new Ledger(5)
    assert_eq(ledger.doubled, 10)
    assert_eq(ledger.label, "ledger 5")
  end

  test("@name and this.name call it on self too") do
    ledger = new Ledger(5)
    assert_eq(ledger.via_at, 15)
    assert_eq(ledger.via_this, 20)
  end

  test("a block inside a method still calls it on self") do
    assert_eq(new Ledger(5).via_block, [5, 10])
  end

  test("a call from outside the class is refused") do
    assert_raises("private method '_scale' called for an instance of Ledger") do
      new Ledger(5)._scale(2)
    end
  end

  test("another instance of the same class is refused too") do
    assert_raises("private method '_scale' called for an instance of Ledger") do
      new Ledger(1).peek(new Ledger(2))
    end
  end

  test("a private section covers every method after it") do
    assert_raises("private method '_describe'") do
      new Ledger(1)._describe
    end
  end

  test("a public section ends it") do
    assert_eq(new Ledger(1).reopened, "public again")
  end

  test("a private modifier on one def covers that def only") do
    assert_raises("private method '_one_off'") do
      new Ledger(1)._one_off
    end
    assert_eq(new Ledger(1).after_one_off, "still public")
  end

  test("a subclass calls an inherited private method on self") do
    assert_eq(new SubLedger(2).tenfold, 20)
  end

  test("a subclass instance still refuses it from outside") do
    assert_raises("private method '_scale' called for an instance of SubLedger") do
      new SubLedger(2)._scale(1)
    end
  end

  test("send bypasses the check, as in Ruby") do
    assert_eq(new Ledger(2).send("_scale", 2), 4)
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
  test("reachable from the class on another instance") do
    assert(new Wallet(10).richer_than?(new Wallet(5)))
    assert_not(new Wallet(1).richer_than?(new Wallet(5)))
  end

  test("reachable from a subclass on a parent instance") do
    assert_eq(new Purse(1).compare(new Wallet(7)), 7)
  end

  test("refused from outside") do
    assert_raises("protected method 'amount' called for an instance of Wallet") do
      new Wallet(3).amount
    end
  end
end
