class Gateway
  static def charge(amount)
    "real #{amount}"
  end

  def ping
    "real ping"
  end
end

class Checkout
  def pay(amount)
    Gateway.charge(amount)
  end
end

describe("stubbing real classes") do
  test("static stub via lambda, recorded") do
    mock = Mock.stub_class(Gateway, "charge", fn(amount) { "stubbed #{amount}" })
    assert_eq(new Checkout().pay(5), "stubbed 5")
    mock.assert_received("charge", [5])
  end

  test("stub is gone in the next test") do
    assert_eq(Gateway.charge(1), "real 1")
  end

  test("instance stub with a constant") do
    Mock.stub_instance(Gateway, "ping", "pong")
    assert_eq(new Gateway().ping(), "pong")
  end

  test("instance stub is gone too") do
    assert_eq(new Gateway().ping(), "real ping")
  end
end

class StubbedPost < Model
end

describe("stubbing model classes") do
  test("static model finders and instance save") do
    Mock.stub_class(StubbedPost, "find", fn(id) { {
      "id": id,
      "title": "fake"
    } })
    Mock.stub_class(StubbedPost, "all", [1, 2])
    assert_eq(StubbedPost.find(9)["title"], "fake")
    assert_eq(StubbedPost.all().length, 2)
  end
end

class Ledger
  static def record(entry)
    "recorded #{entry}"
  end

  def total
    42
  end
end

describe("allow chains and spies") do
  test("and_return replaces a class method") do
    mock = Mock.allow(Ledger).to_receive("record").and_return("stubbed")
    assert_eq(Ledger.record("a"), "stubbed")
    mock.assert_received("record", ["a"])
  end

  test("and_call runs a lambda with the arguments") do
    Mock.allow(Ledger).to_receive("record").and_call(fn(entry) { "lambda #{entry}" })
    assert_eq(Ledger.record("b"), "lambda b")
  end

  test("and_call_original records and still runs the real method") do
    spy = Mock.allow(Ledger).to_receive("record").and_call_original
    assert_eq(Ledger.record("c"), "recorded c")
    spy.assert_received("record", ["c"])
    assert_eq(spy.call_count("record"), 1)
  end

  test("allow_any_instance with and_call_original on an instance method") do
    spy = Mock.allow_any_instance(Ledger).to_receive("total").and_call_original
    assert_eq(new Ledger().total(), 42)
    assert(spy.received?("total"))
  end

  test("allow_any_instance with and_return") do
    Mock.allow_any_instance(Ledger).to_receive("total").and_return(7)
    assert_eq(new Ledger().total(), 7)
  end

  test("all of it is undone after the test") do
    assert_eq(Ledger.record("d"), "recorded d")
    assert_eq(new Ledger().total(), 42)
  end
end

class Tariff
  static def rate
    "real rate"
  end
end

describe("a stub from before_all") do
  before_all() do
    Mock.stub_class(Tariff, "rate", "suite rate")
  end

  test("reaches the first test") do
    assert_eq(Tariff.rate(), "suite rate")
  end

  test("still reaches the second") do
    assert_eq(Tariff.rate(), "suite rate")
  end

  test("survives a test that stubs something else") do
    Mock.stub_class(Ledger, "record", "per test")
    assert_eq(Ledger.record("x"), "per test")
    assert_eq(Tariff.rate(), "suite rate")
  end

  test("while the per-test stub still ends with its test") do
    assert_eq(Ledger.record("x"), "recorded x")
  end
end

describe("after a suite with a before_all stub") do
  test("the stub is gone") do
    assert_eq(Tariff.rate(), "real rate")
  end
end

class CallbackInvoice < Model
  before_save("veto")

  def veto
    false
  end
end

describe("stubbing a model method that has callbacks") do
  test("a stubbed save answers from the stub, not the callbacks") do
    mock = Mock.allow_any_instance(CallbackInvoice).to_receive("save").and_return("stubbed")
    invoice = new CallbackInvoice()
    assert_eq(invoice.save(), "stubbed")
    mock.assert_received("save")
  end

  test("a spied save still runs the callbacks, and is recorded") do
    spy = Mock.allow_any_instance(CallbackInvoice).to_receive("save").and_call_original
    invoice = new CallbackInvoice()
    # The before_save veto answers false without reaching the database.
    assert_eq(invoice.save(), false)
    spy.assert_received("save", [])
    assert_eq(spy.call_count("save"), 1)
  end
end
