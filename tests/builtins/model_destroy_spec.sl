# `destroy` is not a model method — `delete` is. It used to read a field that
# was never set and "call" nil: the row stayed, and the caller was told
# nothing. It raises, with or without parentheses; a model that defines its
# own `destroy` keeps it. No database needed: nothing here is saved.

class DestroyGadget < Model
end

class DestroyTool < Model
  def destroy
    "its own"
  end
end

describe("Model#destroy") do
  test("says to use delete, called with parentheses") do
    message = ""
    try
      DestroyGadget.new.destroy()
    catch error
      message = "#{error}"
    end
    assert(message.includes?("use `delete`"))
  end

  test("says so without parentheses too") do
    message = ""
    try
      DestroyGadget.new.destroy
    catch error
      message = "#{error}"
    end
    assert(message.includes?("use `delete`"))
  end

  test("a model's own destroy is the model's") do
    assert_eq(DestroyTool.new.destroy, "its own")
  end
end
