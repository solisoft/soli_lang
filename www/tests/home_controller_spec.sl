# HomeController: the probes a load balancer and an uptime check call.

describe("HomeController") do
  describe("GET /up") do
    # The framework answers `/up` itself (www/docs/ci-cd.md), before the
    # router: the route to `home#up` is never reached.
    test("is the built-in readiness probe") do
      response = get("/up")
      assert_eq(res_status(response), 200)
      assert_eq(res_body(response), "ready")
    end
  end

  describe("GET /health") do
    test("answers a JSON status") do
      response = get("/health")
      assert_eq(res_status(response), 200)
      assert_eq(res_json(response)["status"], "ok")
    end
  end
end
