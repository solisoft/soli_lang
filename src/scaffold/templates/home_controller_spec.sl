# HomeController E2E tests — written by `soli new`. See docs/testing-e2e.md.

describe("HomeController") do
  test("GET / renders the home page") do
    response = get("/")
    assert_eq(res_status(response), 200)
    assert_eq(view_path(), "home/index.html")
    assert_eq(assigns()["title"], "Welcome")
  end

  test("GET /health answers ok") do
    response = get("/health")
    assert_eq(res_status(response), 200)
    assert_eq(json_parse(res_body(response))["status"], "ok")
  end
end
