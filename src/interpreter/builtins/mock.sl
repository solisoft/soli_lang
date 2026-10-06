# Mock — engine-embedded Soli stdlib for test doubles (see builtins::mock).
#
#   repo = new Mock("repo", {"find": fn(id) { {"id": id} }, "count": 3})
#   repo.find(7)                      # => {"id": 7}, recorded
#   repo.assert_received("find", [7])
#   repo.assert_not_received("delete")
#
# Stubs are a hash of method name => value, or a lambda called with the
# arguments (up to four). Call a double's methods WITH parentheses
# (`repo.count()`): a bare `repo.count` yields the bound method, not its
# result. A message with no stub throws, so a typo fails loudly.

class Mock
  label: String
  stubs: Hash
  calls_made: Array

  new(label: String = "mock", stubs: Hash = {})
    @label = label
    @stubs = stubs
    @calls_made = []
  end

  # Replace `method` on a real class for the rest of the current test:
  #   Mock.stub_class(Post, "find", fn(id) { fake_post })
  # Inside the test, `Post.find(1)` reaches the stub from any code that runs in
  # this process. Undone automatically when the test ends (a stub set in
  # `before_all` lasts the suite). Returns the Mock, so `assert_received` works
  # on it. Only the stubbed method is replaced.
  static def stub_class(target, method, handler)
    stubs = {}
    stubs[method] = handler
    mock = new Mock(__mock_class_name(target) + "." + method, stubs)
    __mock_register_stub(target, method, mock, false, false)
    mock
  end

  # The same for an instance method: every instance of `target` (or a
  # subclass) answers `method` with the stub.
  static def stub_instance(target, method, handler)
    stubs = {}
    stubs[method] = handler
    mock = new Mock(__mock_class_name(target) + "#" + method, stubs)
    __mock_register_stub(target, method, mock, true, false)
    mock
  end

  # A spy: the call is recorded on the returned Mock and the REAL method runs.
  #   spy = Mock.spy_class(Gateway, "charge")
  #   Checkout.new.pay(5)            # really charges
  #   spy.assert_received("charge", [5])
  static def spy_class(target, method)
    mock = new Mock(__mock_class_name(target) + "." + method, {})
    __mock_register_stub(target, method, mock, false, true)
    mock
  end

  static def spy_instance(target, method)
    mock = new Mock(__mock_class_name(target) + "#" + method, {})
    __mock_register_stub(target, method, mock, true, true)
    mock
  end

  # RSpec-shaped chains, for stubbing one method on a real class:
  #   Mock.allow(Gateway).to_receive("charge").and_return({"ok": true})
  #   Mock.allow(Gateway).to_receive("charge").and_call(fn(amount) { ... })
  #   Mock.allow(Gateway).to_receive("charge").and_call_original
  #   Mock.allow_any_instance(User).to_receive("save").and_return(true)
  # Each chain ends by returning the Mock, so `assert_received` works on it.
  static def allow(target)
    new MockAllowance(target, false)
  end

  static def allow_any_instance(target)
    new MockAllowance(target, true)
  end

  static def unstub_all
    __mock_clear_stubs()
  end

  def stub(name, value)
    @stubs[name] = value
    this
  end

  def calls
    @calls_made
  end

  def calls_to(name)
    @calls_made.filter { |call| call["name"] == name }
  end

  def call_count(name)
    @calls_to(name).length
  end

  def received?(name)
    @call_count(name) > 0
  end

  def reset_calls
    @calls_made = []
    this
  end

  def assert_received(name, expected_args = nil)
    matching = @calls_to(name)
    throw "Expected " + @label + " to receive " + name + ", but it never did" if matching.length == 0
    unless expected_args.nil?
      found = matching.any? { |call| call["args"] == expected_args }
      unless found
        received_args = matching.map { |call| call["args"] }
        throw "Expected " + @label + " to receive " + name + " with " + str(expected_args) + ", got " + str(received_args)
      end
    end
    # Counted like any other assertion, so a test that only checks a mock is
    # not reported as asserting nothing.
    __mock_count_assertion()
    true
  end

  def assert_not_received(name)
    count = @call_count(name)
    throw "Expected " + @label + " not to receive " + name + ", but it did " + str(count) + " time(s)" if count > 0

    __mock_count_assertion()
    true
  end

  def invoke(handler, args)
    return handler() if args.length == 0
    return handler(args[0]) if args.length == 1
    return handler(args[0], args[1]) if args.length == 2
    return handler(args[0], args[1], args[2]) if args.length == 3

    handler(args[0], args[1], args[2], args[3])
  end

  def method_missing(name, args)
    throw @label + " received unexpected message " + name unless @stubs.has_key(name)

    @calls_made.push({"name": name, "args": args})
    handler = @stubs[name]
    return @invoke(handler, args) if type(handler) == "Function"

    handler
  end
end

class MockAllowance
  target: Any
  instance_level: Bool

  new(target, instance_level)
    @target = target
    @instance_level = instance_level
  end

  def to_receive(method)
    new MockReceive(@target, method, @instance_level)
  end
end

class MockReceive
  target: Any
  method: String
  instance_level: Bool

  new(target, method, instance_level)
    @target = target
    @method = method
    @instance_level = instance_level
  end

  def and_return(value)
    @and_call(value)
  end

  # `handler` is a value, or a lambda called with the arguments.
  def and_call(handler)
    return Mock.stub_instance(@target, @method, handler) if @instance_level

    Mock.stub_class(@target, @method, handler)
  end

  def and_call_original
    return Mock.spy_instance(@target, @method) if @instance_level

    Mock.spy_class(@target, @method)
  end
end
