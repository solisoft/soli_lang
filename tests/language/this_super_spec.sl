// ============================================================================
// This and Super Keyword Test Suite
// ============================================================================

describe("This Keyword", fn() {
    test("this in instance method", fn() {
        class Counter {
            value: Int = 0;

            fn get_value() {
                return this.value;
            }

            fn increment() {
                this.value = this.value + 1;
            }
        }

        let c = new Counter();
        assert_eq(c.get_value(), 0);
        c.increment();
        assert_eq(c.get_value(), 1);
    });

    test("this in constructor", fn() {
        class Point {
            x: Int;
            y: Int;

            new(x: Int, y: Int) {
                this.x = x;
                this.y = y;
            }
        }

        let p = new Point(3, 4);
        assert_eq(p.x, 3);
        assert_eq(p.y, 4);
    });

    test("this in nested function", fn() {
        class Container {
            value: Int = 42;

            fn get_value() {
                let helper = fn() {
                    return this.value;
                };
                return helper();
            }
        }

        let c = new Container();
        assert_eq(c.get_value(), 42);
    });

    test("this chaining", fn() {
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
        }

        let c = new Chainer();
        c.add(5).multiply(2);
        assert_eq(c.value, 10);
    });

    test("this with different method calls", fn() {
        class Calculator {
            value: Int = 0;

            fn add(n: Int) {
                this.value = this.value + n;
            }

            fn get_value() {
                return this.value;
            }
        }

        let calc = new Calculator();
        calc.add(10);
        calc.add(5);
        assert_eq(calc.get_value(), 15);
    });
});

describe("Super Keyword", fn() {
    test("super in method call", fn() {
        class Animal {
            fn speak() {
                return "sound";
            }
        }

        class Dog extends Animal {
            fn speak() {
                return super.speak() + " bark";
            }
        }

        let d = new Dog();
        assert_eq(d.speak(), "sound bark");
    });

    test("super in constructor", fn() {
        class Base {
            value: Int;

            new(v) {
                this.value = v;
            }
        }

        class Derived extends Base {
            extra: Int;

            new(a, b) {
                this.value = a;
                this.extra = b;
            }
        }

        let d = new Derived(10, 20);
        assert_eq(d.value, 10);
        assert_eq(d.extra, 20);
    });

    test("super accessing parent method", fn() {
        class Adder {
            fn add(a: Int, b: Int) -> Int {
                return a + b;
            }
        }

        class Multiplier extends Adder {
            fn multiply(a: Int, b: Int) -> Int {
                return super.add(a, b) * 2;
            }
        }

        let m = new Multiplier();
        assert_eq(m.multiply(3, 4), 14);
    });

    test("super in multiple levels of inheritance", fn() {
        class Level1 {
            fn get_name() {
                return "Level1";
            }
        }

        class Level2 extends Level1 {
            fn get_name() {
                return super.get_name() + " -> Level2";
            }
        }

        class Level3 extends Level2 {
            fn get_name() {
                return super.get_name() + " -> Level3";
            }
        }

        let l3 = new Level3();
        assert_eq(l3.get_name(), "Level1 -> Level2 -> Level3");
    });

    test("super with field access", fn() {
        class Base {
            value: Int = 100;
        }

        class Derived extends Base {
            fn get_base_value() {
                return super.value;
            }
        }

        let d = new Derived();
        assert_eq(d.get_base_value(), 100);
    });
});

describe("This and Super Combined", fn() {
    test("this and super in same class", fn() {
        class Parent {
            fn greet() {
                return "Hello";
            }
        }

        class Child extends Parent {
            fn greet() {
                return super.greet() + ", Child!";
            }

            fn greet_verbose() {
                return this.greet();
            }
        }

        let c = new Child();
        assert_eq(c.greet_verbose(), "Hello, Child!");
    });
});
