# Concern hooks (the ActiveSupport::Concern shape): `included do`,
# `extended do`, `class_methods do`, `static def included(base)` /
# `extended(base)`, and the forms `include` accepts.

HOOK_LOG = []

def concern_mark(klass)
  HOOK_LOG.push(klass.inspect)
  klass.define_method("flagged", fn() { "yes" })
end

def concern_log(klass)
  HOOK_LOG.push(klass.inspect)
end

module Flag
  included do
    concern_mark()
  end
end

class FlagHost
  include Flag
end

module FlagBrace
  included {
    concern_mark()
  }
end

class BraceHost
  include FlagBrace
end

module Once
  included do
    concern_log()
  end
end

class OnceHost
  include Once
  include Once
end

module ExtHook
  extended do
    concern_log()
  end

  def ext_name
    "ext"
  end
end

class ExtHost
  extend ExtHook
end

class ExtIncluder
  include ExtHook
end

module IncHook
  included do
    concern_log()
  end
end

class IncExtender
  extend IncHook
end

module Finders
  class_methods do
    static def label
      "finders"
    end

    static def doubled(n)
      n * 2
    end

    def plain
      "plain def"
    end
  end
end

class Record
  include Finders
end

class Untouched
end

module Watch
  static def included(base)
    HOOK_LOG.push("in:" + base.inspect)
  end

  static def extended(base)
    HOOK_LOG.push("ex:" + base.inspect)
  end
end

class WatchHost
  include Watch
end

class WatchExt
  extend Watch
end

module Alpha
  def alpha
    "a"
  end
end

module Beta
  def beta
    "b"
  end
end

class Pair
  include Alpha, Beta
end

class TwoStatements
  include Alpha
  include Beta
end

class ParenHost
  include(Alpha)
end

class NotAModule
  def x
    1
  end
end

def hook_entries(prefix)
  HOOK_LOG.filter { |entry| entry.starts_with?(prefix) }
end

describe("included do") do
  test("runs against the host and can define methods") do
    assert_eq(new FlagHost().flagged, "yes")
    assert_contains(HOOK_LOG, "<class FlagHost>")
  end

  test("the brace form included { } works the same") do
    assert_eq(new BraceHost().flagged, "yes")
  end

  test("does not re-run when the same module is included twice") do
    assert_eq(hook_entries("<class OnceHost>").length, 1)
  end

  test("does not run when the module is extended") do
    assert_eq(hook_entries("<class IncExtender>"), [])
  end
end

describe("extended do") do
  test("runs when the module is extended") do
    assert_eq(hook_entries("<class ExtHost>"), ["<class ExtHost>"])
    assert_eq(ExtHost.ext_name, "ext")
  end

  test("does not run when the module is included") do
    assert_eq(hook_entries("<class ExtIncluder>"), [])
    assert_eq(new ExtIncluder().ext_name, "ext")
  end
end

describe("class_methods do") do
  test("installs class methods on the includer") do
    assert_eq(Record.label, "finders")
    assert_eq(Record.doubled(21), 42)
  end

  test("a plain def inside it becomes a class method too") do
    assert_eq(Record.plain, "plain def")
  end

  test("leaves a class that never included alone") do
    assert_raises("Cannot access property 'label' on Untouched") do
      Untouched.label
    end
    assert_eq(Finders.label, "finders")
  end
end

describe("static def included / extended") do
  test("included receives the host class") do
    assert_eq(hook_entries("in:"), ["in:<class WatchHost>"])
  end

  test("extended receives the extending class") do
    assert_eq(hook_entries("ex:"), ["ex:<class WatchExt>"])
  end
end

describe("include / extend syntax") do
  test("include A, B mixes both") do
    pair = new Pair()
    assert_eq(pair.alpha, "a")
    assert_eq(pair.beta, "b")
  end

  test("two include statements mix both") do
    both = new TwoStatements()
    assert_eq(both.alpha, "a")
    assert_eq(both.beta, "b")
  end

  test("include(A) parenthesized form") do
    assert_eq(new ParenHost().alpha, "a")
  end

  test("cannot include a class") do
    assert_raises("can only include a module (got class NotAModule)") do
      class BadHost
        include NotAModule
      end
    end
  end
end
