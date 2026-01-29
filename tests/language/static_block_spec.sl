// ============================================================================
// Static Block Test Suite
// ============================================================================

describe("Static Block", fn() {
    test("static block executes when class is defined", fn() {
        class TestClass {
            static {
                TestClass.executed = true;
            }
        }
        assert_eq(TestClass.executed, true);
    });

    test("static block can define static fields via assignment", fn() {
        class Config {
            static {
                Config.timeout = 30;
                Config.max_retries = 3;
            }
        }
        assert_eq(Config.timeout, 30);
        assert_eq(Config.max_retries, 3);
    });

    test("static block has access to 'this' referencing the class", fn() {
        class MyClass {
            static {
                this.counter = 0;
            }
        }
        assert_eq(MyClass.counter, 0);
    });

    test("multiple statements in static block", fn() {
        class Processor {
            static {
                Processor.initialized = true;
                Processor.start_time = 100;
                Processor.end_time = 200;
            }
        }
        assert_eq(Processor.initialized, true);
        assert_eq(Processor.start_time, 100);
        assert_eq(Processor.end_time, 200);
    });

    test("static block can call functions", fn() {
        fn get_value() {
            return 42;
        }
        class MathHelper {
            static {
                MathHelper.result = get_value();
            }
        }
        assert_eq(MathHelper.result, 42);
    });

    test("static block with control flow", fn() {
        class Config {
            static {
                if (true) {
                    Config.value = "yes";
                } else {
                    Config.value = "no";
                }
            }
        }
        assert_eq(Config.value, "yes");
    });

    test("static block with loops", fn() {
        class Counter {
            static {
                Counter.sum = 0;
                for (i in [1, 2, 3, 4, 5]) {
                    Counter.sum = Counter.sum + i;
                }
            }
        }
        assert_eq(Counter.sum, 15);
    });
});
