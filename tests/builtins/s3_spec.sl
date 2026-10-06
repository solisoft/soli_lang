# S3: everything that is decided before a request leaves — argument checks,
# the bucket-name guard on copy_object (SEC-021), and the credential error.
# Nothing here talks to S3.

describe("S3") do
  describe("argument validation") do
    test("create_bucket and delete_bucket want a string bucket name") do
      assert_raises("S3.create_bucket() expects string bucket name, got int") do
        S3.create_bucket(5)
      end
      assert_raises("S3.delete_bucket() expects string bucket name, got null") do
        S3.delete_bucket(nil)
      end
    end

    test("fixed-arity methods refuse a wrong argument count") do
      assert_raises("expected 1, got 0") do
        S3.create_bucket()
      end
      assert_raises("expected 2, got 1") do
        S3.get_object("bucket")
      end
    end

    test("put_object takes three or four arguments") do
      assert_raises("S3.put_object() expects 3-4 arguments (bucket, key, body, options?), got 2") do
        S3.put_object("bucket", "key")
      end
      assert_raises("got 5") do
        S3.put_object("bucket", "key", "body", {}, "extra")
      end
    end

    test("put_object wants string arguments") do
      assert_raises("S3.put_object() expects string key, got int") do
        S3.put_object("bucket", 1, "body")
      end
    end

    test("delete_object wants a string key") do
      assert_raises("S3.delete_object() expects string key, got int") do
        S3.delete_object("bucket", 3)
      end
    end

    test("list_objects takes a bucket and an optional prefix") do
      assert_raises("S3.list_objects() expects 1-2 arguments (bucket, prefix?), got 0") do
        S3.list_objects()
      end
      assert_raises("got 3") do
        S3.list_objects("bucket", "prefix", "extra")
      end
      assert_raises("S3.list_objects() expects string prefix, got int") do
        S3.list_objects("bucket", 7)
      end
    end
  end

  describe("copy_object paths") do
    test("source and dest must be bucket/key") do
      assert_raises("Source must be in format 'bucket/key'") do
        S3.copy_object("nokey", "bucket/key")
      end
      assert_raises("Dest must be in format 'bucket/key'") do
        S3.copy_object("bucket/key", "nokey")
      end
    end

    test("a query string smuggled into the source bucket is refused") do
      message = assert_raises() do
        S3.copy_object("my-bucket?versionId=evil/foo", "bucket/key")
      end
      assert_contains(message, "Invalid bucket name 'my-bucket?versionId=evil'")
      assert_contains(message, "only lowercase letters, digits, '.' and '-' are allowed")
    end

    test("bucket names are 3 to 63 characters") do
      assert_raises("Invalid bucket name 'ab': length must be 3-63 characters") do
        S3.copy_object("ab/key", "bucket/key")
      end
      long_name = "b" * 64
      assert_raises("length must be 3-63 characters") do
        S3.copy_object("bucket/key", "#{long_name}/key")
      end
    end

    test("uppercase bucket names are refused, on either side") do
      assert_raises("Invalid bucket name 'Bucket'") do
        S3.copy_object("Bucket/key", "bucket/key")
      end
      assert_raises("Invalid bucket name 'Dest'") do
        S3.copy_object("bucket/key", "Dest/key")
      end
    end
  end

  describe("without credentials") do
    before_each() do
      skip("S3 credentials are set in this environment") if hasenv("AWS_ACCESS_KEY_ID") || hasenv("S3_ACCESS_KEY")
    end

    test("every call that reaches the client names the missing variables") do
      assert_raises("S3_ACCESS_KEY or AWS_ACCESS_KEY_ID not set") do
        S3.list_buckets()
      end
      assert_raises("S3_ACCESS_KEY or AWS_ACCESS_KEY_ID not set") do
        S3.put_object("bucket", "key", "body")
      end
    end

    test("a valid copy gets past the bucket guard to the credential check") do
      assert_raises("S3_ACCESS_KEY or AWS_ACCESS_KEY_ID not set") do
        S3.copy_object("source.bucket/a/b.txt", "dest-bucket/c.txt")
      end
    end
  end
end
