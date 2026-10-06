# The spec-side auth helpers (as_user, as_guest, create_session…) keep the
# signed-in user in a thread-local, and the HTTP verb helpers need a test
# server, which a plain spec file does not start.

describe("auth test helpers") do
  before_each() do
    as_guest()
  end

  after_each() do
    as_guest()
  end

  test("a guest is signed out") do
    assert_eq(signed_in?(), false)
    assert_eq(signed_in(), false)
    assert_eq(signed_out(), true)
    assert_eq(signed_out?(), true)
    assert_null(current_user())
  end

  test("as_user signs in that user id") do
    assert_null(as_user(9))
    assert_eq(signed_in(), true)
    assert_eq(current_user(), {"id": 9})
  end

  test("as_user refuses anything but an integer id") do
    assert_raises("as_user(user_id) expects integer argument") do
      as_user({"id": 3})
    end
  end

  test("as_admin signs in user 1") do
    as_admin()
    assert_eq(current_user(), {"id": 1})
  end

  test("as_guest signs a user out") do
    as_user(5)
    as_guest()
    assert_null(current_user())
    assert_eq(signed_out(), true)
  end

  test("create_session signs the user in and destroy_session signs them out") do
    assert_eq(create_session(77), "session_test_77")
    assert_eq(current_user(), {"id": 77})
    assert_null(destroy_session())
    assert_eq(signed_in(), false)
  end

  test("with_token and clear_authorization return nil") do
    assert_null(with_token("abc123"))
    assert_null(clear_authorization())
  end
end

describe("HTTP verb helpers without a test server") do
  test("login raises") do
    assert_raises("Test server is not running") do
      login("user@example.com", "secret")
    end
  end

  test("get, post, put, patch and delete raise and say how to start one") do
    assert_raises("Test server is not running. Call test_server_start() first.") do
      get("/anywhere")
    end
    assert_raises("Test server is not running. Call test_server_start() first.") do
      post("/anywhere", {})
    end
    assert_raises("Test server is not running. Call test_server_start() first.") do
      put("/anywhere", {})
    end
    assert_raises("Test server is not running. Call test_server_start() first.") do
      patch("/anywhere", {})
    end
    assert_raises("Test server is not running. Call test_server_start() first.") do
      delete("/anywhere")
    end
  end

  test("head, options and request raise the same way") do
    assert_raises("Test server is not running") do
      head("/x")
    end
    assert_raises("Test server is not running") do
      options("/x")
    end
    assert_raises("Test server is not running") do
      request("PUT", "/x")
    end
  end
end

# test_server_start only reserves a port and flips state here: the app that
# answers runs in a subprocess the `soli test` parent spawns for an app
# project, and tests/ has none. So this checks the state machine, not traffic.
describe("test server state") do
  after_each() do
    test_server_stop()
  end

  test("is stopped with no URL by default") do
    assert_eq(test_server_running(), false)
    assert_eq(test_server_url(), "")
  end

  test("start reports a running server on a loopback URL") do
    port = test_server_start()
    assert_gt(port, 0)
    assert_eq(test_server_running(), true)
    assert_eq(test_server_url(), "http://127.0.0.1:#{port}")
  end

  test("stop clears the running state and the URL") do
    test_server_start()
    test_server_stop()
    assert_eq(test_server_running(), false)
    assert_eq(test_server_url(), "")
  end
end
