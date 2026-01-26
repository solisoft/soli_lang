// ============================================================================
// Interfaces Test Suite
// ============================================================================

describe("Interface Declaration", fn() {
    test("interface with method signature", fn() {
        interface Greetable {
            fn greet();
        }

        class Person implements Greetable {
            fn greet() {
                return "Hello";
            }
        }

        let p = new Person();
        assert_eq(p.greet(), "Hello");
    });

    test("interface with multiple methods", fn() {
        interface CRUD {
            fn create();
            fn read();
            fn update(data);
            fn delete();
        }

        class SimpleStore implements CRUD {
            data: String = "";

            fn create() {
                this.data = "created";
            }

            fn read() {
                return this.data;
            }

            fn update(data) {
                this.data = data;
            }

            fn delete() {
                this.data = "";
            }
        }

        let store = new SimpleStore();
        store.create();
        assert_eq(store.read(), "created");
        store.update("updated");
        assert_eq(store.read(), "updated");
        store.delete();
        assert_eq(store.read(), "");
    });

    test("interface with typed parameters", fn() {
        interface Calculator {
            fn add(a, b);
            fn multiply(a, b);
        }

        class SimpleCalc implements Calculator {
            fn add(a, b) {
                return a + b;
            }

            fn multiply(a, b) {
                return a * b;
            }
        }

        let calc = new SimpleCalc();
        assert_eq(calc.add(3, 4), 7);
        assert_eq(calc.multiply(2, 5), 10);
    });

    test("empty interface", fn() {
        interface Empty {
        }

        class EmptyImpl implements Empty {
        }

        let e = new EmptyImpl();
        assert_not_null(e);
    });
});

describe("Interface Implementation", fn() {
    test("class implements single interface", fn() {
        interface Printable {
            fn print();
        }

        class Document implements Printable {
            content: String;

            new(content: String) {
                this.content = content;
            }

            fn print() {
                return this.content;
            }
        }

        let doc = new Document("Hello World");
        assert_eq(doc.print(), "Hello World");
    });

    test("class implements multiple interfaces", fn() {
        interface Loggable {
            fn log();
        }

        interface Serializable {
            fn serialize();
        }

        class Data implements Loggable, Serializable {
            value: Int;

            new(value: Int) {
                this.value = value;
            }

            fn log() {
                return "Data: " + str(this.value);
            }

            fn serialize() {
                return "{\"value\":" + str(this.value) + "}";
            }
        }

        let d = new Data(42);
        assert_eq(d.log(), "Data: 42");
        assert_eq(d.serialize(), "{\"value\":42}");
    });

    test("class with interface and inheritance", fn() {
        class Base {
            base_value: Int = 10;
        }

        interface Incrementable {
            fn increment();
        }

        class Derived extends Base implements Incrementable {
            fn increment() {
                this.base_value = this.base_value + 1;
                return this.base_value;
            }
        }

        let d = new Derived();
        assert_eq(d.base_value, 10);
        assert_eq(d.increment(), 11);
        assert_eq(d.increment(), 12);
    });

    test("implementation order does not matter", fn() {
        interface A {
            fn method_a();
        }

        interface B {
            fn method_b();
        }

        class AB implements A, B {
            fn method_a() { return "A"; }
            fn method_b() { return "B"; }
        }

        let ab = new AB();
        assert_eq(ab.method_a(), "A");
        assert_eq(ab.method_b(), "B");
    });
});
