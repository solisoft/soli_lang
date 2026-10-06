# Test doubles: `new Mock(...)` for a collaborator passed in, Mock.stub_* and
# Mock.allow chains for a real class, and the mock HTTP server for a service
# called over the network. A double's methods are called with parentheses:
# a bare `repo.count` yields the bound method, not its result.

port = mock_http_server_start()
base = "http://127.0.0.1:#{port}"

describe("Mock doubles") do
  test("answer stubbed values and lambdas, and record every call") do
    repo = new Mock(
      "repo",
      {"count": 3, "find": fn(id) { {"id": id} }}
    )
    assert_eq(repo.count(), 3)
    assert_eq(repo.find(7), {"id": 7})
    repo.assert_received("find", [7])
    repo.assert_not_received("delete")
    assert_eq(repo.call_count("count"), 1)
    called = repo.calls().map do |call|
      call["name"]
    end
    assert_eq(called, ["count", "find"])
    find_args = repo.calls_to("find").map do |call|
      call["args"]
    end
    assert_eq(find_args, [[7]])
    assert(repo.received?("find"))
    assert_not(repo.received?("delete"))
  end

  test("stub adds a method later and returns the double") do
    repo = new Mock("repo")
    assert_eq(repo.stub("sum", fn(a, b) { a + b }), repo)
    assert_eq(repo.sum(2, 3), 5)
  end

  test("reset_calls forgets what was recorded") do
    repo = new Mock("repo", {"count": 3})
    repo.count()
    repo.reset_calls()
    assert_eq(repo.calls(), [])
    assert_eq(repo.call_count("count"), 0)
  end

  test("an unexpected message throws, naming the double") do
    repo = new Mock("repo")
    assert_raises("repo received unexpected message nope") do
      repo.nope()
    end
  end

  test("assert_received fails when the method was never called") do
    repo = new Mock("repo", {"count": 1})
    message = assert_raises("Expected repo to receive count, but it never did") do
      repo.assert_received("count")
    end
    assert_eq(message, "Expected repo to receive count, but it never did")
  end

  test("assert_received fails on other arguments, listing the calls made") do
    repo = new Mock("repo", {"find": fn(id) { id }})
    repo.find(7)
    assert_raises("Expected repo to receive find with [8], got [[7]]") do
      repo.assert_received("find", [8])
    end
  end

  test("assert_not_received fails once the method was called") do
    repo = new Mock("repo", {"find": fn(id) { id }})
    repo.find(7)
    assert_raises("Expected repo not to receive find, but it did 1 time(s)") do
      repo.assert_not_received("find")
    end
  end
end

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

class CatchAll
  def method_missing(name, args)
    "#{name}:#{args.length}"
  end
end

describe("instance method_missing arity") do
  test("a trailing args parameter receives every argument") do
    catcher = new CatchAll()
    assert_eq(catcher.a(), "a:0")
    assert_eq(catcher.b(1, 2, 3), "b:3")
  end
end

describe("Mock.unstub_all") do
  test("undoes the stubs before the test ends") do
    Mock.stub_class(Gateway, "charge", "fake")
    assert_eq(Gateway.charge(1), "fake")
    Mock.unstub_all()
    assert_eq(Gateway.charge(1), "real 1")
  end
end

describe("mock HTTP services") do
  test("mock_http_server_start answers the same port when called again") do
    assert_eq(mock_http_server_start(), port)
  end

  test("a scripted route answers its status and body") do
    mock_http_route("/scripted/item", 201, "{\"id\":1}")
    response = HTTP.request("GET", "#{base}/scripted/item", {})
    assert_eq(response["status"], 201)
    assert_eq(response["body"], "{\"id\":1}")
  end

  test("the query string is ignored when matching a route") do
    mock_http_route("/query/item", 200, "same")
    assert_eq(HTTP.get("#{base}/query/item?page=2"), "same")
  end

  test("scripting a path again replaces its answer") do
    mock_http_route("/rescripted", 200, "first")
    mock_http_route("/rescripted", 404, "gone")
    assert_raises("HTTP 404 error: gone") do
      HTTP.get("#{base}/rescripted")
    end
  end

  test("a path never scripted answers 200 with {\"ok\":true}") do
    assert_eq(HTTP.get("#{base}/never/scripted"), "{\"ok\":true}")
  end

  test("mock_http_last_body returns what was sent") do
    HTTP.post("#{base}/sent/json", {"key": "value"})
    HTTP.post("#{base}/sent/text", "raw text")
    assert_eq(mock_http_last_body("/sent/json"), "{\"key\":\"value\"}")
    assert_eq(mock_http_last_body("/sent/text"), "raw text")
  end

  test("mock_http_last_body is nil when nothing arrived") do
    assert_null(mock_http_last_body("/sent/nothing"))
  end

  test("mock_http_route refuses a status that is not a number") do
    assert_raises("mock_http_route: status must be an HTTP status code, got 200") do
      mock_http_route("/bad", "200", "body")
    end
  end
end
