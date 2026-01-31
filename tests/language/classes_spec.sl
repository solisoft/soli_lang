// ============================================================================
// Classes Test Suite
// ============================================================================

describe("Basic Classes", fn() {
    test("class declaration and instantiation", fn() {
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

    test("class with methods", fn() {
        class Rectangle {
            width: Int;
            height: Int;

            new(w: Int, h: Int) {
                this.width = w;
                this.height = h;
            }

            fn area() {
                return this.width * this.height;
            }

            fn perimeter() {
                return 2 * (this.width + this.height);
            }
        }
        let rect = new Rectangle(5, 3);
        assert_eq(rect.area(), 15);
        assert_eq(rect.perimeter(), 16);
    });

    test("class with default field values", fn() {
        class Counter {
            count: Int = 0;

            fn increment() {
                this.count = this.count + 1;
            }

            fn get() {
                return this.count;
            }
        }
        let c = new Counter();
        assert_eq(c.get(), 0);
        c.increment();
        c.increment();
        assert_eq(c.get(), 2);
    });

    test("class method chaining", fn() {
        class Builder {
            value: String = "";

            fn add(s: String) {
                this.value = this.value + s;
                return this;
            }

            fn build() {
                return this.value;
            }
        }
        let result = new Builder().add("Hello").add(" ").add("World").build();
        assert_eq(result, "Hello World");
    });
});

describe("Static Methods", fn() {
    test("static method can be called without instance", fn() {
        class MathUtils {
            static fn random() {
                return 42;
            }
        }
        assert_eq(MathUtils.random(), 42);
    });

    test("static method with parameters", fn() {
        class MathUtils {
            static fn add(a: Int, b: Int) -> Int {
                return a + b;
            }
        }
        assert_eq(MathUtils.add(5, 3), 8);
    });

    test("static method with return type", fn() {
        class Calculator {
            static fn multiply(a: Float, b: Float) -> Float {
                return a * b;
            }
        }
        assert_eq(Calculator.multiply(3.0, 4.0), 12.0);
    });

    test("static method inheritance", fn() {
        class Base {
            static fn greet() -> String {
                return "Hello";
            }
        }

        class Derived extends Base {
        }

        assert_eq(Derived.greet(), "Hello");
    });

    test("static method override", fn() {
        class Base {
            static fn get_value() -> Int {
                return 10;
            }
        }

        class Derived extends Base {
            static fn get_value() -> Int {
                return 20;
            }
        }

        assert_eq(Derived.get_value(), 20);
    });

    test("static method can call other static methods", fn() {
        class MathOps {
            static fn double(x: Int) -> Int {
                return x * 2;
            }

            static fn quadruple(x: Int) -> Int {
                return MathOps.double(MathOps.double(x));
            }
        }
        assert_eq(MathOps.quadruple(5), 20);
    });

    test("static factory method pattern", fn() {
        class Point {
            x: Float;
            y: Float;

            new(x: Float, y: Float) {
                this.x = x;
                this.y = y;
            }

            static fn origin() -> Point {
                return new Point(0.0, 0.0);
            }

            static fn unit() -> Point {
                return new Point(1.0, 1.0);
            }
        }

        let origin = Point.origin();
        assert_eq(origin.x, 0.0);
        assert_eq(origin.y, 0.0);

        let unit = Point.unit();
        assert_eq(unit.x, 1.0);
        assert_eq(unit.y, 1.0);
    });
});

describe("Static Fields", fn() {
    test("static field declaration and access", fn() {
        class Counter {
            static count: Int = 0;
        }
        assert_eq(Counter.count, 0);
    });

    test("static field initial value", fn() {
        class Config {
            static debug: Bool = false;
            static version: String = "1.0.0";
        }
        assert_eq(Config.debug, false);
        assert_eq(Config.version, "1.0.0");
    });

    test("static field mutation via Class.field = value", fn() {
        class Config {
            static debug: Bool = false;
        }
        Config.debug = true;
        assert_eq(Config.debug, true);
    });

    test("static field in subclass shares with parent", fn() {
        class Base {
            static instance_count: Int = 0;
        }

        class Derived extends Base {
        }

        assert_eq(Derived.instance_count, 0);
        Base.instance_count = 5;
        assert_eq(Derived.instance_count, 5);
    });

    test("static field for class-level state", fn() {
        class IdGenerator {
            static next_id: Int = 1;

            static fn generate() -> Int {
                let id = IdGenerator.next_id;
                IdGenerator.next_id = IdGenerator.next_id + 1;
                return id;
            }
        }

        assert_eq(IdGenerator.generate(), 1);
        assert_eq(IdGenerator.generate(), 2);
        assert_eq(IdGenerator.generate(), 3);
    });

    test("static field with type annotation", fn() {
        class Constants {
            static pi: Float = 3.14159;
            static max_size: Int = 1000;
        }
        assert_eq(Constants.pi, 3.14159);
        assert_eq(Constants.max_size, 1000);
    });
});

describe("Private and Protected Visibility", fn() {
    test("private keyword is parsed on field", fn() {
        class Secret {
            private password: String = "secret";
            fn get_password() {
                return this.password;
            }
        }
        let s = new Secret();
        assert_eq(s.get_password(), "secret");
        assert_eq(s.password, "secret");
    });

    test("private keyword is parsed on method", fn() {
        class Container {
            private fn compute() -> Int {
                return 42;
            }
            fn get_value() {
                return this.compute();
            }
        }
        let c = new Container();
        assert_eq(c.get_value(), 42);
        let result = c.compute();
        assert_eq(result, 42);
    });

    test("protected keyword is parsed on field", fn() {
        class Base {
            protected value: Int = 10;
        }
        let b = new Base();
        assert_eq(b.value, 10);
    });

    test("protected keyword is parsed on method", fn() {
        class Base {
            protected fn internal_method() -> String {
                return "internal";
            }
        }
        let b = new Base();
        let result = b.internal_method();
        assert_eq(result, "internal");
    });

    test("private field with default value", fn() {
        class SafeBox {
            private code: String = "1234";
            private attempts: Int = 0;

            fn try_code(input: String) -> Bool {
                this.attempts = this.attempts + 1;
                return this.code == input;
            }

            fn get_attempts() -> Int {
                return this.attempts;
            }
        }
        let box = new SafeBox();
        assert_eq(box.try_code("0000"), false);
        assert_eq(box.try_code("1234"), true);
        assert_eq(box.get_attempts(), 2);
    });

    test("class with multiple visibility modifiers", fn() {
        class User {
            private id: Int;
            protected email: String;
            name: String;

            new(id: Int, email: String, name: String) {
                this.id = id;
                this.email = email;
                this.name = name;
            }

            fn get_id() -> Int {
                return this.id;
            }
        }
        let u = new User(1, "test@example.com", "Alice");
        assert_eq(u.get_id(), 1);
        assert_eq(u.email, "test@example.com");
        assert_eq(u.name, "Alice");
    });

    test("static private field", fn() {
        class SecureCounter {
            private static counter: Int = 0;

            static fn increment() {
                SecureCounter.counter = SecureCounter.counter + 1;
            }

            static fn get_count() -> Int {
                return SecureCounter.counter;
            }
        }
        assert_eq(SecureCounter.get_count(), 0);
        SecureCounter.increment();
        SecureCounter.increment();
        assert_eq(SecureCounter.get_count(), 2);
    });
});

describe("Native Static Methods", fn() {
    test("DateTime.now() returns current datetime", fn() {
        let now = DateTime.now();
        assert_not_null(now);
        assert_eq(type(now), "DateTime");
    });

    test("DateTime.parse() parses ISO string", fn() {
        let dt = DateTime.parse("2024-01-15T10:30:00Z");
        assert_not_null(dt);
        assert_eq(type(dt), "DateTime");
    });

    test("Duration.of_seconds() creates duration", fn() {
        let dur = Duration.of_seconds(120);
        assert_not_null(dur);
        assert_eq(type(dur), "Duration");
    });

    test("Duration.of_minutes() creates duration", fn() {
        let dur = Duration.of_minutes(5);
        assert_not_null(dur);
    });

    test("Duration.of_hours() creates duration", fn() {
        let dur = Duration.of_hours(2);
        assert_not_null(dur);
    });

    test("Duration.between() calculates difference", fn() {
        let dt1 = DateTime.parse("2024-01-15T10:00:00Z");
        let dt2 = DateTime.parse("2024-01-15T11:30:00Z");
        let dur = Duration.between(dt1, dt2);
        assert_not_null(dur);
    });
});

describe("Constructor Named Parameters", fn() {
    test("constructor with all named parameters", fn() {
        class User {
            name: String;
            age: Int;
            active: Bool;

            new(name: String = "Guest", age: Int = 0, active: Bool = true) {
                this.name = name;
                this.age = age;
                this.active = active;
            }
        }
        let user = new User(name: "Alice", age: 30, active: false);
        assert_eq(user.name, "Alice");
        assert_eq(user.age, 30);
        assert_eq(user.active, false);
    });

    test("constructor with mixed positional and named parameters", fn() {
        class Config {
            host: String;
            port: Int;
            ssl: Bool;
            debug: Bool;

            new(host: String, port: Int = 80, ssl: Bool = false, debug: Bool = false) {
                this.host = host;
                this.port = port;
                this.ssl = ssl;
                this.debug = debug;
            }
        }
        let config = new Config("example.com", ssl: true);
        assert_eq(config.host, "example.com");
        assert_eq(config.port, 80);
        assert_eq(config.ssl, true);
        assert_eq(config.debug, false);
    });

    test("constructor with some named parameters", fn() {
        class Server {
            name: String;
            port: Int;
            workers: Int;

            new(name: String, port: Int = 8080, workers: Int = 4) {
                this.name = name;
                this.port = port;
                this.workers = workers;
            }
        }
        let server = new Server("api-server", workers: 8);
        assert_eq(server.name, "api-server");
        assert_eq(server.port, 8080);
        assert_eq(server.workers, 8);
    });

    test("constructor duplicate named parameter throws error", fn() {
        class Point {
            x: Int;
            y: Int;

            new(x: Int = 0, y: Int = 0) {
                this.x = x;
                this.y = y;
            }
        }
        let error_thrown = false;
        try {
            new Point(x: 5, x: 10);
        } catch (error) {
            error_thrown = true;
        }
        assert_eq(error_thrown, true);
    });

    test("constructor unknown parameter name throws error", fn() {
        class Circle {
            radius: Int;

            new(radius: Int = 1) {
                this.radius = radius;
            }
        }
        let error_thrown = false;
        try {
            new Circle(diameter: 10);
        } catch (error) {
            error_thrown = true;
        }
        assert_eq(error_thrown, true);
    });
});

describe("Nested Classes", fn() {
    test("basic nested class declaration", fn() {
        class Outer {
            class Inner {
                fn greet() {
                    return "Hello from Inner";
                }
            }
        }
        let inner = new Outer::Inner();
        assert_eq(inner.greet(), "Hello from Inner");
    });

    test("nested class with constructor", fn() {
        class Container {
            class Item {
                fn create_value() {
                    return 42;
                }
            }
        }
        let item = new Container::Item();
        assert_eq(item.create_value(), 42);
    });

    test("multiple nested classes", fn() {
        class Service {
            class Database {
                fn connect() {
                    return "DB connected";
                }
            }
            
            class Cache {
                fn get(key) {
                    return "cached:" + key;
                }
            }
        }
        let db = new Service::Database();
        let cache = new Service::Cache();
        assert_eq(db.connect(), "DB connected");
        assert_eq(cache.get("test"), "cached:test");
    });

    test("nested class accessing parent class", fn() {
        class Parent {
            fn get_name() {
                return "Parent";
            }
            
            class Child {
                fn introduce() {
                    return "Child of Parent";
                }
            }
        }
        let child = new Parent::Child();
        assert_eq(child.introduce(), "Child of Parent");
    });
});
