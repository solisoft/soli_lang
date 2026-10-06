# ============================================================================
# `@foo` sugar for `this.foo` — read/write inside class methods.
# ============================================================================

describe("@ sigil desugars to this", fn() {
  test("@foo reads this.foo", fn() {
    class Box
      value: Int

      new(v: Int)
        this.value = v
      end

      def peek -> Int
        @value
      end
    end
    let b = new Box(7)
    assert_eq(b.peek(), 7)
  })

  test("@foo = x writes to this.foo", fn() {
    class Counter
      n: Int

      new()
        this.n = 0
      end

      def bump
        @n = @n + 1
      end

      def get -> Int
        @n
      end
    end
    let c = new Counter()
    c.bump()
    c.bump()
    c.bump()
    assert_eq(c.get(), 3)
  })

  test("@foo and this.foo reference the same field", fn() {
    class Twin
      x: Int

      new()
        this.x = 0
      end

      def via_at(v: Int)
        @x = v
      end

      def via_this -> Int
        return this.x
      end
    end
    let t = new Twin()
    t.via_at(42)
    assert_eq(t.via_this(), 42)
  })

  test("@foo() calls an instance method", fn() {
    class Greeter
      name: String

      new(n: String)
        this.name = n
      end

      def hello -> String
        return "Hello, " + @name_upcase()
      end

      def name_upcase -> String
        return @name.upcase()
      end
    end
    let g = new Greeter("ada")
    assert_eq(g.hello(), "Hello, ADA")
  })

  test("@foo += n compound-assigns to the instance field", fn() {
    class Acc
      total: Int

      new()
        this.total = 10
      end

      def add(n: Int)
        @total += n
      end
    end
    let a = new Acc()
    a.add(5)
    a.add(3)
    assert_eq(a.total, 18)
  })

  test("@foo.bar chains member access", fn() {
    class Inner
      label: String

      new(s: String)
        this.label = s
      end
    end
    class Outer
      inner: Any

      new(i)
        this.inner = i
      end

      def inner_label -> String
        return @inner.label
      end
    end
    let o = new Outer(new Inner("nested"))
    assert_eq(o.inner_label(), "nested")
  })

  test("@foo[k] indexes into a collection field", fn() {
    class Bag
      items: Array

      new()
        this.items = [
          "a",
          "b",
          "c"
        ]
      end

      def second -> String
        return @items[1]
      end

      def push(x: String)
        @items.push(x)
      end
    end
    let b = new Bag()
    assert_eq(b.second(), "b")
    b.push("d")
    assert_eq(b.items.length, 4)
  })

  test("@foo resolves to fields set by the parent class", fn() {
    class Base
      tag: String

      new(t: String)
        this.tag = t
      end
    end
    class Child < Base
      new(t: String)
        super(t)
      end

      def wrapped -> String
        return "<" + @tag + ">"
      end
    end
    let c = new Child("hi")
    assert_eq(c.wrapped(), "<hi>")
  })

  test("@foo mutation persists across method calls", fn() {
    class Log
      entries: Array

      new()
        this.entries = []
      end

      def add(line: String)
        @entries.push(line)
      end

      def count -> Int
        return @entries.length
      end
    end
    let l = new Log()
    l.add("one")
    l.add("two")
    l.add("three")
    assert_eq(l.count(), 3)
  })

  test("@foo works for lazy initialization", fn() {
    class Cache
      value: Any

      new()
        this.value = null
      end

      def get -> Int
        @value = 42 if @value.nil?
        return @value
      end
    end
    let c = new Cache()
    assert_eq(c.get(), 42)
    assert_eq(c.get(), 42)
  })

  test("assigning @foo from a parameter of the same name works", fn() {
    class Rec
      name: String

      new(name: String)
        @name = name
      end
    end
    let r = new Rec("soli")
    assert_eq(r.name, "soli")
  })

  test("@foo reads null for an unset declared field", fn() {
    class MaybeSet
      slot: Any

      def peek -> Any
        return @slot
      end
    end
    let m = new MaybeSet()
    assert_null(m.peek())
  })
})
