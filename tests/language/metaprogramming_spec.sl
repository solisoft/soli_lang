// ============================================================================
// Metaprogramming Features Test Suite
// ============================================================================

class Foo {
    fn greet(name) {
        return "Hello, " + name + "!";
    }

    fn method_missing(name) {
        return "Method '" + name + "' was called";
    }
}

describe("respond_to?", fn() {
    test("responds to defined method", fn() {
        let foo = Foo.new();
        assert(foo.respond_to?("greet"));
    });

    test("does not respond to undefined method", fn() {
        let foo = Foo.new();
        assert_not(foo.respond_to?("nonexistent"));
    });

    test("responds to built-in methods", fn() {
        let foo = Foo.new();
        assert(foo.respond_to?("inspect"));
        assert(foo.respond_to?("class"));
    });
});

describe("send", fn() {
    test("calls method by name", fn() {
        let foo = Foo.new();
        let result = foo.send("greet", "World");
        assert_eq(result, "Hello, World!");
    });

    test("calls method_missing via send", fn() {
        let foo = Foo.new();
        let result = foo.send("foobar");
        assert_eq(result, "Method 'foobar' was called");
    });
});

describe("instance_variables", fn() {
    test("lists instance variables", fn() {
        let foo = Foo.new();
        foo._name = "test";
        foo._count = 42;
        let vars = foo.instance_variables;
        assert(vars.includes?("@_name"));
        assert(vars.includes?("@_count"));
    });

    test("returns empty array when no instance variables", fn() {
        let foo = Foo.new();
        let vars = foo.instance_variables;
        assert_eq(vars.length, 0);
    });
});

describe("instance_variable_get", fn() {
    test("gets existing instance variable", fn() {
        let foo = Foo.new();
        foo._name = "test_value";
        let value = foo.instance_variable_get("@_name");
        assert_eq(value, "test_value");
    });

    test("returns null for nonexistent variable", fn() {
        let foo = Foo.new();
        let value = foo.instance_variable_get("@_nonexistent");
        assert_eq(value, null);
    });

    test("works without @ prefix", fn() {
        let foo = Foo.new();
        foo._name = "test_value";
        let value = foo.instance_variable_get("_name");
        assert_eq(value, "test_value");
    });
});

describe("instance_variable_set", fn() {
    test("sets instance variable", fn() {
        let foo = Foo.new();
        let value = foo.instance_variable_set("@_name", "set_value");
        assert_eq(value, "set_value");
        assert_eq(foo.instance_variable_get("@_name"), "set_value");
    });

    test("works without @ prefix", fn() {
        let foo = Foo.new();
        foo.instance_variable_set("_count", 42);
        assert_eq(foo.instance_variable_get("@_count"), 42);
    });
});

describe("methods", fn() {
    test("lists method names", fn() {
        let foo = Foo.new();
        let methods = foo.methods;
        assert(methods.includes?("greet"));
        assert(methods.includes?("respond_to?"));
        assert(methods.includes?("send"));
        assert(methods.includes?("inspect"));
    });
});

describe("method_missing", fn() {
    test("is called for undefined methods", fn() {
        let foo = Foo.new();
        let result = foo.undefined_method();
        assert_eq(result, "Method 'undefined_method' was called");
    });
});

describe("instance_eval", fn() {
    test("executes block with this bound to instance", fn() {
        class Foo { name: String }
        let foo = new Foo()
        foo.name = "Test"
        let result = foo.instance_eval { this.name }
        assert_eq(result, "Test")
    });

    test("can modify instance state", fn() {
        class Counter { count: Int = 0 }
        let c = new Counter()
        c.instance_eval {
            this.count = 42
        }
        assert_eq(c.count, 42)
    });

    test("self is same as this", fn() {
        class Foo { value: Int = 10 }
        let foo = new Foo()
        let result = foo.instance_eval { self.value }
        assert_eq(result, 10)
    });
});

describe("class_eval", fn() {
    test("executes block with self bound to class", fn() {
        class Foo {
            static name: String = "FooClass"
        }
        let result = Foo.class_eval { self.name }
        assert_eq(result, "FooClass")
    });

    test("can access static methods via self", fn() {
        class Foo {
            static value: Int = 42
        }
        let result = Foo.class_eval { self.value }
        assert_eq(result, 42)
    });
});

describe("define_method", fn() {
    test("defines a method on instance's class", fn() {
        let foo = Foo.new();
        foo.define_method("greet2", || { "Hello!" });
        assert(foo.respond_to?("greet2"));
        assert_eq(foo.greet2(), "Hello!");
    });

    test("defined method can take arguments", fn() {
        class Calculator { }
        let calc = Calculator.new();
        calc.define_method("add", |a, b| { a + b });
        assert_eq(calc.add(1, 2), 3);
    });
});

describe("define_method", fn() {
    test("defines a method on instance's class", fn() {
        let foo = Foo.new();
        foo.define_method("greet2", || { "Hello!" });
        assert(foo.respond_to?("greet2"));
        assert_eq(foo.greet2(), "Hello!");
    });

    test("defined method can take arguments", fn() {
        class Calculator { }
        let calc = Calculator.new();
        calc.define_method("add", |a, b| { a + b });
        assert_eq(calc.add(1, 2), 3);
    });
});