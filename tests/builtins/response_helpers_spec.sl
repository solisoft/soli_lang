# Controller response helpers called outside a request: redirects build their
# response hash, halt/forbidden raise, and the view helpers report nothing.

describe("redirect") do
  test("builds a 302 to a local path") do
    assert_eq(redirect("/dashboard"), {"status": 302, "headers": {"Location": "/dashboard"}, "body": ""})
  end

  test("keeps the query string and fragment") do
    assert_eq(redirect("/a?b=1#c")["headers"]["Location"], "/a?b=1#c")
  end

  test("refuses an off-site URL") do
    assert_raises("redirect() only accepts local absolute paths like '/dashboard'") do
      redirect("https://evil.example.com/phish")
    end
  end

  test("refuses a protocol-relative URL") do
    assert_raises("redirect() only accepts local absolute paths") do
      redirect("//evil.example.com")
    end
  end
end

describe("redirect_external") do
  test("builds a 302 to an https URL") do
    response = redirect_external("https://payments.example.net/checkout")
    assert_eq(response["status"], 302)
    assert_eq(response["headers"]["Location"], "https://payments.example.net/checkout")
  end

  test("refuses anything but http and https") do
    assert_raises("redirect_external() only accepts http:// or https:// URLs") do
      redirect_external("javascript:alert(1)")
    end
    assert_raises("redirect_external() only accepts http:// or https:// URLs") do
      redirect_external("/local")
    end
  end
end

describe("halt and forbidden") do
  test("halt raises with exactly its message") do
    message = assert_raises() do
      halt(404, "Not here")
    end
    assert_eq(message, "Not here")
  end

  test("forbidden raises with its message") do
    assert_raises("no access") do
      forbidden("no access")
    end
  end
end

describe("render helpers outside a request") do
  test("render_text returns nil and needs a body") do
    assert_null(render_text("plain body"))
    assert_raises("render_text() requires at least one argument") do
      render_text()
    end
  end

  test("render_jsonp falls back to JSON without a callback") do
    assert_null(render_jsonp({"answer": 42}))
  end

  test("partial raises before the templates are initialized") do
    assert_raises("Template system not initialized. Call init_templates() first.") do
      partial("nonexistent/partial_name_xyz")
    end
  end

  test("render_partial raises the same way") do
    assert_raises("Template system not initialized") do
      render_partial("nonexistent/partial_name_xyz")
    end
  end
end

describe("request state outside a request") do
  before_each() do
    # The token lives in the session. An earlier spec file on this worker
    # (form_builder_spec) can leave a session id whose session this store
    # does not hold, and csrf_token then mints a new token on every call.
    session_regenerate()
  end

  test("csrf_token is a stable 32-character hex token") do
    token = csrf_token()
    assert_match(token, "^[0-9a-f]{32}$")
    assert_eq(csrf_token(), token)
  end

  test("current_action is empty") do
    assert_eq(current_action(), "")
  end

  test("nothing is assigned or rendered") do
    assert_eq(assigns(), {})
    assert_null(assign("anything"))
    assert_eq(view_path(), "")
    assert_eq(render_template(), false)
  end
end
