# Controller behavior reachable without a server: class configuration, actions
# and private helpers, response helpers (redirect, halt, render_json), strong
# parameters with permit(), and respond_to content negotiation.

class ParentController < Controller
  static {
    this.layout = "parent"
  }
end

class ChildController < ParentController
end

class OverrideController < ParentController
  static {
    this.layout = "override"
  }
end

# The filtered hook DSL desugars `this.before_action(:show) = fn` into a call
# on a no-op static method; real registration happens when `soli serve` scans
# the source. Defining this class proves the DSL parses and resolves.
class FilteredController < Controller
  static {
    this.before_action(:show, :edit) = fn(req) { req }
    this.after_action(:create) = fn(req, response) { response }
  }
end

class ArticlesController < Controller
  # GET /articles/:id
  def show(req)
    @title = "Article #{req["id"]}"
    {"status": 200, "body": @title}
  end

  # GET /articles/:id/summary
  def summary(req)
    @_shout(req["id"])
  end

  private

  def _shout(id)
    "ARTICLE #{id}"
  end
end

# A request hash as respond_to reads it
def request_with(headers, path = "/posts/1", query = {})
  {"headers": headers, "path": path, "query": query}
end

def respond(body)
  {"status": 200, "headers": {}, "body": body}
end

# A respond_to run from inside another one's branch
def inner_json_body
  inner = respond_to(request_with({"accept": "application/json"}, "/p"), fn(format) {
    format.json { respond("inner-json") }
  })
  inner["body"]
end

describe("Controller classes") do
  test("an action is an instance method whose last expression is the response") do
    assert_eq(new ArticlesController().show({"id": 3}), {"status": 200, "body": "Article 3"})
  end

  test("a controller instance is a Controller") do
    assert(new ArticlesController().is_a?("Controller"))
    assert(new FilteredController().is_a?("Controller"))
  end

  test("an action can call a private helper on self") do
    assert_eq(new ArticlesController().summary({"id": 7}), "ARTICLE 7")
  end

  test("a private helper cannot be called from outside") do
    assert_raises("private method '_shout' called for an instance of ArticlesController") do
      new ArticlesController()._shout(1)
    end
  end

  test("a static block sets the layout") do
    assert_eq(ParentController.layout, "parent")
  end

  test("a child controller inherits the parent's layout") do
    assert_eq(ChildController.layout, "parent")
  end

  test("a child controller can override the layout") do
    assert_eq(OverrideController.layout, "override")
    assert_eq(ParentController.layout, "parent")
  end

  test("the filtered hook DSL resolves to a no-op static method") do
    assert_null(FilteredController.before_action(:show, fn(req) { req }))
  end
end

describe("response helpers") do
  test("redirect builds a 302 with a Location header") do
    assert_eq(redirect("/posts"), {"status": 302, "headers": {"Location": "/posts"}, "body": ""})
  end

  test("redirect refuses an external URL") do
    assert_raises("redirect() only accepts local absolute paths like '/dashboard'") do
      redirect("https://evil.example")
    end
  end

  test("redirect refuses a protocol-relative URL") do
    assert_raises("redirect() only accepts local absolute paths") do
      redirect("//evil.example")
    end
  end

  test("redirect_external allows an explicit external destination") do
    assert_eq(
      redirect_external("https://github.com/login"),
      {"status": 302, "headers": {"Location": "https://github.com/login"}, "body": ""}
    )
  end

  test("render_json and render_text hand the response to the fast path and return nil") do
    assert_null(render_json({"a": 1}))
    assert_null(render_text("pong"))
  end

  test("render_json needs data") do
    assert_raises("render_json() requires at least one argument") do
      render_json()
    end
  end

  context("halt") do
    test("raises, so a bare call stops the action, and catch sees the message") do
      # Sinatra's halt: the request handler turns the raise into the response
      reached = false
      message = nil
      try
        halt(403, "Forbidden")
        reached = true
      catch error
        message = error
      end
      assert_not(reached)
      assert_eq(message, "Forbidden")
    end

    test("raises the message as the error") do
      message = assert_raises() do
        halt(404, "Not found")
      end
      assert_eq(message, "Not found")
    end

    test("refuses a status outside 100..599") do
      assert_raises("halt() status must be 100..599, got 700") do
        halt(700, "x")
      end
    end
  end
end

describe("permit (strong parameters)") do
  context("scalar slots") do
    test("keeps listed scalars and drops unlisted keys") do
      params = {"title": "Hi", "body": "Text", "is_admin": true}
      assert_eq(permit(params, {"title": true, "body": true}), {"title": "Hi", "body": "Text"})
    end

    test("keeps every scalar type: string, int, float, bool and nil") do
      params = {"name": "a", "count": 3, "ratio": 0.5, "published": false, "note": nil}
      shape = {"name": true, "count": true, "ratio": true, "published": true, "note": true}
      assert_eq(permit(params, shape), params)
    end

    test("drops a hash or an array posted into a scalar slot") do
      params = {"title": {"sneaky": "x"}, "body": ["a", "b"], "slug": "ok"}
      assert_eq(permit(params, {"title": true, "body": true, "slug": true}), {"slug": "ok"})
    end

    test("leaves a missing key absent rather than nil") do
      permitted = permit({"title": "Hi"}, {"title": true, "body": true})
      assert_eq(permitted, {"title": "Hi"})
      assert_not(permitted.has_key("body"))
    end

    test("a false spec permits nothing") do
      assert_eq(permit({"title": "Hi"}, {"title": false}), {})
    end
  end

  context("nested hashes") do
    test("filters a nested hash by its own shape") do
      params = {"author": {"name": "Ada", "role": "admin"}}
      assert_eq(permit(params, {"author": {"name": true}}), {"author": {"name": "Ada"}})
    end

    test("recurses through several levels") do
      params = {"post": {"meta": {"seo": {"title": "T", "evil": 1}, "x": 2}}}
      shape = {"post": {"meta": {"seo": {"title": true}}}}
      assert_eq(permit(params, shape), {"post": {"meta": {"seo": {"title": "T"}}}})
    end

    test("drops a scalar posted where a hash is expected") do
      assert_eq(permit({"author": "Ada"}, {"author": {"name": true}}), {})
    end
  end

  context("arrays") do
    test("[] keeps an array's scalar elements and drops containers in it") do
      params = {"tags": ["a", {"evil": "x"}, "b", ["nested"], 3]}
      assert_eq(permit(params, {"tags": []}), {"tags": ["a", "b", 3]})
    end

    test("[] drops a scalar posted where an array is expected") do
      assert_eq(permit({"tags": "a"}, {"tags": []}), {})
    end

    test("[{...}] filters each hash of an array") do
      params = {"items": [{"sku": "A1", "qty": 2, "price": 0}, {"sku": "B2", "admin": true}]}
      permitted = permit(params, {"items": [{"sku": true, "qty": true}]})
      assert_eq(permitted, {"items": [{"sku": "A1", "qty": 2}, {"sku": "B2"}]})
    end

    test("[{...}] drops elements that are not hashes") do
      params = {"items": [{"sku": "A1"}, "loose", 5]}
      assert_eq(permit(params, {"items": [{"sku": true}]}), {"items": [{"sku": "A1"}]})
    end

    test("[{...}] turns a numeric-keyed hash (items[0][sku] form fields) into an array") do
      params = {"items": {"0": {"sku": "a", "evil": 1}, "1": {"sku": "b"}}}
      assert_eq(permit(params, {"items": [{"sku": true}]}), {"items": [{"sku": "a"}, {"sku": "b"}]})
    end

    test("[{...}] drops a hash whose keys are not all numeric") do
      params = {"items": {"0": {"sku": "a"}, "x": {"sku": "b"}}}
      assert_eq(permit(params, {"items": [{"sku": true}]}), {})
    end
  end

  context("arguments") do
    test("nil params (a missing sub-hash) filter to an empty hash") do
      assert_eq(permit(nil, {"title": true}), {})
    end

    test("the result follows the shape's key order") do
      assert_eq(permit({"a": 1, "b": 2}, {"b": true, "a": true}).keys, ["b", "a"])
    end

    test("leaves the params hash untouched") do
      params = {"title": "Hi", "is_admin": true}
      permit(params, {"title": true})
      assert_eq(params, {"title": "Hi", "is_admin": true})
    end

    test("refuses params that are not a hash") do
      assert_raises("permit() expects a params hash, got string") do
        permit("title=Hi", {"title": true})
      end
    end

    test("refuses a shape that is not a hash") do
      assert_raises("permit() expects a shape hash, got string") do
        permit({"title": "Hi"}, "title")
      end
    end
  end
end

describe("respond_to (DSL form)") do
  test("an html-only handler matches Accept: text/html") do
    response = respond_to(request_with({"accept": "text/html"}), fn(format) {
      format.html { respond("html") }
    })
    assert_eq(response, {"status": 200, "headers": {}, "body": "html"})
  end

  test("a trailing do block works like the fn form") do
    pending("bug: respond_to(req) do |format| ... end raises Undefined variable 'respond_to'")
    response = respond_to(request_with({"accept": "application/json"})) do |format|
      format.json { respond("json") }
    end
    assert_eq(response["body"], "json")
  end

  # EUI 01 §2.4: a page and its already-resolved form are two representations
  # of one resource, so they share a route and a URL.
  test("eui wins when the caller asks for frames") do
    response = respond_to(request_with({"accept": "application/vnd.eui.frames"}, "/docs/intro"), fn(format) {
      format.html { respond("html") }
      format.eui { respond("frames") }
    })
    assert_eq(response["body"], "frames")
  end

  test("a browser's Accept, */* and all, still gets the page") do
    accept = "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"
    response = respond_to(request_with({"accept": accept}, "/docs/intro"), fn(format) {
      format.html { respond("html") }
      format.eui { respond("frames") }
    })
    assert_eq(response["body"], "html")
  end

  test("a .eui address names the frames representation") do
    # What a cache holds, since no CDN wants to key on Accept
    response = respond_to(request_with({}, "/docs/intro.eui"), fn(format) {
      format.html { respond("html") }
      format.eui { respond("frames") }
    })
    assert_eq(response["body"], "frames")
  end

  test("json wins over html when Accept asks for json") do
    response = respond_to(request_with({"accept": "application/json"}), fn(format) {
      format.html { respond("html") }
      format.json { respond("json") }
    })
    assert_eq(response["body"], "json")
  end

  test("q-values: json q=0.9 beats html q=0.5") do
    response = respond_to(request_with({"accept": "text/html;q=0.5,application/json;q=0.9"}), fn(format) {
      format.html { respond("html") }
      format.json { respond("json") }
    })
    assert_eq(response["body"], "json")
  end

  test("a .json URL extension beats Accept: text/html") do
    response = respond_to(request_with({"accept": "text/html"}, "/posts/1.json"), fn(format) {
      format.html { respond("html") }
      format.json { respond("json") }
    })
    assert_eq(response["body"], "json")
  end

  test("a .txt URL extension picks the text branch") do
    response = respond_to(request_with({}, "/posts/1.txt"), fn(format) {
      format.html { respond("html") }
      format.text { respond("text") }
    })
    assert_eq(response["body"], "text")
  end

  test("?format=xml beats Accept: text/html") do
    response = respond_to(request_with({"accept": "text/html"}, "/posts/1", {"format": "xml"}), fn(format) {
      format.html { respond("html") }
      format.xml { respond("xml") }
    })
    assert_eq(response["body"], "xml")
  end

  test("HX-Request: true picks the htmx branch over html") do
    response = respond_to(request_with({"hx-request": "true", "accept": "text/html"}), fn(format) {
      format.html { respond("full") }
      format.htmx { respond("partial") }
    })
    assert_eq(response["body"], "partial")
  end

  test("X-Requested-With: XMLHttpRequest picks the xhr branch") do
    response = respond_to(request_with({"x-requested-with": "XMLHttpRequest", "accept": "text/html"}), fn(format) {
      format.html { respond("full") }
      format.xhr { respond("xhr") }
    })
    assert_eq(response["body"], "xhr")
  end

  test("Accept: */* falls through to the first registered handler") do
    response = respond_to(request_with({"accept": "*/*"}), fn(format) {
      format.html { respond("html-first") }
      format.json { respond("json") }
    })
    assert_eq(response["body"], "html-first")
  end

  test("an unmatched Accept returns 406 Not Acceptable") do
    response = respond_to(request_with({"accept": "application/pdf"}), fn(format) {
      format.html { respond("html") }
    })
    assert_eq(response, {
      "status": 406,
      "headers": {"Content-Type": "text/plain; charset=utf-8"},
      "body": "Not Acceptable"
    })
  end

  test("any is the catch-all when no other format matches") do
    response = respond_to(request_with({"accept": "application/pdf"}), fn(format) {
      format.html { respond("html") }
      format.any { respond("fallback") }
    })
    assert_eq(response["body"], "fallback")
  end

  test("the last registration of a format wins") do
    response = respond_to(request_with({"accept": "application/json"}), fn(format) {
      format.json { respond("first") }
      format.json { respond("second") }
    })
    assert_eq(response["body"], "second")
  end

  test("a nested respond_to does not clobber the outer one") do
    response = respond_to(request_with({"accept": "text/html"}, "/p"), fn(format) {
      format.html { respond("outer:" + inner_json_body()) }
    })
    assert_eq(response["body"], "outer:inner-json")
  end

  test("excel matches an .xlsx URL extension") do
    response = respond_to(request_with({"accept": "text/html"}, "/reports/q1.xlsx"), fn(format) {
      format.html { respond("html") }
      format.excel { respond("xlsx") }
    })
    assert_eq(response["body"], "xlsx")
  end

  test("csv matches Accept: text/csv") do
    response = respond_to(request_with({"accept": "text/csv"}, "/exports"), fn(format) {
      format.html { respond("html") }
      format.csv { respond("csv") }
    })
    assert_eq(response["body"], "csv")
  end

  test("pdf matches Accept: application/pdf") do
    response = respond_to(request_with({"accept": "application/pdf"}, "/invoices/1"), fn(format) {
      format.html { respond("html") }
      format.pdf { respond("pdf") }
    })
    assert_eq(response["body"], "pdf")
  end
end

describe("respond_to (hash form)") do
  test("dispatches like the DSL") do
    handlers = {"html": fn() { respond("html") }, "json": fn() { respond("json") }}
    assert_eq(respond_to(request_with({"accept": "application/json"}), handlers)["body"], "json")
  end

  test("returns 406 when nothing matches") do
    handlers = {"html": fn() { respond("html") }}
    assert_eq(respond_to(request_with({"accept": "application/pdf"}), handlers)["status"], 406)
  end

  test("falls back to the first inserted handler on */*") do
    handlers = {"html": fn() { respond("html-first") }, "json": fn() { respond("json") }}
    assert_eq(respond_to(request_with({"accept": "*/*"}), handlers)["body"], "html-first")
  end
end
