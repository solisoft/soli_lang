// ============================================================================
// Concern hooks: included / extended / class_methods
// ============================================================================

describe("included do", fn() {
    test("runs against the host and can define methods", fn() {
        def concern_mark(klass)
            klass.define_method("flagged", fn() { "yes" })
        end

        module Flag
            included do
                concern_mark()
            end
        end

        class Host
            include Flag
        end

        assert_eq(new Host().flagged, "yes")
    })

    test("brace form included { } works", fn() {
        def concern_mark_brace(klass)
            klass.define_method("braced", fn() { true })
        end

        module FlagBrace
            included {
                concern_mark_brace()
            }
        end

        class HostBrace
            include FlagBrace
        end

        assert(new HostBrace().braced)
    })

    test("does not re-run when the same module is included twice", fn() {
        COUNTER = []

        def concern_tick(klass)
            COUNTER.push(klass.inspect)
        end

        module Once
            included do
                concern_tick()
            end
        end

        class OnceHost
            include Once
            include Once
        end

        assert_eq(len(COUNTER), 1)
    })
})

describe("extended do", fn() {
    test("runs when the module is extended", fn() {
        EXT_LOG = []

        def concern_mark_ext(klass)
            EXT_LOG.push(klass.inspect)
        end

        module ExtHook
            extended do
                concern_mark_ext()
            end
        end

        class ExtHost
            extend ExtHook
        end

        assert(EXT_LOG.includes?("<class ExtHost>"))
    })
})

describe("class_methods do", fn() {
    test("installs class methods on the includer", fn() {
        module Finders
            class_methods do
                def label
                    "finders"
                end

                def doubled(n)
                    n * 2
                end
            end
        end

        class Record
            include Finders
        end

        assert_eq(Record.label, "finders")
        assert_eq(Record.doubled(21), 42)
    })

    test("does not put class_methods on a class that never included", fn() {
        module OnlyFinders
            class_methods do
                def only_here
                    1
                end
            end
        end

        class Untouched
        end

        let raised = false
        try
            Untouched.only_here
        catch e
            raised = true
        end
        assert(raised)
        assert_eq(OnlyFinders.only_here, 1)
    })
})

describe("def self.included / self.extended", fn() {
    test("included receives the host class", fn() {
        SEEN = []

        module Watch
            def self.included(base)
                SEEN.push("in:" + base.inspect)
            end

            def self.extended(base)
                SEEN.push("ex:" + base.inspect)
            end
        end

        class WatchHost
            include Watch
        end

        class WatchExt
            extend Watch
        end

        assert(SEEN.includes?("in:<class WatchHost>"))
        assert(SEEN.includes?("ex:<class WatchExt>"))
    })
})

describe("include / extend syntax", fn() {
    test("include A, B mixes both", fn() {
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

        let p = new Pair()
        assert_eq(p.alpha, "a")
        assert_eq(p.beta, "b")
    })

    test("include(A) parenthesized form", fn() {
        module Gamma
            def gamma
                "g"
            end
        end

        class ParenHost
            include(Gamma)
        end

        assert_eq(new ParenHost().gamma, "g")
    })

    test("cannot include a class", fn() {
        class NotAModule
            def x
                1
            end
        end

        let raised = false
        try
            class Bad
                include NotAModule
            end
        catch e
            raised = true
        end
        assert(raised)
    })
})
