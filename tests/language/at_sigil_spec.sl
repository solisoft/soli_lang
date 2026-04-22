// ============================================================================
// `@foo` sugar for `this.foo` — read/write inside class methods.
// ============================================================================

describe("@ sigil desugars to this", fn() {
    test("@foo reads this.foo", fn() {
        class Box {
            value: Int;

            new(v: Int) {
                this.value = v;
            }

            fn peek() -> Int {
                @value
            }
        }
        let b = new Box(7);
        assert_eq(b.peek(), 7);
    });

    test("@foo = x writes to this.foo", fn() {
        class Counter {
            n: Int;

            new() {
                this.n = 0;
            }

            fn bump() {
                @n = @n + 1;
            }

            fn get() -> Int {
                @n
            }
        }
        let c = new Counter();
        c.bump();
        c.bump();
        c.bump();
        assert_eq(c.get(), 3);
    });

    test("@foo and this.foo reference the same field", fn() {
        class Twin {
            x: Int;

            new() {
                this.x = 0;
            }

            fn via_at(v: Int) {
                @x = v;
            }

            fn via_this() -> Int {
                return this.x;
            }
        }
        let t = new Twin();
        t.via_at(42);
        assert_eq(t.via_this(), 42);
    });

    test("@foo() calls an instance method", fn() {
        class Greeter {
            name: String;

            new(n: String) {
                this.name = n;
            }

            fn hello() -> String {
                return "Hello, " + @name_upcase();
            }

            fn name_upcase() -> String {
                return @name.upcase();
            }
        }
        let g = new Greeter("ada");
        assert_eq(g.hello(), "Hello, ADA");
    });

    test("@foo += n compound-assigns to the instance field", fn() {
        class Acc {
            total: Int;

            new() {
                this.total = 10;
            }

            fn add(n: Int) {
                @total += n;
            }
        }
        let a = new Acc();
        a.add(5);
        a.add(3);
        assert_eq(a.total, 18);
    });

    test("@foo.bar chains member access", fn() {
        class Inner {
            label: String;

            new(s: String) {
                this.label = s;
            }
        }
        class Outer {
            inner: Any;

            new(i: Any) {
                this.inner = i;
            }

            fn inner_label() -> String {
                return @inner.label;
            }
        }
        let o = new Outer(new Inner("nested"));
        assert_eq(o.inner_label(), "nested");
    });

    test("@foo[k] indexes into a collection field", fn() {
        class Bag {
            items: Array;

            new() {
                this.items = ["a", "b", "c"];
            }

            fn second() -> String {
                return @items[1];
            }

            fn push(x: String) {
                @items.push(x);
            }
        }
        let b = new Bag();
        assert_eq(b.second(), "b");
        b.push("d");
        assert_eq(b.items.length, 4);
    });

    test("@foo resolves to fields set by the parent class", fn() {
        class Base {
            tag: String;

            new(t: String) {
                this.tag = t;
            }
        }
        class Child extends Base {
            new(t: String) {
                super(t);
            }

            fn wrapped() -> String {
                return "<" + @tag + ">";
            }
        }
        let c = new Child("hi");
        assert_eq(c.wrapped(), "<hi>");
    });

    test("@foo mutation persists across method calls", fn() {
        class Log {
            entries: Array;

            new() {
                this.entries = [];
            }

            fn add(line: String) {
                @entries.push(line);
            }

            fn count() -> Int {
                return @entries.length;
            }
        }
        let l = new Log();
        l.add("one");
        l.add("two");
        l.add("three");
        assert_eq(l.count(), 3);
    });

    test("@foo works for lazy initialization", fn() {
        class Cache {
            value: Any;

            new() {
                this.value = null;
            }

            fn get() -> Int {
                if @value == null {
                    @value = 42;
                }
                return @value;
            }
        }
        let c = new Cache();
        assert_eq(c.get(), 42);
        assert_eq(c.get(), 42);
    });

    test("assigning @foo from a parameter of the same name works", fn() {
        class Rec {
            name: String;

            new(name: String) {
                @name = name;
            }
        }
        let r = new Rec("soli");
        assert_eq(r.name, "soli");
    });

    test("@foo reads null for an unset declared field", fn() {
        class MaybeSet {
            slot: Any;

            fn peek() -> Any {
                return @slot;
            }
        }
        let m = new MaybeSet();
        assert_null(m.peek());
    });
});
