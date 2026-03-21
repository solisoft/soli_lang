// Tests for S3 built-in class

describe("S3", fn() {
    test("S3 class exists", fn() {
        assert_eq(type(S3), "Class")
    });

    test("S3.list_buckets is a function", fn() {
        assert_eq(type(S3.list_buckets), "Function")
    });

    test("S3.create_bucket is a function", fn() {
        assert_eq(type(S3.create_bucket), "Function")
    });

    test("S3.delete_bucket is a function", fn() {
        assert_eq(type(S3.delete_bucket), "Function")
    });

    test("S3.put_object is a function", fn() {
        assert_eq(type(S3.put_object), "Function")
    });

    test("S3.get_object is a function", fn() {
        assert_eq(type(S3.get_object), "Function")
    });

    test("S3.delete_object is a function", fn() {
        assert_eq(type(S3.delete_object), "Function")
    });

    test("S3.list_objects is a function", fn() {
        assert_eq(type(S3.list_objects), "Function")
    });

    test("S3.copy_object is a function", fn() {
        assert_eq(type(S3.copy_object), "Function")
    });

    test("S3.list_buckets fails without credentials", fn() {
        let did_fail = false
        try {
            S3.list_buckets()
        } catch e {
            did_fail = true
        }
        assert(did_fail)
    });
});
