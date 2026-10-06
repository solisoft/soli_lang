# `static { … }` blocks: run once, when the class is defined, with `this` bound
# to the class itself.

events = []
events.push("before")

class Loader
  static {
    events.push("static")
    this.count = 1
  }
end

events.push("after")

class LoaderChild < Loader
end

class Settings
  static {
    Settings.timeout = 30
    Settings.max_retries = 3
  }
end

class Processor
  static {
    Processor.initialized = true
    Processor.start_time = 100
    Processor.end_time = Processor.start_time * 2
  }
end

def answer
  42
end

class MathHelper
  static {
    MathHelper.result = answer()
  }
end

class Flags
  static {
    if answer() > 40
      this.mode = "big"
    else
      this.mode = "small"
    end
  }
end

class Counter
  static {
    this.sum = 0
    for i in [1, 2, 3, 4, 5]
      this.sum = this.sum + i
    end
  }
end

class Tunable
  static {
    this.level = 1
  }
end

class TwoBlocks
  static {
    this.first = 1
  }

  static {
    this.second = 2
  }
end

describe("Static block") do
  test("runs exactly once, at the point the class is defined") do
    assert_eq(events, ["before", "static", "after"])
  end

  test("defines static fields by assigning on the class name") do
    assert_eq(Settings.timeout, 30)
    assert_eq(Settings.max_retries, 3)
  end

  test("binds this to the class") do
    assert_eq(Loader.count, 1)
  end

  test("a later statement reads a field set earlier in the block") do
    assert_eq(Processor.initialized, true)
    assert_eq(Processor.start_time, 100)
    assert_eq(Processor.end_time, 200)
  end

  test("can call a top-level function") do
    assert_eq(MathHelper.result, 42)
  end

  test("runs control flow") do
    assert_eq(Flags.mode, "big")
  end

  test("runs loops") do
    assert_eq(Counter.sum, 15)
  end

  test("a subclass reads the fields its parent's block set") do
    assert_eq(LoaderChild.count, 1)
  end

  test("a field it set stays writable from outside") do
    Tunable.level = 5
    assert_eq(Tunable.level, 5)
  end

  test("every block of a class runs") do
    pending("bug: with two static blocks, the parser keeps only the last (src/parser/declarations.rs)")
    assert_eq(TwoBlocks.second, 2)
    assert_eq(TwoBlocks.first, 1)
  end

  test("an error raised in the block propagates") do
    pending("bug: the tree engine swallows a static-block error when the class is declared inside a function")
    assert_raises("boom") do
      class Exploding
        static {
          throw "boom"
        }
      end
    end
  end
end
