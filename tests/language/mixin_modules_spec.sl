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
    def self.included(base)
        HOOK_LOG.push("included:" + base.inspect)
    end

    def self.extended(base)
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
        def label
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

describe("Mixin modules", fn() {
    test("include copies instance methods", fn() {
        assert_eq(new MixinUser("Ada").greet, "hello, Ada")
    })

    test("class methods win over included methods", fn() {
        assert_eq(new MixinOverride().greet, "from class")
    })

    test("extend adds class methods", fn() {
        assert_eq(MixinWidget.build, "built")
    })

    test("module methods are callable on the module", fn() {
        assert_eq(MixinMathish.double(21), 42)
    })

    test("first include wins when both define the same method", fn() {
        let c = new MixinCombo()
        assert_eq(c.tag, "a")
        assert_eq(c.extra, "x")
    })

    test("cannot instantiate a module", fn() {
        let raised = false
        try
            new MixinOnly()
        catch e
            raised = true
        end
        assert(raised)
    })

    test("nested class inside a module", fn() {
        assert_eq(new MixinAdmin::User().role, "admin")
    })

    test("self.included and self.extended receive the host class", fn() {
        assert(HOOK_LOG.includes?("included:<class MixinTrackedHost>"))
        assert(HOOK_LOG.includes?("extended:<class MixinTrackedExt>"))
    })

    test("class_methods become class methods on the includer", fn() {
        assert_eq(MixinItem.label, "finders")
    })

    test("included do runs against the host class", fn() {
        assert_eq(new MixinTaggedHost().hooked, "from-included-do")
    })
})
