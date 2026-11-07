# Template entry points (render / render_partial / partial) as a spec sees
# them. A spec runs without a views directory — only `soli serve` and
# `soli jobs` call init_templates — so no builtin can render an ERB template
# here: the engine itself (<%= %> escaping, <%- %>, loops, content_for,
# comments) is covered by the Rust unit tests in src/template/ and by the
# server end-to-end tests. What a spec can pin is how the entry points
# validate their arguments and fail without a template system.

describe("render() argument validation") do
  test("requires a template name") do
    assert_raises("render() requires at least 1 argument (template name)") do
      render()
    end
  end

  test("refuses a template name that is not a string") do
    assert_raises("render() template name must be a string, got int") do
      render(1)
    end
  end

  test("refuses data that is not a hash") do
    assert_raises("render() data must be a hash, got int") do
      render("posts/index", 5)
    end
  end

  test("refuses options that are not a hash") do
    assert_raises("render() options must be a hash, got int") do
      render("posts/index", {}, 5)
    end
  end

  test("refuses an option it does not know, naming the ones it takes") do
    assert_raises("unknown option \"header\": it takes \"layout\", \"status\", \"headers\"") do
      render("posts/index", {}, {"header": {"X-A": "1"}})
    end
  end

  test("refuses headers that are not a hash") do
    assert_raises("render() option \"headers\" must be a hash, got string") do
      render("posts/index", {}, {"headers": "Cache-Control: no-store"})
    end
  end

  test("refuses a header value holding a CR or LF") do
    assert_raises("control character") do
      render("posts/index", {}, {"headers": {"X-A": "a\r\nSet-Cookie: x=1"}})
    end
  end

  test("refuses a header name that is not an HTTP token") do
    assert_raises("is not a valid HTTP header name") do
      render("posts/index", {}, {"headers": {"X A": "1"}})
    end
  end

  test("checks the headers before looking for the template") do
    assert_raises("Template system not initialized") do
      options = {"status": 200, "headers": {"Cache-Control": "no-store", "X-Count": 3}}
      render("posts/index", {}, options)
    end
  end
end

describe("rendering without a template system") do
  test("render raises a clear error instead of returning empty output") do
    assert_raises("Template system not initialized") do
      render("hello", {"name": "World"})
    end
  end

  test("render_partial raises the same error") do
    assert_raises("Template system not initialized") do
      render_partial("hello", {"name": "World"})
    end
  end

  test("partial raises the same error") do
    assert_raises("Template system not initialized") do
      partial("hello", {"name": "World"})
    end
  end
end
