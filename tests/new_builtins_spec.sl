# Url, Logger, Retry, CircuitBreaker, Toml/Yaml, Semaphore and Money — one
# describe per class so a failure names its surface.

describe("Url") do
  describe("parse") do
    test("splits every component") do
      url = Url.parse("https://user:pw@api.ex.com:8443/v1/items?page=2#top")
      assert_eq(url, {
        "scheme": "https",
        "username": "user",
        "password": "pw",
        "host": "api.ex.com",
        "port": 8443,
        "path": "/v1/items",
        "query": "page=2",
        "fragment": "top"
      })
    end

    test("absent parts are nil and the path defaults to /") do
      bare = Url.parse("https://ex.com")
      assert_null(bare["port"])
      assert_null(bare["query"])
      assert_null(bare["fragment"])
      assert_null(bare["username"])
      assert_eq(bare["path"], "/")
    end

    test("a relative URL raises") do
      assert_raises("relative URL without a base") do
        Url.parse("not a url")
      end
    end
  end

  describe("query params") do
    test("params decodes values and keeps a bare key as nil") do
      params = Url.params("https://ex.com/?q=red%20shoe&flag&n=7")
      assert_eq(params["q"], "red shoe")
      assert_null(params["flag"])
      assert_eq(params["n"], "7")
    end

    test("params of a URL with no query is empty") do
      assert_eq(Url.params("https://ex.com/"), {})
    end

    test("param reads one key, nil when missing") do
      assert_eq(Url.param("https://ex.com/?page=2", "page"), "2")
      assert_null(Url.param("https://ex.com/?page=2", "nope"))
    end

    test("set_param adds, encodes, replaces and removes") do
      assert_eq(Url.set_param("https://ex.com/?b=2", "a", "1"), "https://ex.com/?b=2&a=1")
      assert_eq(Url.set_param("https://ex.com/", "q", "a b"), "https://ex.com/?q=a%20b")
      assert_eq(Url.set_param("https://ex.com/?a=1&b=2", "a", nil), "https://ex.com/?b=2")
    end
  end

  describe("join and build") do
    test("join resolves relative, query-only, absolute-path and absolute references") do
      assert_eq(Url.join("https://ex.com/a/b", "c"), "https://ex.com/a/c")
      assert_eq(Url.join("https://ex.com/a/b", "?page=2"), "https://ex.com/a/b?page=2")
      assert_eq(Url.join("https://ex.com/a/b", "/root"), "https://ex.com/root")
      assert_eq(Url.join("https://ex.com/a/b", "https://other.com/x"), "https://other.com/x")
    end

    test("build assembles parts and encodes the query hash") do
      parts = {"scheme": "https", "host": "api.ex.com", "path": "/v1/x", "query": {"page": 2, "q": "a b"}}
      assert_eq(Url.build(parts), "https://api.ex.com/v1/x?page=2&q=a%20b")
    end
  end

  describe("component encoding") do
    test("encodes and decodes a space") do
      assert_eq(Url.encode_component("Ann Lee"), "Ann%20Lee")
      assert_eq(Url.decode_component("Ann%20Lee"), "Ann Lee")
    end

    test("encodes reserved characters and UTF-8, and decodes them back") do
      encoded = Url.encode_component("a&b=c/d?é")
      assert_eq(encoded, "a%26b%3Dc%2Fd%3F%C3%A9")
      assert_eq(Url.decode_component(encoded), "a&b=c/d?é")
    end
  end
end

describe("Logger") do
  after_each() do
    Logger.set_capture(false)
    Logger.configure({"level": "info"})
  end

  test("captures an entry with its level, message and fields") do
    Logger.configure({"level": "info"})
    Logger.set_capture(true)
    Logger.info("hello", {"k": 1})
    # Read while capture is still on — disabling clears the buffer.
    entries = Logger.entries()
    assert_eq(entries.length, 1)
    assert_match(entries[0], r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ \[INFO\] hello k=1$")
  end

  test("entries below the configured level are not captured") do
    Logger.configure({"level": "info"})
    Logger.set_capture(true)
    Logger.debug("hidden")
    assert_eq(Logger.entries(), [])
  end

  test("turning capture off clears the buffer") do
    Logger.set_capture(true)
    Logger.info("gone")
    Logger.set_capture(false)
    assert_eq(Logger.entries(), [])
  end

  test("level is configurable and reported upper-case") do
    Logger.configure({"level": "error"})
    assert_eq(Logger.level(), "ERROR")
    Logger.configure({"level": "info"})
    assert_eq(Logger.level(), "INFO")
  end
end

describe("Retry") do
  test("returns the block result on first success") do
    assert_eq(Retry.with_backoff(fn() { "ok" }), "ok")
  end

  # The blocks are passed inline: a zero-parameter lambda held in a variable
  # auto-invokes on bare access, so `Retry.with_backoff(flaky)` would run it
  # once, outside the retry loop.
  test("retries until the block succeeds") do
    attempts = {"n": 0}
    result = Retry.with_backoff(fn() {
      attempts["n"] = attempts["n"] + 1
      throw "flaky" if attempts["n"] < 3
      "recovered on #{attempts["n"]}"
    }, {"attempts": 5, "base_delay": 0.01})
    assert_eq(result, "recovered on 3")
  end

  test("re-raises the last error once the attempts run out, after trying each") do
    attempts = {"n": 0}
    message = assert_raises() do
      Retry.with_backoff(fn() {
        attempts["n"] = attempts["n"] + 1
        throw "always"
      }, {"attempts": 2, "base_delay": 0.01})
    end
    assert_contains(message, "always")
    assert_eq(attempts["n"], 2)
  end

  test("within succeeds immediately without waiting for the deadline") do
    started = DateTime.microtime
    assert_eq(Retry.within(fn() { "ready" }, {"deadline": 10}), "ready")
    assert_lt(DateTime.microtime - started, 5000000.0)
  end

  test("within re-raises when the deadline passes") do
    assert_raises("never") do
      Retry.within(fn() { throw "never" }, {"deadline": 0.05})
    end
  end
end

describe("CircuitBreaker") do
  after_each() do
    CircuitBreaker.reset("spec-cb")
  end

  test("transitions closed -> open -> half_open -> closed") do
    CircuitBreaker.configure("spec-cb", {"threshold": 2, "reset_after": 0.05})
    assert(CircuitBreaker.allow("spec-cb"))
    CircuitBreaker.record_failure("spec-cb")
    assert_eq(CircuitBreaker.state("spec-cb"), "closed")
    CircuitBreaker.record_failure("spec-cb")
    assert_eq(CircuitBreaker.state("spec-cb"), "open")
    assert_not(CircuitBreaker.allow("spec-cb"))

    sleep(0.06)
    assert_eq(CircuitBreaker.state("spec-cb"), "half_open")
    CircuitBreaker.record_success("spec-cb")
    assert_eq(CircuitBreaker.state("spec-cb"), "closed")
  end

  test("a success resets the failure count") do
    CircuitBreaker.configure("spec-cb", {"threshold": 2})
    CircuitBreaker.record_failure("spec-cb")
    CircuitBreaker.record_success("spec-cb")
    CircuitBreaker.record_failure("spec-cb")
    assert_eq(CircuitBreaker.state("spec-cb"), "closed")
  end

  test("reset closes an open breaker") do
    CircuitBreaker.configure("spec-cb", {"threshold": 1, "reset_after": 60})
    CircuitBreaker.record_failure("spec-cb")
    assert_eq(CircuitBreaker.state("spec-cb"), "open")
    CircuitBreaker.reset("spec-cb")
    assert_eq(CircuitBreaker.state("spec-cb"), "closed")
    assert(CircuitBreaker.allow("spec-cb"))
  end

  test("a breaker never configured reads as closed") do
    assert_eq(CircuitBreaker.state("spec-cb-never-configured"), "closed")
  end
end

describe("Toml") do
  test("parses nested tables and arrays") do
    config = Toml.parse("title = \"demo\"\n\n[owner]\nname = \"Ann\"\nports = [1, 2]\n")
    assert_eq(config, {"title": "demo", "owner": {"name": "Ann", "ports": [1, 2]}})
  end

  test("a parse error names the line and column") do
    assert_raises("TOML parse error at line 1, column 6") do
      Toml.parse("this is [ not toml")
    end
  end

  test("stringify writes top-level keys, then a table per nested hash") do
    text = Toml.stringify({"title": "demo", "owner": {"name": "Ann"}})
    assert_eq(text, "title = \"demo\"\n\n[owner]\nname = \"Ann\"\n")
  end

  test("stringify round-trips through parse") do
    document = {"a": 1, "b": {"c": true, "ports": [1, 2]}}
    assert_eq(Toml.parse(Toml.stringify(document)), document)
  end

  test("stringify refuses a non-hash") do
    assert_raises("Toml.stringify() expects a hash, got array") do
      Toml.stringify([1, 2])
    end
  end

  test("stringify writes an empty hash as an empty document") do
    assert_eq(Toml.stringify({}), "")
  end

  test("stringify refuses a nil value, which TOML cannot express") do
    assert_raises("Toml.stringify(): unsupported unit type") do
      Toml.stringify({"a": nil})
    end
  end
end

describe("Yaml") do
  test("parses arrays and nesting") do
    document = Yaml.parse("name: demo\ncounts:\n  - 1\n  - 2\n")
    assert_eq(document, {"name": "demo", "counts": [1, 2]})
  end

  test("stringify writes one key per line") do
    assert_eq(Yaml.stringify({"name": "x"}), "name: x\n")
  end

  test("stringify round-trips through parse") do
    document = {"name": "x", "list": [1, 2], "nested": {"ok": true}}
    assert_eq(Yaml.parse(Yaml.stringify(document)), document)
  end

  test("a parse error says what was expected") do
    assert_raises("did not find expected ',' or ']'") do
      Yaml.parse("a: [unclosed")
    end
  end
end

describe("Semaphore") do
  test("acquires up to the limit, refuses, then admits after a release") do
    first = Semaphore.try_acquire("spec-sem", 2)
    second = Semaphore.try_acquire("spec-sem", 2)
    assert_not_null(first)
    assert_not_null(second)
    assert_ne(first, second)
    assert_null(Semaphore.try_acquire("spec-sem", 2))

    assert(Semaphore.release("spec-sem", first))
    third = Semaphore.try_acquire("spec-sem", 2)
    assert_not_null(third)

    Semaphore.release("spec-sem", second)
    Semaphore.release("spec-sem", third)
  end

  test("a second release of the same token answers false") do
    token = Semaphore.try_acquire("spec-sem2", 1)
    assert(Semaphore.release("spec-sem2", token))
    assert_not(Semaphore.release("spec-sem2", token))
  end

  test("count reports the limit and the permits held") do
    token = Semaphore.try_acquire("spec-sem3", 1)
    assert_eq(Semaphore.count("spec-sem3"), {"limit": 1, "held": 1})
    Semaphore.release("spec-sem3", token)
    assert_eq(Semaphore.count("spec-sem3")["held"], 0)
  end

  test("count of a semaphore never used is nil") do
    assert_null(Semaphore.count("spec-sem-never-used"))
  end
end

describe("Money") do
  test("builds from a string and exposes amount and currency") do
    price = Money.new("49.90", "EUR")
    assert_eq(price["currency"], "EUR")
    assert_eq(type(price["amount"]), "decimal")
    assert_eq(price["amount"].to_s, "49.90")
    assert_eq(Money.format(price), "49.90 €")
  end

  test("an unparseable amount raises") do
    assert_raises("cannot parse \"abc\" as an amount") do
      Money.new("abc", "EUR")
    end
  end

  describe("arithmetic") do
    test("add sums exactly") do
      total = Money.add(Money.new("49.90", "EUR"), Money.new(99.1, "EUR"))
      assert_eq(Money.format(total, {"symbol": false}), "149.00 EUR")
      assert_eq(Money.compare(total, Money.new("149.00", "EUR")), 0)
    end

    test("add refuses mixed currencies") do
      assert_raises("currency mismatch (EUR vs USD)") do
        Money.add(Money.new(1, "EUR"), Money.new(1, "USD"))
      end
    end

    test("compare answers -1, 0 or 1") do
      assert_eq(Money.compare(Money.new(1, "EUR"), Money.new(2, "EUR")), -1)
      assert_eq(Money.compare(Money.new(2, "EUR"), Money.new(1, "EUR")), 1)
    end

    test("allocate splits without losing cents, the remainder going first") do
      shares = Money.allocate(Money.new(100, "EUR"), [1, 1, 1])
      formatted = shares.map { |share| Money.format(share, {"symbol": false}) }
      assert_eq(formatted, ["33.34 EUR", "33.33 EUR", "33.33 EUR"])

      sum = Money.add(Money.add(shares[0], shares[1]), shares[2])
      assert_eq(Money.compare(sum, Money.new(100, "EUR")), 0)
    end
  end

  describe("format") do
    test("uses locale separators") do
      assert_eq(Money.format(Money.new(1234.5, "EUR"), {"locale": "de"}), "1.234,50 €")
    end

    test("puts a dollar sign first and groups thousands") do
      assert_eq(Money.format(Money.new(1234567.5, "USD")), "$1,234,567.50")
    end

    test("keeps the sign of a negative amount") do
      assert_eq(Money.format(Money.new(-9.99, "USD"), {"symbol": false}), "-9.99 USD")
    end
  end
end
