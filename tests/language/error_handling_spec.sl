# Error handling: throw, try/catch/finally (and the begin/rescue/ensure
# aliases), typed catch, what catch receives, postfix rescue, and a throw
# crossing a native callback.

class SpecAppError
  message: String

  new(message: String)
    @message = message
  end
end

class SpecNotFoundError < SpecAppError
  new(message: String)
    super(message)
  end
end

class SpecForbiddenError < SpecAppError
  new(message: String)
    super(message)
  end
end

class SpecUnrelatedError
end

class SpecOtherError
end

def throws_oops
  throw "oops"
end

def might_fail(flag: Bool)
  throw "failed" if flag

  "success"
end

finally_log = []

def return_from_try
  try
    return 42
  finally
    finally_log.push("finally")
  end
end

def return_from_catch_clause
  try
    throw "error"
  catch error
    return 100
  finally
    finally_log.push("finally")
  end
end

def divide(a, b)
  throw "Division by zero is not allowed" if b == 0

  a / b
end

def recurse_forever(depth)
  recurse_forever(depth + 1)
end

def find_user(id)
  throw {"code": 404, "message": "no such user"} if id < 1

  {"id": id}
end

describe("throw") do
  test("raises the thrown string as the message") do
    assert_raises("error message") do
      throw "error message"
    end
  end

  test("stops the code after it") do
    reached = false
    assert_raises("stop") do
      throw "stop"
      reached = true
    end
    assert_not(reached)
  end

  test("propagates out of nested function calls") do
    assert_raises("Division by zero is not allowed") do
      divide(10, 0)
    end
    assert_eq(divide(10, 2), 5)
  end

  test("a runtime error raises like a throw") do
    assert_raises("Index out of bounds: 5 (length 1)") do
      [1][5]
    end
  end

  test("unbounded recursion raises a catchable error instead of crashing") do
    # `soli file.sl` raises "call stack too deep (256 frames)" on both engines;
    # inside `soli test` the worker thread's stack overflows first — already at
    # ~200 frames, under the cap — and the whole process aborts ("thread has
    # overflowed its stack").
    pending("bug: in soli test, deep recursion overflows the worker stack and aborts")
    assert_raises("call stack too deep") do
      recurse_forever(0)
    end
  end
end

describe("try/catch") do
  test("a try without an error skips the catch") do
    result = 0
    try
      result = 42
    catch error
      result = -1
    end
    assert_eq(result, 42)
  end

  test("catch handles a thrown error and the rest of try is skipped") do
    result = ""
    try
      throw "error message"
      result = "not reached"
    catch error
      result = "caught: #{error}"
    end
    assert_eq(result, "caught: error message")
  end

  test("the catch variable can be parenthesized") do
    result = ""
    try
      throw "boom"
    catch (error)
      result = "caught: " + error
    end
    assert_eq(result, "caught: boom")
  end

  test("catch without a variable") do
    ran = false
    try
      throw "anonymous"
    catch
      ran = true
    end
    assert(ran)
  end

  test("try/catch/finally with braces") do
    order = []
    try {
      throw "braced"
    } catch (error) {
      order.push("catch: #{error}")
    } finally {
      order.push("finally")
    }
    assert_eq(order, ["catch: braced", "finally"])
  end

  test("nested: a throw from an inner catch reaches the outer catch") do
    result = ""
    try
      try
        throw "inner"
      catch error
        result = "inner caught"
        throw "outer"
      end
    catch error
      result = result + " " + error + " caught"
    end
    assert_eq(result, "inner caught outer caught")
  end
end

describe("finally") do
  before_each() do
    finally_log = []
  end

  test("runs after a try that did not throw") do
    try
      finally_log.push("try")
    catch error
      finally_log.push("catch")
    finally
      finally_log.push("finally")
    end
    assert_eq(finally_log, ["try", "finally"])
  end

  test("runs after the catch") do
    try
      throw "error"
    catch error
      finally_log.push("catch")
    finally
      finally_log.push("finally")
    end
    assert_eq(finally_log, ["catch", "finally"])
  end

  test("runs with no catch clause") do
    try
      finally_log.push("try")
    finally
      finally_log.push("finally")
    end
    assert_eq(finally_log, ["try", "finally"])
  end

  test("runs on an empty try") do
    # soli-lint-disable-next-line style/empty-block
    try
    finally
      finally_log.push("finally")
    end
    assert_eq(finally_log, ["finally"])
  end

  test("runs when try returns, and the return value stands") do
    assert_eq(return_from_try(), 42)
    assert_eq(finally_log, ["finally"])
  end

  test("runs when a catch clause returns") do
    assert_eq(return_from_catch_clause(), 100)
    assert_eq(finally_log, ["finally"])
  end

  test("nested: the inner finally runs before the outer catch") do
    try
      try
        throw "inner"
      finally
        finally_log.push("inner finally")
      end
    catch error
      finally_log.push("outer catch")
    finally
      finally_log.push("outer finally")
    end
    assert_eq(finally_log, ["inner finally", "outer catch", "outer finally"])
  end
end

# `finally` runs on every exit path. The VM once compiled it as straight-line
# code after the catch clauses, so a `return` skipped it and a throw with no
# catch clause was discarded outright.
handles = []

def acquire_then_return -> String
  handles.push("h")
  try
    return "early"
  finally
    handles.pop
  end
end

def acquire_then_throw -> String
  handles.push("h")
  try
    throw "failed"
  finally
    handles.pop
  end
  "unreachable"
end

def throws_through_finally -> String
  try
    throw "BOOM"
  finally
    handles.push("cleaned")
  end
  "SWALLOWED"
end

def return_from_finally -> String
  try
    throw "T"
  finally
    return "from-finally"
  end
end

def throw_from_finally -> String
  try
    throw "T"
  finally
    throw "TF"
  end
end

def return_from_catch_with_cleanup -> String
  try
    throw "x"
  catch error
    return "from-catch"
  finally
    handles.push("cleaned")
  end
end

describe("finally on every exit path") do
  before_each() do
    handles = []
  end

  test("runs when the try block returns, and the return value stands") do
    assert_eq(acquire_then_return(), "early")
    assert_eq(handles, [])
  end

  test("runs when the try block throws") do
    assert_raises("failed") do
      acquire_then_throw()
    end
    assert_eq(handles, [])
  end

  test("does not swallow an exception when there is no catch clause") do
    assert_raises("BOOM") do
      throws_through_finally()
    end
    assert_eq(handles, ["cleaned"])
  end

  test("runs when a catch clause returns") do
    assert_eq(return_from_catch_with_cleanup(), "from-catch")
    assert_eq(handles, ["cleaned"])
  end

  test("a return inside finally replaces a pending exception") do
    assert_eq(return_from_finally(), "from-finally")
  end

  test("a throw inside finally replaces a pending exception") do
    message = assert_raises("TF") do
      throw_from_finally()
    end
    assert_not(message.includes?("T at"))
  end
end

describe("Typed catch") do
  test("matches the thrown class") do
    result = ""
    try
      throw new SpecAppError("oops")
    catch SpecAppError error
      result = "caught: " + error.message
    catch error
      result = "generic"
    end
    assert_eq(result, "caught: oops")
  end

  test("skips a non-matching class and tries the next clause") do
    result = ""
    try
      throw new SpecOtherError()
    catch SpecUnrelatedError error
      result = "unrelated"
    catch SpecOtherError error
      result = "other"
    end
    assert_eq(result, "other")
  end

  test("matches a subclass through inheritance") do
    result = ""
    try
      throw new SpecNotFoundError("child")
    catch SpecAppError error
      result = "base caught: " + error.message
    end
    assert_eq(result, "base caught: child")
  end

  test("picks the first matching clause among several") do
    result = ""
    try
      throw new SpecForbiddenError("no access")
    catch SpecNotFoundError error
      result = "404"
    catch SpecForbiddenError error
      result = "403: " + error.message
    catch error
      result = "other"
    end
    assert_eq(result, "403: no access")
  end

  test("a bare catch catches a value that is not an instance") do
    result = ""
    try
      throw "a string"
    catch error
      result = "bare: " + error
    end
    assert_eq(result, "bare: a string")
  end

  test("a typed clause does not match a thrown string") do
    result = ""
    try
      throw "a string"
    catch SpecUnrelatedError error
      result = "typed"
    catch error
      result = "bare: " + error
    end
    assert_eq(result, "bare: a string")
  end

  test("with no matching clause the error goes to the outer try") do
    result = ""
    try
      try
        throw new SpecOtherError()
      catch SpecUnrelatedError error
        result = "inner"
      end
    catch error
      result = "outer: #{type(error)}"
    end
    assert_eq(result, "outer: SpecOtherError")
  end

  test("works with finally") do
    result = ""
    finally_ran = false
    try
      throw new SpecAppError("test")
    catch SpecAppError error
      result = error.message
    finally
      finally_ran = true
    end
    assert_eq(result, "test")
    assert(finally_ran)
  end
end

describe("What catch receives") do
  test("the thrown value, with its type") do
    caught = nil
    try
      throw 42
    catch error
      caught = error
    end
    assert_eq(caught, 42)
    assert_eq(type(caught), "int")
  end

  test("a thrown hash stays a hash across function calls") do
    caught = nil
    try
      find_user(0)
    catch error
      caught = error
    end
    assert_eq(caught, {"code": 404, "message": "no such user"})
    assert_eq(caught.class, "hash")
  end

  test("a thrown array stays an array") do
    caught = nil
    try
      throw [1, 2, 3]
    catch error
      caught = error
    end
    assert_eq(caught[1], 2)
  end

  test("a thrown nil is caught as nil") do
    caught = "unset"
    try
      throw nil
    catch error
      caught = error
    end
    assert_null(caught)
  end

  test("a runtime error is caught as its message string") do
    caught = nil
    try
      [1][5]
    catch error
      caught = error
    end
    assert_eq(caught.class, "string")
    assert_contains(caught, "Index out of bounds: 5 (length 1)")
  end
end

# begin/rescue/ensure are aliases of try/catch/finally. `soli fmt` rewrites
# them to the canonical keywords, which would take this coverage away: keep the
# aliases here.
describe("Ruby-style begin/rescue/ensure") do
  test("begin/rescue/ensure runs the rescue then the ensure") do
    order = []
    begin
      throw "boom"
    rescue error
      order.push("rescue: " + error)
    ensure
      order.push("ensure")
    end
    assert_eq(order, ["rescue: boom", "ensure"])
  end

  test("rescue without ensure binds the error") do
    caught = ""
    begin
      throw "kaboom"
    rescue failure
      caught = failure
    end
    assert_eq(caught, "kaboom")
  end

  test("rescue with no binding") do
    ran = false
    begin
      throw "anonymous"
    rescue
      ran = true
    end
    assert(ran)
  end

  test("ensure without rescue still runs") do
    ran = false
    begin
      ran = "body"
    ensure
      ran = "#{ran}+ensure"
    end
    assert_eq(ran, "body+ensure")
  end

  test("a postfix rescue still works inside a begin body") do
    value = 0
    begin
      value = (10 / 0) rescue 99
    rescue error
      value = -1
    end
    assert_eq(value, 99)
  end

  test("a begin with no error skips the rescue") do
    result = 0
    begin
      result = 42
    rescue error
      result = -1
    end
    assert_eq(result, 42)
  end
end

describe("Postfix rescue") do
  test("returns the fallback when the expression throws") do
    assert_eq(throws_oops() rescue "fallback", "fallback")
  end

  test("returns the expression's value when it does not") do
    assert_eq(42 rescue "fallback", 42)
  end

  test("catches a runtime error too") do
    assert_eq((10 / 0) rescue "division", "division")
  end

  test("on a lambda call") do
    failing = fn() { throw "error" }
    assert_eq(failing() rescue "recovered", "recovered")
  end

  test("in an assignment") do
    value = throws_oops() rescue "default"
    assert_eq(value, "default")
  end

  test("chooses per call") do
    assert_eq(might_fail(true) rescue "oops", "oops")
    assert_eq(might_fail(false) rescue "oops", "success")
  end

  test("nests: the inner rescue answers first") do
    result = (throws_oops() rescue "inner") rescue "outer"
    assert_eq(result, "inner")
  end

  test("the fallback is a whole expression") do
    assert_eq(throws_oops() rescue 1 + 2 * 3, 7)
  end

  test("the fallback takes ||") do
    assert_eq(throws_oops() rescue "a" || "b", "a")
  end

  test("the fallback takes ??") do
    assert_eq(throws_oops() rescue nil ?? "default", "default")
  end

  test("on a pipeline into a throwing lambda") do
    result = 5 |> fn(x) { throw "fail" } rescue 100
    assert_eq(result, 100)
  end

  test("after a member access") do
    service = {"get": fn() { throw "err" }}
    assert_eq(service.get() rescue "fallback", "fallback")
  end

  test("a fallback of nil") do
    assert_null(throws_oops() rescue nil)
  end
end

# map/filter/each/reduce/sort_by and friends drive the callback from Rust, so a
# throw has to cross that boundary to reach the caller's catch. Both engines
# once destroyed it there, and sort_by swallowed it outright. These assert the
# VALUE, not just that something was raised.
describe("throw inside a native callback") do
  test("map keeps the thrown hash a hash") do
    code = 0
    try
      [1, 2, 3].map do |x|
        throw {"code": 404}
      end
    catch error
      code = error["code"]
    end
    assert_eq(code, 404)
  end

  test("filter keeps the thrown value") do
    code = 0
    try
      [1, 2, 3].filter do |x|
        throw {"code": 422}
      end
    catch error
      code = error["code"]
    end
    assert_eq(code, 422)
  end

  test("reduce keeps the thrown value") do
    code = 0
    try
      [1, 2, 3].reduce(fn(acc, x) { throw {"code": 500} }, 0)
    catch error
      code = error["code"]
    end
    assert_eq(code, 500)
  end

  test("each keeps the thrown value") do
    code = 0
    try
      [1, 2, 3].each do |x|
        throw {"code": 418}
      end
    catch error
      code = error["code"]
    end
    assert_eq(code, 418)
  end

  test("sort_by does not swallow the throw") do
    reached = false
    code = 0
    try
      [3, 1, 2].sort_by do |x|
        throw {"code": 409}
      end
      reached = true
    catch error
      code = error["code"]
    end
    assert_not(reached)
    assert_eq(code, 409)
  end

  test("hash each keeps the thrown value") do
    code = 0
    pairs = {"a": 1, "b": 2}
    try
      pairs.each do |key, value|
        throw {"code": 404}
      end
    catch error
      code = error["code"]
    end
    assert_eq(code, 404)
  end

  test("int times keeps the thrown value") do
    code = 0
    try
      3.times do |i|
        throw {"code": 404}
      end
    catch error
      code = error["code"]
    end
    assert_eq(code, 404)
  end

  test("a thrown class instance is still an instance") do
    message = ""
    try
      [1].map do |x|
        throw new SpecAppError("in-map")
      end
    catch SpecAppError error
      message = error.message
    end
    assert_eq(message, "in-map")
  end
end
