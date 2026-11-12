# The site's middleware, called as the router calls it: a request hash in,
# `{"continue": …}` out.

describe("authenticate middleware") do
  test("refuses a request with no API key") do
    result = authenticate({"headers": {}})
    assert_eq(result["continue"], false)
    assert_eq(result["response"]["status"], 401)
    assert_eq(json_parse(result["response"]["body"])["error"], "Unauthorized")
  end

  test("refuses a wrong API key") do
    result = authenticate({"headers": {"X-Api-Key": "wrong-key"}})
    assert_eq(result["continue"], false)
  end

  test("lets a valid key through, in either header case") do
    assert_eq(authenticate({"headers": {"X-Api-Key": "secret-key-123"}})["continue"], true)
    assert_eq(authenticate({"headers": {"x-api-key": "secret-key-123"}})["continue"], true)
  end
end

describe("add_cors_headers middleware") do
  test("answers an OPTIONS preflight itself") do
    result = add_cors_headers({"method": "OPTIONS", "headers": {}})
    assert_eq(result["continue"], false)
    assert_eq(result["response"]["status"], 204)
    assert_eq(result["response"]["headers"]["Access-Control-Allow-Origin"], "*")
  end

  test("lets any other request continue") do
    request = {"method": "GET", "headers": {}}
    result = add_cors_headers(request)
    assert_eq(result["continue"], true)
    assert_eq(result["request"], request)
  end
end
