# The built-in Error hierarchy, typed catch clauses, and user-defined errors.

class ChildValueError < ValueError
end

class MessageError
  message: String

  new(message: String)
    @message = message
  end
end

class AppError < Error
  message: String

  new(message: String)
    @message = message
  end
end

class NotFoundError < AppError
  new(message: String)
    super(message)
  end
end

class FieldError
  message: String
  field: String

  new(message: String, field: String)
    @message = message
    @field = field
  end
end

# Throws `error` and answers the name of the first clause that caught it.
def caught_by(error)
  clause = nil
  try
    throw error
  catch TypeError e
    clause = "TypeError"
  catch ValueError e
    clause = "ValueError"
  catch KeyError e
    clause = "KeyError"
  catch IndexError e
    clause = "IndexError"
  catch RuntimeError e
    clause = "RuntimeError"
  catch Error e
    clause = "Error"
  catch e
    clause = "catch-all"
  end
  clause
end

TYPED_CATCH_SCRIPT = "/tmp/soli_error_spec_typed_catch.sl"

describe("built-in error classes") do
  test("each subclass is caught by its own typed clause") do
    assert_eq(caught_by(new ValueError()), "ValueError")
    assert_eq(caught_by(new TypeError()), "TypeError")
    assert_eq(caught_by(new KeyError()), "KeyError")
    assert_eq(caught_by(new IndexError()), "IndexError")
    assert_eq(caught_by(new RuntimeError()), "RuntimeError")
  end

  test("a bare Error is caught by catch Error") do
    assert_eq(caught_by(new Error()), "Error")
  end

  test("a thrown value that is not an instance only reaches the catch-all") do
    assert_eq(caught_by("plain string"), "catch-all")
    assert_eq(caught_by(42), "catch-all")
  end

  test("a subclass is caught as Error") do
    result = ""
    try
      throw new ValueError()
    catch Error e
      result = "caught Error"
    end
    assert_eq(result, "caught Error")
  end

  test("the caught value keeps its class") do
    result = ""
    try
      throw new KeyError()
    catch e
      result = e.class
    end
    assert_eq(result, "KeyError")
  end
end

describe("typed catch") do
  test("the first matching clause wins") do
    result = ""
    try
      throw new ValueError()
    catch TypeError e
      result = "TypeError"
    catch ValueError e
      result = "ValueError"
    catch Error e
      result = "Error"
    end
    assert_eq(result, "ValueError")
  end

  test("a clause matches a subclass through inheritance") do
    result = ""
    try
      throw new ChildValueError()
    catch ValueError e
      result = "caught ValueError"
    catch Error e
      result = "caught Error"
    end
    assert_eq(result, "caught ValueError")
  end

  test("a non-matching clause falls through to the next") do
    result = ""
    try
      throw new ValueError()
    catch TypeError e
      result = "TypeError"
    catch Error e
      result = "caught Error"
    end
    assert_eq(result, "caught Error")
  end

  test("with no matching clause the error reaches the outer try") do
    result = ""
    try
      try
        throw new TypeError()
      catch ValueError e
        result = "inner"
      end
    catch e
      result = "outer #{e.class}"
    end
    assert_eq(result, "outer TypeError")
  end

  test("a runtime error is caught as its message string") do
    result = nil
    try
      numbers = [1]
      numbers[99]
    catch e
      result = e
    end
    assert_eq(result.class, "string")
    assert_contains(result, "Index out of bounds: 99")
  end
end

describe("typed catch in a script run on the VM") do
  after_each() do
    File.delete(TYPED_CATCH_SCRIPT) if File.exists(TYPED_CATCH_SCRIPT)
  end

  test("a sole typed catch lets execution continue") do
    pending("bug: on the VM a lone typed catch at script level rethrows `<fn >` after its body")
    soli_path = System.run_sync(["sh", "-c", "command -v soli"])["stdout"].trim
    script = "try\n  throw new ValueError()\ncatch ValueError e\n  print(\"c\")\nend\nprint(\"after\")\n"
    barf(TYPED_CATCH_SCRIPT, script)
    result = System.run_sync([soli_path, TYPED_CATCH_SCRIPT])
    assert_eq(result["stdout"], "c\nafter\n")
  end
end

describe("custom error classes") do
  test("a plain class carries its message to a typed catch") do
    result = ""
    try
      throw new MessageError("custom error")
    catch MessageError e
      result = e.message
    end
    assert_eq(result, "custom error")
  end

  test("a subclass of Error is caught by catch Error") do
    result = ""
    try
      throw new AppError("test error")
    catch Error e
      result = "caught Error: #{e.message}"
    catch e
      result = "other"
    end
    assert_eq(result, "caught Error: test error")
  end

  test("catching the parent catches a grandchild of Error") do
    result = ""
    try
      throw new NotFoundError("missing")
    catch AppError e
      result = "#{e.class}: #{e.message}"
    end
    assert_eq(result, "NotFoundError: missing")
  end

  test("an error can carry several fields") do
    result = ""
    try
      throw new FieldError("invalid", "email")
    catch FieldError e
      result = "#{e.field}: #{e.message}"
    end
    assert_eq(result, "email: invalid")
  end

  test("a thrown hash arrives intact") do
    result = nil
    try
      throw {"code": 404, "message": "no such user"}
    catch e
      result = e
    end
    assert_eq(result, {"code": 404, "message": "no such user"})
  end
end
