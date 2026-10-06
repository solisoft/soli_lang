# The router builtins that config/routes.sl compiles to. They register into
# the router and return nil; only their argument checks are observable here.

describe("router builtins") do
  test("router_match registers a route") do
    assert_null(router_match("GET", "/spec/match/path", "specs#show"))
    assert_null(router_match("GET", "/spec/match/:id", "specs#show", "spec_match"))
  end

  test("router_match needs 3 or 4 arguments") do
    assert_raises("router_match expects 3 or 4 arguments, got 2") do
      router_match("GET", "/x")
    end
  end

  test("resource enter and exit balance out") do
    assert_null(router_resource_enter("spec_widgets", {}))
    assert_null(router_resource_exit())
  end

  test("router_resource_enter needs a string name") do
    assert_raises("Expected string for resource name") do
      router_resource_enter(42, {})
    end
  end

  test("router_live registers a LiveView route") do
    assert_null(router_live("spec_counter", "live#counter"))
  end

  test("skip_csrf registers an exemption pattern") do
    assert_null(skip_csrf("/spec/webhooks/*"))
  end
end

describe("cors") do
  test("registers a rule with or without options") do
    assert_null(cors("/spec/api/*"))
    assert_null(cors("/spec/strict/*", {"origins": ["https://app.example.com"], "credentials": true, "max_age": 600}))
  end

  test("refuses an unknown option and lists the known ones") do
    assert_raises("unknown option 'originz' (expected origins, methods, headers, expose, credentials, max_age)") do
      cors("/spec/bad/*", {"originz": "*"})
    end
  end
end
