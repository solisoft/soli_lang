# Errors about an unknown name suggest a close known one.

def compute_total(items)
  items.length
end

class Invoice
  amount: Int

  new(amount: Int)
    @amount = amount
  end

  def total_with_tax
    @amount * 2
  end

  static def build_default
    Invoice.new(1)
  end
end

def greet(name: String = "you", greeting: String = "Hello")
  "#{greeting}, #{name}"
end

describe("did you mean") do
  test("an undefined function names the close one") do
    # soli-lint-disable-next-line smell/undefined-local
    assert_raises("Undefined variable 'compute_totl' (did you mean 'compute_total'?)") do
      compute_totl([1])
    end
  end

  test("an unknown String method names the built-in") do
    assert_raises("Cannot access property 'upcse' on String (did you mean 'upcase'?)") do
      "abc".upcse
    end
  end

  test("an unknown Array method names the built-in") do
    assert_raises("(did you mean 'length'?)") do
      [1, 2].lenght
    end
  end

  test("an unknown instance method names the class's method") do
    invoice = Invoice.new(3)
    assert_raises("(did you mean 'total_with_tax'?)") do
      invoice.total_with_tx
    end
  end

  test("an unknown static method names the class method") do
    assert_raises("(did you mean 'build_default'?)") do
      Invoice.build_defualt
    end
  end

  test("an unknown named argument names the parameter") do
    assert_raises("Undefined variable 'greting' (did you mean 'greeting'?)") do
      greet(greting: "Hi")
    end
  end

  test("a name with nothing close gets no suggestion") do
    error = ""
    try
      "abc".zzzzqqq
    catch e
      error = e.to_s
    end
    assert(error.includes?("Cannot access property 'zzzzqqq' on String"))
    assert(!error.includes?("did you mean"))
  end
end
