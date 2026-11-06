# The HTTP client against the in-process mock server. An unscripted path
# answers 200 with {"ok":true}; `mock_http_route` scripts a status and body for
# one path, and `mock_http_last_body` shows what the client sent there. Every
# path lives under /http-spec/ so no other spec's scripted routes collide.

port = mock_http_server_start()
base = "http://127.0.0.1:" + str(port)
const OK_BODY = "{\"ok\":true}"

# Bytes a text read chokes on — a PNG signature (0x89, CR LF, 0x1A), NUL,
# 0xFF 0xFE, a Latin-1 é — then every byte value once.
const BINARY_BYTES = [137, 80, 78, 71, 13, 10, 26, 10, 0, 255, 254, 233, ...(0..256).map { |i| i }]
const BINARY_PATH = "/tmp/soli_http_spec_binary.png"

def url(path)
  "#{base}/http-spec/#{path}"
end

# The status of an HTTP.stream_to stream once it is no longer running.
def stream_to_result(id)
  status = HTTP.stream_status(id)
  tries = 0
  while status["state"] == "running" && tries < 200
    sleep(0.02)
    tries += 1
    status = HTTP.stream_status(id)
  end
  status
end

describe("HTTP") do
  before_each() do
    mock_http_route("/http-spec/hello", 200, "hello")
    mock_http_route("/http-spec/missing", 404, "{\"error\":\"nope\"}")
    mock_http_route("/http-spec/boom", 500, "boom")
    mock_http_route("/http-spec/jsonp", 200, "cb({\"a\":1})")
  end

  describe("get") do
    test("returns the body of a 200") do
      assert_eq(HTTP.get(url("hello")), "hello")
      assert_eq(HTTP.get(url("anything")), OK_BODY)
    end

    test("raises on a 4xx with the status and body") do
      assert_raises("HTTP 404 error: {\"error\":\"nope\"}") do
        HTTP.get(url("missing"))
      end
    end

    test("raises on a 5xx") do
      assert_raises("HTTP 500 error: boom") do
        HTTP.get(url("boom"))
      end
    end

    test("refuses a scheme other than http and https") do
      assert_raises("URL scheme 'ftp:' is not allowed") do
        HTTP.get("ftp://example.com/")
      end
    end

    test("refuses an invalid header name before sending") do
      assert_raises("Invalid HTTP header name: \"Bad Name\"") do
        HTTP.get(url("headers"), {"headers": {"Bad Name": "v"}})
      end
    end
  end

  describe("bodies") do
    test("post sends a hash as JSON") do
      assert_eq(HTTP.post(url("post"), {"key": "value"}), OK_BODY)
      assert_eq(mock_http_last_body("/http-spec/post"), "{\"key\":\"value\"}")
    end

    test("post sends a string as-is") do
      HTTP.post(url("post_text"), "raw text")
      assert_eq(mock_http_last_body("/http-spec/post_text"), "raw text")
    end

    test("put sends a hash as JSON") do
      assert_eq(HTTP.put(url("put"), {"data": "test"}), OK_BODY)
      assert_eq(mock_http_last_body("/http-spec/put"), "{\"data\":\"test\"}")
    end

    test("patch sends a hash as JSON") do
      assert_eq(HTTP.patch(url("patch"), {"name": "Dave"}), OK_BODY)
      assert_eq(mock_http_last_body("/http-spec/patch"), "{\"name\":\"Dave\"}")
    end
  end

  describe("delete and head") do
    test("delete returns the body and raises on a 4xx") do
      assert_eq(HTTP.delete(url("delete")), OK_BODY)
      assert_raises("HTTP 404 error") do
        HTTP.delete(url("missing"))
      end
    end

    test("head returns the status line") do
      assert_eq(HTTP.head(url("ping")), "200 OK")
      assert_eq(HTTP.head(url("missing")), "404 Not Found")
    end
  end

  describe("request") do
    test("returns the whole response") do
      response = HTTP.request("GET", url("hello"))
      assert_eq(response["status"], 200)
      assert_eq(response["status_text"], "OK")
      assert_eq(response["body"], "hello")
      assert_eq(response["headers"]["content-type"], "application/json")
      assert_eq(response["headers"]["content-length"], "5")
    end

    test("returns a non-2xx response instead of raising") do
      response = HTTP.request("GET", url("missing"))
      assert_eq(response["status"], 404)
      assert_eq(response["status_text"], "Not Found")
      assert_eq(response["body"], "{\"error\":\"nope\"}")
    end

    test("sends its fourth argument as the body") do
      HTTP.request("POST", url("request_body"), {}, "payload")
      assert_eq(mock_http_last_body("/http-spec/request_body"), "payload")
    end

    test("keeps every Set-Cookie as an array, joins other repeated headers") do
      cookies = ["a=1; Path=/; Expires=Wed, 21 Oct 2026 07:28:00 GMT", "b=2; HttpOnly"]
      mock_http_route("/http-spec/cookies", 200, "ok", {"Set-Cookie": cookies, "X-Thing": ["one", "two"]})
      response = HTTP.request("GET", url("cookies"))
      assert_eq(response["headers"]["set-cookie"], cookies)
      assert_eq(response["headers"]["x-thing"], "one, two")
      assert_eq(res_header(response, "Set-Cookie"), cookies)
    end

    test("answers a single Set-Cookie as a one-element array") do
      mock_http_route("/http-spec/one_cookie", 200, "ok", {"Set-Cookie": "only=1"})
      response = HTTP.request("GET", url("one_cookie"))
      assert_eq(response["headers"]["set-cookie"], ["only=1"])
    end
  end

  describe("redirects") do
    before_each() do
      mock_http_route("/http-spec/moved", 302, "moved", {"Location": url("hello")})
    end

    test("are followed by default") do
      response = HTTP.request("GET", url("moved"))
      assert_eq(response["status"], 200)
      assert_eq(response["body"], "hello")
      assert_eq(HTTP.get(url("moved")), "hello")
    end

    test("follow_redirects: false returns the 3xx and its location") do
      response = HTTP.request("GET", url("moved"), {"follow_redirects": false})
      assert_eq(response["status"], 302)
      assert_eq(response["status_text"], "Found")
      assert_eq(response["headers"]["location"], url("hello"))
      assert_eq(response["body"], "moved")
    end

    test("follow_redirects: false reaches the verb helpers and HTTP.parallel") do
      assert_eq(HTTP.head(url("moved"), {"follow_redirects": false}), "302 Found")
      assert_raises("HTTP 302 error: moved") do
        HTTP.get(url("moved"), {"follow_redirects": false})
      end
      results = HTTP.parallel([
        {"url": url("moved"), "follow_redirects": false},
        {"url": url("moved")}
      ])
      assert_eq(results[0]["status"], 302)
      assert_eq(results[0]["headers"]["location"], url("hello"))
      assert_eq(results[1]["status"], 200)
    end

    test("follow_redirects must be a bool") do
      assert_raises("HTTP follow_redirects must be true or false, got string") do
        HTTP.request("GET", url("moved"), {"follow_redirects": "false"})
      end
    end
  end

  describe("response option") do
    before_each() do
      barf(BINARY_PATH, BINARY_BYTES)
      mock_http_route("/http-spec/file.png", 200, slurp(BINARY_PATH, "binary"))
      mock_http_route("/http-spec/empty", 200, [])
      mock_http_route("/http-spec/latin1", 200, Encoding.encode("café\r\nà", "latin1"))
    end

    after_each() do
      File.delete(BINARY_PATH) if file_exists(BINARY_PATH)
    end

    test("binary: the body is the bytes, as slurp(path, \"binary\") reads them") do
      response = HTTP.request("GET", url("file.png"), {"Authorization": "Bearer t", "response": "binary"})
      assert_eq(response["status"], 200)
      assert_eq(response["body"], slurp(BINARY_PATH, "binary"))
      assert_eq(response["body"].length, 268)
      assert_eq(HTTP.get(url("file.png"), {"response": "binary"}), slurp(BINARY_PATH, "binary"))
    end

    test("base64: the body is Base64.encode of the bytes") do
      expected = Base64.encode(slurp(BINARY_PATH, "binary"))
      response = HTTP.request("GET", url("file.png"), {"response": "base64"})
      assert_eq(response["body"], expected)
      assert_eq(HTTP.get(url("file.png"), {"response": "base64"}), expected)
      assert_eq(HTTP.post(url("file.png"), "", {"response": "base64"}), expected)
    end

    test("a charset: the body is decoded into UTF-8") do
      response = HTTP.request("GET", url("latin1"), {"response": "latin1"})
      assert_eq(response["body"], "café\r\nà")
      assert_eq(HTTP.get(url("latin1"), {"response": "iso-8859-1"}), "café\r\nà")
    end

    test("an empty body is an empty array, or an empty string") do
      assert_eq(HTTP.request("GET", url("empty"), {"response": "binary"})["body"], [])
      assert_eq(HTTP.get(url("empty"), {"response": "base64"}), "")
      assert_eq(HTTP.get(url("empty"), {"response": "latin1"}), "")
    end

    test("without it the body is UTF-8 text, as before") do
      assert_eq(HTTP.request("GET", url("hello"))["body"], "hello")
      assert_eq(HTTP.get(url("hello"), {"response": "text"}), "hello")
      assert_raises("invalid UTF-8 in response body") do
        HTTP.request("GET", url("file.png"))
      end
      assert_raises("use {\"response\": \"binary\"|\"base64\"}") do
        HTTP.get(url("latin1"))
      end
    end

    test("an unknown value, or one that is not a string, raises") do
      assert_raises("HTTP response must be \"text\", \"binary\", \"base64\" or a charset label like \"latin1\", got \"bytes\"") do
        HTTP.request("GET", url("hello"), {"response": "bytes"})
      end
      assert_raises("HTTP response must be a string") do
        HTTP.get(url("hello"), {"response": true})
      end
    end

    test("the JSON helpers refuse a byte mode") do
      assert_raises("HTTP.get_json() parses the body as text") do
        HTTP.get_json(url("hello"), {"response": "binary"})
      end
    end

    test("HTTP.parallel takes it per request, HTTP.get_all for the batch") do
      results = HTTP.parallel([
        {"url": url("file.png"), "response": "binary"},
        {"url": url("hello")}
      ])
      assert_eq(results[0]["body"], slurp(BINARY_PATH, "binary"))
      assert_eq(results[1]["body"], "hello")
      bodies = HTTP.get_all([url("file.png"), url("empty")], {"response": "base64"})
      assert_eq(bodies, [Base64.encode(slurp(BINARY_PATH, "binary")), ""])
    end

    test("a stream gives its raw bytes with next_chunk") do
      stream = HTTP.stream("GET", url("file.png"), {"response": "binary"})
      bytes = []
      chunk = stream.next_chunk
      while !chunk.nil?
        bytes = [...bytes, ...chunk]
        chunk = stream.next_chunk
      end
      assert_eq(bytes, slurp(BINARY_PATH, "binary"))
      assert_null(stream.next_chunk)
    end

    test("a base64 stream gives its chunks as strings, a charset stream its lines decoded") do
      stream = HTTP.stream("GET", url("file.png"), {"response": "base64"})
      assert_eq(type(stream.next_chunk), "string")
      stream.close
      stream = HTTP.stream("GET", url("latin1"), {"response": "latin1"})
      assert_eq(stream.next_line, "café")
      assert_eq(stream.next_line, "à")
      assert_null(stream.next_line)
    end

    test("mock_http_route refuses an array that is not bytes") do
      assert_raises("mock_http_route: an Array body holds bytes (Ints 0..=255)") do
        mock_http_route("/http-spec/bad", 200, [256])
      end
    end
  end

  describe("stream") do
    test("reads the answer line by line, then nil") do
      mock_http_route("/http-spec/events", 200, "data: {\"a\":1}\n\ndata: {\"a\":2}\n\ndata: [DONE]")
      stream = HTTP.stream("POST", url("events"), {"Authorization": "Bearer k", "timeout": 5}, "{\"q\":1}")
      assert_eq(stream.status, 200)
      assert_eq(stream.headers["content-type"], "application/json")
      lines = []
      line = stream.next_line
      while !line.nil?
        lines.push(line)
        line = stream.next_line
      end
      assert_eq(lines, ["data: {\"a\":1}", "", "data: {\"a\":2}", "", "data: [DONE]"])
      assert_null(stream.next_line)
      assert_eq(mock_http_last_body("/http-spec/events"), "{\"q\":1}")
    end

    test("answers a non-2xx status, its body readable") do
      stream = HTTP.stream("GET", url("missing"))
      assert_eq(stream.status, 404)
      assert_eq(stream.next_line, "{\"error\":\"nope\"}")
      stream.close
      assert_null(stream.next_line)
    end

    test("refuses a URL the SSRF check refuses, and a bad timeout") do
      assert_raises("not allowed") do
        HTTP.stream("GET", "file:///etc/passwd")
      end
      assert_raises() do
        HTTP.stream("GET", url("hello"), {"timeout": "soon"})
      end
    end
  end

  describe("stream_to") do
    test("reads an answer in the background and keeps its text") do
      mock_http_route("/http-spec/answer", 200, "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Hel\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"lo\"}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n")
      id = HTTP.stream_to("POST", url("answer"), {"timeout": 5}, "{\"q\":1}", {"format": "anthropic"})
      status = stream_to_result(id)
      assert_eq(status["state"], "done")
      assert_eq(status["text"], "Hello")
      assert_eq(status["status"], 200)
      assert_null(status["error"])
      assert_eq(mock_http_last_body("/http-spec/answer"), "{\"q\":1}")
    end

    test("a non-2xx answer fails with its body as the error") do
      status = stream_to_result(HTTP.stream_to("GET", url("missing"), {}, nil, {"format": "openai"}))
      assert_eq(status["state"], "failed")
      assert_eq(status["status"], 404)
      assert_eq(status["error"], "{\"error\":\"nope\"}")
    end

    test("refuses a wrong target at once") do
      assert_raises("unknown format") do
        HTTP.stream_to("GET", url("hello"), {}, nil, {"format": "xml"})
      end
      assert_raises("job class") do
        HTTP.stream_to("GET", url("hello"), {}, nil, {"job": "answer_done"})
      end
      assert_raises("not allowed") do
        HTTP.stream_to("GET", "file:///etc/passwd", {}, nil, {})
      end
      assert_null(HTTP.stream_status("no-such-stream"))
      assert_eq(HTTP.stream_cancel("no-such-stream"), false)
    end
  end

  describe("JSON verbs") do
    test("get_json parses the body") do
      assert_eq(HTTP.get_json(url("users")), {"ok": true})
    end

    test("get_json raises on a 4xx") do
      assert_raises("HTTP 404 error") do
        HTTP.get_json(url("missing"))
      end
    end

    test("post_json, put_json and patch_json send JSON and parse the reply") do
      assert_eq(HTTP.post_json(url("post_json"), {"name": "Alice"}), {"ok": true})
      assert_eq(mock_http_last_body("/http-spec/post_json"), "{\"name\":\"Alice\"}")
      assert_eq(HTTP.put_json(url("put_json"), {"name": "Bob"}), {"ok": true})
      assert_eq(mock_http_last_body("/http-spec/put_json"), "{\"name\":\"Bob\"}")
      assert_eq(HTTP.patch_json(url("patch_json"), {"name": "Carol"}), {"ok": true})
      assert_eq(mock_http_last_body("/http-spec/patch_json"), "{\"name\":\"Carol\"}")
    end

    test("get_jsonp unwraps the callback padding") do
      assert_eq(HTTP.get_jsonp(url("jsonp")), {"a": 1})
    end

    test("get_jsonp refuses a body with no callback") do
      assert_raises("not a JSONP response: no '(' found") do
        HTTP.get_jsonp(url("plain-json"))
      end
    end
  end

  describe("json_parse and json_stringify") do
    test("json_parse parses an object") do
      assert_eq(HTTP.json_parse("{\"name\":\"Alice\",\"age\":30}"), {"name": "Alice", "age": 30})
    end

    test("json_parse keeps arrays, booleans, floats and null") do
      assert_eq(HTTP.json_parse("[1, 2, 3]"), [1, 2, 3])
      assert_eq(HTTP.json_parse("{\"active\": true, \"score\": 9.5, \"note\": null}"), {
        "active": true,
        "score": 9.5,
        "note": nil
      })
    end

    test("json_stringify writes compact JSON") do
      assert_eq(HTTP.json_stringify(42), "42")
      assert_eq(HTTP.json_stringify([1, 2, 3]), "[1,2,3]")
      assert_eq(HTTP.json_stringify({"a": [1, nil, true], "b": "x"}), "{\"a\":[1,null,true],\"b\":\"x\"}")
    end

    test("json_stringify and json_parse round-trip a hash") do
      person = {"name": "Bob", "tags": ["a", "b"], "count": 7}
      assert_eq(HTTP.json_parse(HTTP.json_stringify(person)), person)
    end

    test("json_parse raises on invalid JSON") do
      assert_raises("Failed to parse JSON") do
        HTTP.json_parse("{definitely not json")
      end
    end
  end

  describe("batches") do
    test("get_all fetches URLs in parallel as raw text, in order") do
      assert_eq(HTTP.get_all([url("hello"), url("a"), url("b")]), ["hello", OK_BODY, OK_BODY])
    end

    test("get_all puts an error hash where a request failed") do
      assert_eq(HTTP.get_all([url("hello"), url("missing")]), [
        "hello",
        {"error": "HTTP 404 error: {\"error\":\"nope\"}"}
      ])
    end

    test("get_all_json parses every body") do
      assert_eq(HTTP.get_all_json([url("a"), url("missing")]), [
        {"ok": true},
        {"error": "HTTP 404 error: {\"error\":\"nope\"}"}
      ])
    end

    test("get_all and get_all_json return an empty array for no URLs") do
      assert_eq(HTTP.get_all([]), [])
      assert_eq(HTTP.get_all_json([]), [])
    end

    test("parallel returns whole responses, non-2xx included") do
      results = HTTP.parallel([{"url": url("one")}, {"url": url("missing"), "method": "GET"}])
      assert_eq(results.length, 2)
      assert_eq(results[0]["status"], 200)
      assert_eq(results[0]["body"], OK_BODY)
      assert_eq(results[0]["headers"]["content-type"], "application/json")
      assert_eq(results[1]["status"], 404)
      assert_eq(results[1]["status_text"], "Not Found")
    end

    test("parallel sends a POST config's headers and body") do
      results = HTTP.parallel([{
        "url": url("parallel_post"),
        "method": "POST",
        "headers": {"X-Custom": "yes"},
        "body": {"key": "value"}
      }])
      assert_eq(results[0]["status"], 200)
      assert_eq(mock_http_last_body("/http-spec/parallel_post"), "{\"key\":\"value\"}")
    end

    test("parallel returns an empty array for no configs") do
      assert_eq(HTTP.parallel([]), [])
    end
  end

  describe("failures") do
    # Nothing listens on port 1. The failure must surface at the call, where
    # the rescue is, not at the first read of the response.
    test("a rescue on the call catches a failed request") do
      assert_eq(HTTP.request("GET", "http://127.0.0.1:1/") rescue "rescued", "rescued")
      assert_eq(HTTP.get("http://127.0.0.1:1/") rescue "rescued", "rescued")
    end

    test("try/catch around the call catches a failed request") do
      outcome = "none"
      try
        HTTP.request("GET", "http://127.0.0.1:1/")
        outcome = "no error"
      catch error
        outcome = "caught"
      end
      assert_eq(outcome, "caught")
    end
  end
end
