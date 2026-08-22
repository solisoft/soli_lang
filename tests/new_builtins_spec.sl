# ============================================================================
# Builtins spec — Url, Logger, Retry, CircuitBreaker, Toml/Yaml, Semaphore,
# Money. One describe per class so a failure names its surface.
# ============================================================================

describe("Url", fn() {
  test("parses components with null defaults", fn() {
    let u = Url.parse("https://user:pw@api.ex.com:8443/v1/items?page=2#top");
    assert_eq(u["scheme"], "https");
    assert_eq(u["host"], "api.ex.com");
    assert_eq(u["port"], 8443);
    assert_eq(u["path"], "/v1/items");
    assert_eq(u["query"], "page=2");
    assert_eq(u["fragment"], "top");
    assert_eq(u["username"], "user");

    let bare = Url.parse("https://ex.com");
    assert_null(bare["port"]);
    assert_null(bare["query"]);
    assert_null(bare["fragment"]);
  });

  test("round-trips query params through encoding", fn() {
    let params = Url.params("https://ex.com/?q=red%20shoe&flag&n=7");
    assert_eq(params["q"], "red shoe");
    assert_null(params["flag"]);
    assert_eq(params["n"], "7");

    assert_eq(Url.param("https://ex.com/?page=2", "page"), "2");
    assert_null(Url.param("https://ex.com/?page=2", "nope"));
  });

  test("set_param adds, replaces and removes", fn() {
    assert_eq(
      Url.set_param("https://ex.com/?b=2", "a", "1"),
      "https://ex.com/?b=2&a=1"
    );
    assert_eq(
      Url.set_param("https://ex.com/?a=1&b=2", "a", null),
      "https://ex.com/?b=2"
    );
  });

  test("joins relative references and builds from parts", fn() {
    assert_eq(Url.join("https://ex.com/a/b", "c"), "https://ex.com/a/c");
    assert_eq(Url.join("https://ex.com/a/b", "?page=2"), "https://ex.com/a/b?page=2");

    assert_eq(
      Url.build({ "scheme": "https", "host": "api.ex.com", "path": "/v1/x",
                  "query": { "page": 2, "q": "a b" } }),
      "https://api.ex.com/v1/x?page=2&q=a%20b"
    );
  });

  test("encodes and decodes components", fn() {
    assert_eq(Url.encode_component("Ann Lee"), "Ann%20Lee");
    assert_eq(Url.decode_component("Ann%20Lee"), "Ann Lee");
  });
});

describe("Logger", fn() {
  test("captures entries when capture is on", fn() {
    Logger.set_capture(true);
    Logger.info("hello", { "k": 1 });
    # Read while capture is still on — disabling clears the buffer.
    let entries = Logger.entries();
    Logger.set_capture(false);

    assert(entries.length() >= 1);
    assert(entries[entries.length() - 1].contains("hello"));
  });

  test("level is configurable", fn() {
    Logger.configure({ "level": "error" });
    assert_eq(Logger.level(), "ERROR");
    Logger.configure({ "level": "info" });
    assert_eq(Logger.level(), "INFO");
  });
});

describe("Retry", fn() {
  test("returns the block result on first success", fn() {
    let r = Retry.with_backoff(fn() { return "ok"; });
    assert_eq(r, "ok");
  });

  test("retries until the block succeeds", fn() {
    let attempts = { "n": 0 };
    let r = Retry.with_backoff(fn() {
      attempts["n"] = attempts["n"] + 1;
      if attempts["n"] < 3 {
        throw "flaky";
      }
      return "recovered on " + str(attempts["n"]);
    }, { "attempts": 5, "base_delay": 0.01 });
    assert_eq(r, "recovered on 3");

    let caught = Retry.with_backoff(fn() { throw "always"; }, { "attempts": 2, "base_delay": 0.01 })
      rescue "fallback";
    assert_eq(caught, "fallback");
  });

  test("within succeeds immediately without waiting the deadline", fn() {
    let started = DateTime.microtime();
    let r = Retry.within(fn() { return "ready"; }, { "deadline": 10 });
    assert_eq(r, "ready");
    assert(DateTime.microtime() - started < 5000000.0);
  });
});

describe("CircuitBreaker", fn() {
  test("transitions closed -> open -> half_open", fn() {
    CircuitBreaker.configure("spec-cb", { "threshold": 2, "reset_after": 0.05 });
    assert(CircuitBreaker.allow("spec-cb"));
    CircuitBreaker.record_failure("spec-cb");
    assert_eq(CircuitBreaker.state("spec-cb"), "closed");
    CircuitBreaker.record_failure("spec-cb");
    assert_eq(CircuitBreaker.state("spec-cb"), "open");
    assert(!CircuitBreaker.allow("spec-cb"));

    sleep(0.06);
    assert_eq(CircuitBreaker.state("spec-cb"), "half_open");
    CircuitBreaker.record_success("spec-cb");
    assert_eq(CircuitBreaker.state("spec-cb"), "closed");
    CircuitBreaker.reset("spec-cb");
  });

  test("success resets the failure count", fn() {
    CircuitBreaker.configure("spec-cb2", { "threshold": 2 });
    CircuitBreaker.record_failure("spec-cb2");
    CircuitBreaker.record_success("spec-cb2");
    CircuitBreaker.record_failure("spec-cb2");
    assert_eq(CircuitBreaker.state("spec-cb2"), "closed");
    CircuitBreaker.reset("spec-cb2");
  });
});

describe("Toml / Yaml", fn() {
  test("toml parses nested documents", fn() {
    let config = Toml.parse("title = \"demo\"\n\n[owner]\nname = \"Ann\"\nports = [1, 2]\n");
    assert_eq(config["title"], "demo");
    assert_eq(config["owner"]["name"], "Ann");
    assert_eq(config["owner"]["ports"][1], 2);
  });

  test("toml rejects garbage cleanly", fn() {
    let out = Toml.parse("this is [ not toml") rescue "bad";
    assert_eq(out, "bad");
  });

  test("yaml parses arrays and nesting", fn() {
    let doc = Yaml.parse("name: demo\ncounts:\n  - 1\n  - 2\n");
    assert_eq(doc["name"], "demo");
    assert_eq(doc["counts"][1], 2);

    let text = Yaml.stringify({ "name": "x" });
    assert(text.contains("name"));
  });
});

describe("Semaphore", fn() {
  test("acquires up to the limit then refuses then releases", fn() {
    let t1 = Semaphore.try_acquire("spec-sem", 2);
    let t2 = Semaphore.try_acquire("spec-sem", 2);
    assert(t1.present?);
    assert(t2.present?);
    assert_null(Semaphore.try_acquire("spec-sem", 2));

    assert(Semaphore.release("spec-sem", t1));
    let t3 = Semaphore.try_acquire("spec-sem", 2);
    assert(t3.present?);

    Semaphore.release("spec-sem", t2);
    Semaphore.release("spec-sem", t3);
  });

  test("double release is detectable", fn() {
    let t = Semaphore.try_acquire("spec-sem2", 1);
    assert(Semaphore.release("spec-sem2", t));
    assert(!Semaphore.release("spec-sem2", t));
  });

  test("count reports occupancy", fn() {
    let t = Semaphore.try_acquire("spec-sem3", 1);
    let c = Semaphore.count("spec-sem3");
    assert_eq(c["limit"], 1);
    assert_eq(c["held"], 1);
    Semaphore.release("spec-sem3", t);
  });
});

describe("Money", fn() {
  test("builds from strings and exposes amount/currency", fn() {
    let m = Money.new("49.90", "EUR");
    assert_eq(m["currency"], "EUR");
    assert_eq(Money.format(m), "49.90 €");
  });

  test("arithmetic is currency-checked", fn() {
    let total = Money.add(Money.new("49.90", "EUR"), Money.new(99.10, "EUR"));
    assert_eq(Money.format(total, { "symbol": false }), "149.00 EUR");
    assert_eq(Money.compare(total, Money.new("149.00", "EUR")), 0);

    let mismatch = Money.add(Money.new(1, "EUR"), Money.new(1, "USD")) rescue "mismatch";
    assert_eq(mismatch, "mismatch");
  });

  test("allocate splits without losing cents", fn() {
    let shares = Money.allocate(Money.new(100, "EUR"), [1, 1, 1]);
    assert_eq(shares.length(), 3);
    assert_eq(Money.format(shares[0], { "symbol": false }), "33.34 EUR");
    assert_eq(Money.format(shares[1], { "symbol": false }), "33.33 EUR");

    let sum = Money.add(Money.add(shares[0], shares[1]), shares[2]);
    assert_eq(Money.compare(sum, Money.new(100, "EUR")), 0);
  });

  test("formats locales", fn() {
    assert_eq(Money.format(Money.new(1234.5, "EUR"), { "locale": "de" }), "1.234,50 €");
    assert_eq(Money.format(Money.new(-9.99, "USD"), { "symbol": false }), "-9.99 USD");
  });
});
