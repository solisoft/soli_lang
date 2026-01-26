// ============================================================================
// Class Inheritance Test Suite
// ============================================================================

describe("Class Inheritance Basics", fn() {
    test("subclass inherits from superclass", fn() {
        class Animal {
            name: String;

            new(name: String) {
                this.name = name;
            }

            fn speak() -> String {
                return "...";
            }
        }

        class Dog extends Animal {
            fn speak() -> String {
                return "Woof!";
            }
        }

        let dog = new Dog("Buddy");
        assert_eq(dog.name, "Buddy");
        assert_eq(dog.speak(), "Woof!");
    });

    test("subclass can extend inherited behavior", fn() {
        class Shape {
            fn description() -> String {
                return "A shape";
            }
        }

        class Circle extends Shape {
            radius: Float;

            new(radius: Float) {
                this.radius = radius;
            }

            fn area() -> Float {
                return 3.14159 * this.radius * this.radius;
            }
        }

        let c = new Circle(5.0);
        assert_eq(c.description(), "A shape");
        assert_eq(c.area(), 78.53975);
    });

    test("deep inheritance chain", fn() {
        class A {
            fn method_a() -> String {
                return "A";
            }
        }

        class B extends A {
            fn method_b() -> String {
                return "B";
            }
        }

        class C extends B {
            fn method_c() -> String {
                return "C";
            }
        }

        let c = new C();
        assert_eq(c.method_a(), "A");
        assert_eq(c.method_b(), "B");
        assert_eq(c.method_c(), "C");
    });

    test("instance type shows most derived class", fn() {
        class Base {
        }

        class Derived extends Base {
        }

        let d = new Derived();
        assert_eq(type(d), "Derived");
    });
});

describe("super Keyword", fn() {
    test("super.method() in instance methods", fn() {
        class Base {
            fn greet() -> String {
                return "Hello";
            }
        }

        class Derived extends Base {
            fn greet() -> String {
                return super.greet() + " World";
            }
        }

        assert_eq(new Derived().greet(), "Hello World");
    });

    test("super with field access", fn() {
        class Base {
            value: Int = 10;
        }

        class Derived extends Base {
            fn get_value() -> Int {
                return this.value;
            }
        }

        assert_eq(new Derived().get_value(), 10);
    });

    test("super in constructor", fn() {
        class Person {
            name: String;

            new(name: String) {
                this.name = name;
            }
        }

        class Employee extends Person {
            employee_id: Int;

            new(name: String, id: Int) {
                super(name);
                this.employee_id = id;
            }
        }

        let e = new Employee("Alice", 123);
        assert_eq(e.name, "Alice");
        assert_eq(e.employee_id, 123);
    });

    test("super chaining in deep hierarchy", fn() {
        class GrandParent {
            fn identify() -> String {
                return "GrandParent";
            }
        }

        class Parent extends GrandParent {
            fn identify() -> String {
                return super.identify() + " -> Parent";
            }
        }

        class Child extends Parent {
            fn identify() -> String {
                return super.identify() + " -> Child";
            }
        }

        assert_eq(new Child().identify(), "GrandParent -> Parent -> Child");
    });

    test("super in static method", fn() {
        class Base {
            static fn get_class_name() -> String {
                return "Base";
            }
        }

        class Derived extends Base {
            static fn get_class_name() -> String {
                return super.get_class_name() + "_Derived";
            }
        }

        assert_eq(Derived.get_class_name(), "Base_Derived");
    });

    test("super with static method inheritance", fn() {
        class Logger {
            static fn level() -> String {
                return "INFO";
            }
        }

        class DebugLogger extends Logger {
        }

        assert_eq(DebugLogger.level(), "INFO");
    });

    test("super in multiple inheritance levels", fn() {
        class Level1 {
            fn level() -> Int {
                return 1;
            }
        }

        class Level2 extends Level1 {
            fn level() -> Int {
                return super.level() + 10;
            }
        }

        class Level3 extends Level2 {
            fn level() -> Int {
                return super.level() + 100;
            }
        }

        assert_eq(new Level3().level(), 111);
    });
});

describe("this Keyword", fn() {
    test("this.field access in methods", fn() {
        class Point {
            x: Int;
            y: Int;

            new(x: Int, y: Int) {
                this.x = x;
                this.y = y;
            }

            fn get_x() -> Int {
                return this.x;
            }

            fn get_y() -> Int {
                return this.y;
            }
        }

        let p = new Point(5, 10);
        assert_eq(p.get_x(), 5);
        assert_eq(p.get_y(), 10);
    });

    test("this.method() calls", fn() {
        class Chainer {
            value: Int = 0;

            fn add(n: Int) {
                this.value = this.value + n;
                return this;
            }

            fn multiply(n: Int) {
                this.value = this.value * n;
                return this;
            }

            fn reset() {
                this.value = 0;
                return this;
            }
        }

        let c = new Chainer();
        assert_eq(c.add(5).multiply(2).value, 10);
        assert_eq(c.reset().add(3).value, 3);
    });

    test("this in constructor", fn() {
        class Box {
            width: Int;
            height: Int;
            depth: Int;

            new(w: Int, h: Int, d: Int) {
                this.width = w;
                this.height = h;
                this.depth = d;
            }

            fn volume() -> Int {
                return this.width * this.height * this.depth;
            }
        }

        let box = new Box(2, 3, 4);
        assert_eq(box.volume(), 24);
    });

    test("this in nested method calls", fn() {
        class Outer {
            value: Int = 100;
        }

        let o = new Outer();
        assert_eq(o.value, 100);
    });

    test("this in static context throws error", fn() {
        let threw = false;
        try {
            class Test {
                static fn bad() {
                    return this;
                }
            }
            Test.bad();
        } catch (e) {
            threw = true;
        }
        assert(threw);
    });
});

describe("Method Overriding", fn() {
    test("method override completely replaces super", fn() {
        class Base {
            fn get_value() -> Int {
                return 1;
            }
        }

        class Derived extends Base {
            fn get_value() -> Int {
                return 2;
            }
        }

        assert_eq(new Base().get_value(), 1);
        assert_eq(new Derived().get_value(), 2);
    });

    test("override with super call", fn() {
        class Base {
            fn compute(x: Int) -> Int {
                return x * 2;
            }
        }

        class Derived extends Base {
            fn compute(x: Int) -> Int {
                let result = super.compute(x);
                return result + 1;
            }
        }

        assert_eq(new Derived().compute(5), 11);
    });

    test("override with different signature", fn() {
        class Base {
            fn process(data: String) -> String {
                return "processed: " + data;
            }
        }

        class Derived extends Base {
            fn process(data: String, prefix: String) -> String {
                return prefix + ": " + data;
            }
        }

        assert_eq(new Base().process("test"), "processed: test");
    });

    test("override adds new methods", fn() {
        class Base {
            fn existing() -> String {
                return "exists";
            }
        }

        class Derived extends Base {
            fn new_method() -> String {
                return "new";
            }
        }

        let d = new Derived();
        assert_eq(d.existing(), "exists");
        assert_eq(d.new_method(), "new");
    });
});

describe("Constructor Behavior", fn() {
    test("default constructor when no new defined", fn() {
        class Simple {
            value: Int = 42;
        }

        let s = new Simple();
        assert_eq(s.value, 42);
    });

    test("custom constructor", fn() {
        class Rectangle {
            width: Int;
            height: Int;

            new(w: Int, h: Int) {
                this.width = w;
                this.height = h;
            }

            fn area() -> Int {
                return this.width * this.height;
            }
        }

        let r = new Rectangle(5, 3);
        assert_eq(r.area(), 15);
    });

    test("constructor with default parameters", fn() {
        class Box {
            width: Int;
            height: Int;
            depth: Int;

            new(w: Int, h: Int = 1, d: Int = 1) {
                this.width = w;
                this.height = h;
                this.depth = d;
            }

            fn volume() -> Int {
                return this.width * this.height * this.depth;
            }
        }

        assert_eq(new Box(2).volume(), 2);
        assert_eq(new Box(2, 3).volume(), 6);
        assert_eq(new Box(2, 3, 4).volume(), 24);
    });
});
