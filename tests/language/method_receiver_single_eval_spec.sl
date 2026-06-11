// ============================================================================
// Method-call receiver is evaluated exactly once
//
// Regression for the old fast-path design where the hash/string/model call
// interceptors each evaluated the receiver expression and returned "not
// mine" on a type mismatch — so a side-effectful receiver like
// `make().map(f)` ran `make()` twice. The unified dispatcher evaluates the
// receiver once and dispatches on the value.
// ============================================================================

let eval_count = 0;

fn make_array() {
    eval_count = eval_count + 1;
    return [1, 2, 3];
}

fn make_hash() {
    eval_count = eval_count + 1;
    return {"a": 1, "b": 2};
}

fn make_string() {
    eval_count = eval_count + 1;
    return "hello";
}

class Greeter {
    fn greet(name) { return "hi " + name; }
}

fn make_instance() {
    eval_count = eval_count + 1;
    return new Greeter();
}

describe("Method receiver single evaluation", fn() {
    before_each(fn() {
        eval_count = 0;
    });

    test("array closure method evaluates receiver once", fn() {
        let r = make_array().map(fn(x) x * 2);
        assert_eq(eval_count, 1);
        assert_eq(r, [2, 4, 6]);
    });

    test("array pure method evaluates receiver once", fn() {
        let r = make_array().sum();
        assert_eq(eval_count, 1);
        assert_eq(r, 6);
    });

    test("array mutating method evaluates receiver once", fn() {
        make_array().push(4);
        assert_eq(eval_count, 1);
    });

    test("hash method evaluates receiver once", fn() {
        let keys = make_hash().keys();
        assert_eq(eval_count, 1);
        assert_eq(len(keys), 2);
    });

    test("hash get with literal key evaluates receiver once", fn() {
        let v = make_hash().get("a");
        assert_eq(eval_count, 1);
        assert_eq(v, 1);
    });

    test("hash delete evaluates receiver once", fn() {
        // "delete" is also a model-interceptor name; the interceptor must
        // not re-evaluate non-model receivers.
        make_hash().delete("a");
        assert_eq(eval_count, 1);
    });

    test("string method evaluates receiver once", fn() {
        let r = make_string().upcase();
        assert_eq(eval_count, 1);
        assert_eq(r, "HELLO");
    });

    test("instance method evaluates receiver once", fn() {
        let r = make_instance().greet("bob");
        assert_eq(eval_count, 1);
        assert_eq(r, "hi bob");
    });

    test("sort comparator may mutate the receiver without panicking", fn() {
        // `sort` runs a user comparator, so it must iterate over a snapshot:
        // a comparator that mutates the receiver used to panic with a
        // RefCell double-borrow because `sort` was missing from the
        // closure-takes-user-code list and ran on a live borrow.
        let a = [3, 1, 2];
        let sorted = a.sort(fn(x, y) {
            a.push(99);
            return x - y;
        });
        assert_eq(sorted, [1, 2, 3]);
    });

    test("instance save-named method evaluates receiver once", fn() {
        // "save" is a model persist-interceptor name; a plain class with a
        // user-defined save must not be evaluated twice (or intercepted).
        class Doc {
            new() { this.saved = false; }
            fn save() { this.saved = true; return "saved"; }
        }
        fn make_doc() {
            eval_count = eval_count + 1;
            return new Doc();
        }
        let r = make_doc().save();
        assert_eq(eval_count, 1);
        assert_eq(r, "saved");
    });
});
