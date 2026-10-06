# Mixin modules: `include` copies instance methods, `extend` adds class
# methods, a module's own methods are callable on it, hooks see the host.

module MixinGreetable
  def greet
    "hello, " + @name
  end
end

class MixinUser
  include MixinGreetable

  new(name)
    @name = name
  end
end

module MixinFromModule
  def greet
    "from module"
  end
end

class MixinOverride
  include MixinFromModule

  def greet
    "from class"
  end
end

module MixinBuildable
  def build
    "built"
  end
end

class MixinWidget
  extend MixinBuildable
end

module MixinMathish
  def double(n)
    n * 2
  end
end

module MixinA
  def tag
    "a"
  end
end

module MixinB
  def tag
    "b"
  end

  def extra
    "x"
  end
end

class MixinCombo
  include MixinA
  include MixinB
end

module MixinOnly
  def ping
    "pong"
  end
end

module MixinAdmin
  class User
    def role
      "admin"
    end
  end
end

HOOK_LOG = []

module MixinTracked
  static def included(base)
    HOOK_LOG.push("included:" + base.inspect)
  end

  static def extended(base)
    HOOK_LOG.push("extended:" + base.inspect)
  end
end

class MixinTrackedHost
  include MixinTracked
end

class MixinTrackedExt
  extend MixinTracked
end

module MixinFinders
  class_methods do
    static def label
      "finders"
    end
  end
end

class MixinItem
  include MixinFinders
end

def mixin_mark_host(klass)
  klass.define_method("hooked", fn() { "from-included-do" })
end

module MixinTagged
  included do
    mixin_mark_host()
  end
end

class MixinTaggedHost
  include MixinTagged
end

describe("Mixin modules") do
  test("include copies instance methods") do
    assert_eq(new MixinUser("Ada").greet, "hello, Ada")
  end

  test("class methods win over included methods") do
    assert_eq(new MixinOverride().greet, "from class")
  end

  test("extend adds class methods") do
    assert_eq(MixinWidget.build, "built")
  end

  test("extend does not add instance methods") do
    assert_raises("Cannot access property 'build'") do
      new MixinWidget().build
    end
  end

  test("module methods are callable on the module") do
    assert_eq(MixinMathish.double(21), 42)
  end

  test("first include wins when both define the same method") do
    combo = new MixinCombo()
    assert_eq(combo.tag, "a")
    assert_eq(combo.extra, "x")
  end

  test("cannot instantiate a module") do
    assert_raises("cannot instantiate module MixinOnly") do
      new MixinOnly()
    end
  end

  test("nested class inside a module") do
    assert_eq(new MixinAdmin::User().role, "admin")
  end

  test("static def included / extended receive the host class") do
    assert_eq(HOOK_LOG, ["included:<class MixinTrackedHost>", "extended:<class MixinTrackedExt>"])
  end

  test("class_methods become class methods on the includer") do
    assert_eq(MixinItem.label, "finders")
  end

  test("included do runs against the host class") do
    assert_eq(new MixinTaggedHost().hooked, "from-included-do")
  end
end
