# Tests for app/middleware/auth.sl — written by `soli new`. A middleware is a
# function: call it with a request hash. Update these with your own logic.

describe("authenticate middleware") do
  test("stops a request without an API key") do
    req = {"headers": {}}
    assert(authenticate(req) != req)
  end

  test("passes a request with an API key on") do
    req = {"headers": {"x-api-key": "secret"}}
    assert_eq(authenticate(req), req)
  end
end
