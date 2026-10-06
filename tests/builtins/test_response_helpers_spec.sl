# The spec-side response helpers: res_* readers and status predicates over
# plain response hashes, and the query-count assertions over the hashes a
# test request returns.

def response_with(status)
  {"status": status}
end

describe("res_* readers") do
  test("res_status reads the status") do
    assert_eq(res_status({"status": 201, "body": "created"}), 201)
  end

  test("res_status raises when there is no status") do
    assert_raises("Response missing status field") do
      res_status({"body": "no status here"})
    end
  end

  test("res_body reads the body") do
    assert_eq(res_body({"status": 200, "body": "{\"ok\":true}"}), "{\"ok\":true}")
  end

  test("res_json parses a string body") do
    assert_eq(res_json({"status": 200, "body": "{\"ok\":true,\"n\":3}"}), {"ok": true, "n": 3})
  end

  test("res_header looks a header up case-insensitively, nil when absent") do
    response = {"status": 302, "headers": {"Location": "/dashboard", "X-Request-Id": "abc123"}}
    assert_eq(res_header(response, "location"), "/dashboard")
    assert_eq(res_header(response, "LOCATION"), "/dashboard")
    assert_eq(res_header(response, "X-Request-Id"), "abc123")
    assert_null(res_header(response, "Missing"))
  end

  test("res_location reads the Location header") do
    assert_eq(res_location({"status": 301, "headers": {"Location": "/new-home"}}), "/new-home")
  end

  test("res_headers returns every header, an empty hash when there are none") do
    assert_eq(res_headers({"status": 301, "headers": {"Location": "/new-home"}}), {"Location": "/new-home"})
    assert_eq(res_headers({"status": 200}), {})
  end
end

describe("res_* status predicates") do
  test("res_ok? covers 2xx, in both spellings") do
    assert(res_ok?(response_with(200)))
    assert(res_ok?(response_with(204)))
    assert(res_ok(response_with(299)))
    assert_not(res_ok?(response_with(199)))
    assert_not(res_ok?(response_with(300)))
    assert_not(res_ok?(response_with(404)))
  end

  test("res_redirect? covers 3xx") do
    assert(res_redirect?(response_with(301)))
    assert(res_redirect?(response_with(302)))
    assert(res_redirect(response_with(307)))
    assert_not(res_redirect?(response_with(200)))
    assert_not(res_redirect?(response_with(400)))
  end

  test("not_found, unauthorized, forbidden and unprocessable match one code each") do
    assert(res_not_found?(response_with(404)))
    assert_not(res_not_found?(response_with(400)))
    assert(res_unauthorized?(response_with(401)))
    assert_not(res_unauthorized?(response_with(403)))
    assert(res_forbidden?(response_with(403)))
    assert_not(res_forbidden?(response_with(401)))
    assert(res_unprocessable?(response_with(422)))
    assert_not(res_unprocessable?(response_with(400)))
    assert(res_not_found(response_with(404)))
    assert(res_unauthorized(response_with(401)))
    assert(res_forbidden(response_with(403)))
    assert(res_unprocessable(response_with(422)))
  end

  test("client_error covers 4xx and server_error 5xx") do
    assert(res_client_error?(response_with(400)))
    assert(res_client_error(response_with(499)))
    assert_not(res_client_error?(response_with(500)))
    assert(res_server_error?(response_with(500)))
    assert(res_server_error(response_with(503)))
    assert_not(res_server_error?(response_with(404)))
  end
end

describe("query-count assertions") do
  test("assert_query_count takes a bare count") do
    assert_query_count(3, 3)
    assert_raises("expected 4 queries but 3 ran") do
      assert_query_count(3, 4)
    end
  end

  test("assert_query_count reads query_count off a response") do
    response = {"query_count": 5}
    assert_query_count(response, 5)
    assert_raises("expected 2 queries but 5 ran") do
      assert_query_count(response, 2)
    end
  end

  test("assert_max_queries enforces an upper bound") do
    assert_max_queries(5, 10)
    assert_max_queries(5, 5)
    assert_raises("expected at most 5 queries but 8 ran") do
      assert_max_queries(8, 5)
    end
  end

  test("assert_no_n_plus_one passes a clean response and names a repeated template") do
    assert_no_n_plus_one({"query_count": 3, "n_plus_one": []})
    message = assert_raises("N+1 detected: 1 template(s) fired in a loop") do
      assert_no_n_plus_one({
        "query_count": 11,
        "n_plus_one": [{"query": "FOR doc IN posts RETURN doc", "count": 11}]
      })
    end
    assert_contains(message, "11x  FOR doc IN posts RETURN doc")
  end

  test("assert_no_ungrouped_reads passes coalesced reads and lists the stragglers") do
    assert_no_ungrouped_reads({"query_count": 1, "ungrouped_reads": []})
    message = assert_raises("3 reads each cost a round-trip outside any `grouped` block") do
      assert_no_ungrouped_reads({
        "query_count": 3,
        "ungrouped_reads": [
          {"query": "FOR doc IN posts RETURN doc"},
          {"query": "FOR doc IN accounts RETURN doc"},
          {"query": "FOR doc IN tags RETURN doc"}
        ]
      })
    end
    assert_contains(message, "FOR doc IN accounts RETURN doc")
  end

  test("dev_queries is empty when no database work ran") do
    assert_eq(dev_queries(), [])
  end
end

describe("viewport readback") do
  test("viewport() reports the default viewport without a browser") do
    assert_eq(viewport(), {"width": 1280, "height": 800, "scale": 1, "mobile": false})
  end
end
